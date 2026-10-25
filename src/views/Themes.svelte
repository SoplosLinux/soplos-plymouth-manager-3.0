<script lang="ts">
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { _ } from "svelte-i18n";
  import { open } from "@tauri-apps/plugin-dialog";
  import { Palette } from "lucide-svelte";
  import { api } from "../lib/api";
  import ThemeCard from "../components/ThemeCard.svelte";
  import { askConfirm, dialog, showError, showInfo } from "../lib/dialogs";
  import { busy, describeOutput, hasStep, runJob } from "../lib/jobs";
  import type { ThemeInfo } from "../lib/themes";

  // The gallery of installed themes, with the actions of the 2.x program:
  // apply, preview, install, remove and refresh. The buttons are in the hero
  // of the window, and call the functions exported here. One click selects a
  // theme; Ctrl or Shift click, or the box of a card, select several, which can
  // be removed together. Every action runs as a job (see lib/jobs.ts) and the
  // list is read again when it ends.

  // For the buttons of the hero: whether exactly one theme is selected (apply
  // and preview need that), how many are, and whether the list is being read.
  export let canSingle = false;
  export let chosenCount = 0;
  export let loading = true;

  const t = (key: string, values?: Record<string, string | number>) => get(_)(key, { values });

  let themes: ThemeInfo[] = [];
  let current: string | null = null;
  let previews: Record<string, string | null> = {};
  let selected = new Set<string>();
  // The last theme clicked: where a Shift click starts its range.
  let anchor = "";

  $: chosen = themes.filter((theme) => selected.has(theme.name));
  // Apply and preview work on one theme; remove works on any number.
  $: single = chosen.length === 1 ? chosen[0] : null;
  $: canSingle = single !== null;
  $: chosenCount = chosen.length;

  // Reads the pictures again every time: a picture can be gone, or changed, since
  // the last read. The ones that are no longer there are dropped at once.
  async function loadPreviews() {
    const next: Record<string, string | null> = {};
    for (const theme of themes) {
      if (theme.hasPreview && theme.name in previews) next[theme.name] = previews[theme.name];
    }
    previews = next;
    for (const theme of themes) {
      if (!theme.hasPreview) continue;
      const picture = await api.invoke<string | null>("read_theme_preview", { name: theme.name }).catch(() => null);
      previews[theme.name] = picture;
      previews = previews;
    }
  }

  export async function refresh() {
    loading = true;
    try {
      [themes, current] = await Promise.all([api.invoke<ThemeInfo[]>("list_themes"), api.invoke<string | null>("get_current_theme")]);
    } catch (error) {
      console.error("Reading the themes failed", error);
      themes = [];
      current = null;
    }
    // What was selected stays selected if it is still there; otherwise the
    // theme in use is, as the 2.x program left it.
    const still = new Set([...selected].filter((name) => themes.some((theme) => theme.name === name)));
    if (still.size > 0) selected = still;
    else selected = current && themes.some((theme) => theme.name === current) ? new Set([current]) : new Set();
    anchor = [...selected][0] ?? "";
    loading = false;
    void loadPreviews();
  }

  onMount(refresh);

  function selectOnly(name: string) {
    selected = new Set([name]);
    anchor = name;
  }

  function toggle(name: string) {
    const next = new Set(selected);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    selected = next;
    anchor = name;
  }

  function selectRange(name: string) {
    const names = themes.map((theme) => theme.name);
    const from = names.indexOf(anchor);
    const to = names.indexOf(name);
    if (from < 0 || to < 0) return selectOnly(name);
    selected = new Set(names.slice(Math.min(from, to), Math.max(from, to) + 1));
  }

  function onCardClick(event: MouseEvent, name: string) {
    if (event.shiftKey) selectRange(name);
    else if (event.ctrlKey || event.metaKey) toggle(name);
    else selectOnly(name);
  }

  const selectAll = () => (selected = new Set(themes.map((theme) => theme.name)));
  const clearSelection = () => (selected = new Set());

  // There is no log in this window, so a job that fails says why in a dialog,
  // with what the tools printed under the text.
  async function reportFailure(bodyKey: string, output: string) {
    await showError(t("errors.title"), t(bodyKey), describeOutput(output));
  }

  export async function apply() {
    if (!single || $busy) return;
    const result = await runJob("apply_theme", { name: single.name });
    await refresh();
    if (!result.success) return reportFailure("errors.apply", result.output);
    if (hasStep(result.output, "noInitramfs")) await showInfo(t("notices.title"), t("steps.noInitramfs"));
  }

  export async function preview() {
    if (!single || $busy) return;
    if (!single.isGraphical) {
      await showInfo(t("themes.textOnlyTitle"), t("themes.textOnlyBody"));
      return;
    }
    const name = single.name;
    const result = await runJob("preview_theme", { name });
    // The capture is a new image: read it again.
    delete previews[name];
    previews = previews;
    await refresh();
    if (!result.success) return reportFailure("errors.preview", result.output);
  }

  export async function install() {
    if ($busy) return;
    const picked = await open({
      title: t("themes.selectBundle"),
      multiple: false,
      directory: false,
      filters: [{ name: t("themes.bundlesFilter"), extensions: ["zip", "tar.gz", "tgz", "tar.xz", "tar.bz2"] }],
    });
    if (typeof picked !== "string") return;
    const result = await runJob("install_theme", { archive: picked });
    await refresh();
    if (!result.success) return reportFailure("errors.install", result.output);
  }

  // Removes every selected theme but the one in use, which cannot go: the
  // others are removed after one question.
  export async function remove() {
    if (chosen.length === 0 || $busy) return;
    const removable = chosen.filter((theme) => theme.name !== current);
    const skipsActive = removable.length < chosen.length;
    if (removable.length === 0) {
      await showInfo(t("themes.removeTitle"), t("themes.removeActive"));
      return;
    }
    let body = removable.length === 1 ? t("themes.removeBody", { name: removable[0].name }) : t("themes.removeManyBody", { count: removable.length });
    if (skipsActive) body += `\n\n${t("themes.removeSkipped")}`;
    if (!(await askConfirm(t("themes.removeTitle"), body, t("common.remove"), true))) return;
    const result = await runJob("remove_themes", { names: removable.map((theme) => theme.name) });
    selected = new Set();
    await refresh();
    if (!result.success) return reportFailure("errors.remove", result.output);
  }

  function handleKeydown(e: KeyboardEvent) {
    if ($dialog) return;
    const target = e.target as HTMLElement | null;
    if (target && ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName)) return;
    const key = e.key.toLowerCase();
    if (e.key === "F5" || (e.ctrlKey && key === "r")) {
      e.preventDefault();
      void refresh();
    } else if (e.ctrlKey && key === "a") {
      e.preventDefault();
      apply();
    } else if (e.ctrlKey && key === "p") {
      e.preventDefault();
      void preview();
    } else if (e.ctrlKey && key === "i") {
      e.preventDefault();
      void install();
    } else if (e.key === "Delete") {
      e.preventDefault();
      void remove();
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

{#if !loading && themes.length === 0}
  <div class="h-full flex flex-col items-center justify-center gap-2 text-center">
    <Palette size={64} class="text-text-muted" />
    <p class="text-[0.9rem] text-text-bright">{$_("themes.emptyTitle")}</p>
    <p class="text-[0.8rem] text-text-muted">{$_("themes.emptyBody")}</p>
  </div>
{:else}
  <div class="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(150px,1fr))]">
    {#each themes as theme (theme.name)}
      <ThemeCard
        {theme}
        preview={previews[theme.name] ?? null}
        selected={selected.has(theme.name)}
        inUse={current === theme.name}
        onClick={(event) => onCardClick(event, theme.name)}
        onToggle={() => toggle(theme.name)}
      />
    {/each}
  </div>
  {#if themes.length > 0}
    <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-[0.7rem] text-text-muted">
      {#if chosen.length > 1}
        <span class="text-primary font-semibold">{$_("themes.selected", { values: { count: chosen.length } })}</span>
      {/if}
      <button class="hover:text-text-bright hover:underline" on:click={selectAll}>{$_("themes.selectAll")}</button>
      {#if chosen.length > 0}
        <button class="hover:text-text-bright hover:underline" on:click={clearSelection}>{$_("themes.clearSelection")}</button>
      {/if}
    </div>
  {/if}
{/if}
