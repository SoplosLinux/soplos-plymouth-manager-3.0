<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getVersion } from "@tauri-apps/api/app";
  import { _, locale } from "svelte-i18n";
  import WindowControls from "./components/WindowControls.svelte";
  import { frameless, maximized } from "./lib/window";
  import { Check, Play, Download, Trash2, RefreshCw, Sun, Moon } from "lucide-svelte";
  import { api } from "./lib/api";
  import { busy } from "./lib/jobs";
  import { theme, toggleTheme } from "./lib/theme";
  import { SUPPORTED_LOCALES, LOCALE_NAMES, setLocale, type SupportedLocale } from "./i18n";
  import type { EnvironmentInfo } from "./lib/themes";
  import AboutDialog from "./components/AboutDialog.svelte";
  import DialogHost from "./components/DialogHost.svelte";
  import Themes from "./views/Themes.svelte";

  // The frame of the other Soplos 3.0 programs without a sidebar (WebApp
  // Manager, AppImage Manager): a hero with the title and the language, and a
  // thin footer with the theme button, the session and the version. What is
  // inside is the 2.x program's: the gallery of themes and, under it, the
  // buttons to apply, preview, install, remove and refresh.

  let appVersion = "";
  let environment: EnvironmentInfo = { desktopEnvironment: "unknown", displayProtocol: "unknown", initramfs: "none", initramfsAvailable: [] };
  let aboutOpen = false;
  let gallery: Themes;
  let canSingle = false;
  let chosenCount = 0;
  let loading = true;

  onMount(async () => {
    try {
      appVersion = await getVersion();
    } catch {
      appVersion = "";
    }
    try {
      environment = await api.invoke<EnvironmentInfo>("get_environment_info");
    } catch (error) {
      console.error("Reading the environment failed", error);
    }
  });

  // "3.0.0-1-beta" → "3.0.0-beta": the footer shows the app version, not the
  // Debian package revision.
  $: cleanVersion = appVersion.replace(/-\d+-/, "-");

  function onLanguageChange(e: Event) {
    setLocale((e.currentTarget as HTMLSelectElement).value as SupportedLocale);
  }

  // F1 -> About, Ctrl+Q -> quit. The keys of the themes (Ctrl+A, Ctrl+P,
  // Ctrl+I, Ctrl+R, F5 and Delete) are handled by the gallery.
  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "F1") {
      e.preventDefault();
      aboutOpen = true;
    } else if (e.ctrlKey && !e.shiftKey && (e.key === "q" || e.key === "Q")) {
      e.preventDefault();
      getCurrentWindow().close();
    }
  }

  const primaryButton = "flex items-center gap-2 px-5 py-2.5 rounded bg-primary hover:bg-primary-hover text-black text-[0.84rem] font-semibold disabled:opacity-50 disabled:pointer-events-none";
  const secondaryButton =
    "flex items-center gap-2 px-5 py-2.5 rounded border border-border bg-surface-2 text-text-bright hover:border-primary text-[0.84rem] font-semibold disabled:opacity-50 disabled:pointer-events-none";
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="h-screen bg-dark {$frameless && !$maximized ? 'window-card' : ''}">
<div class="relative flex flex-col h-full overflow-hidden">
  {#if $frameless}
    <div class="relative h-7 shrink-0" data-tauri-drag-region>
      <div class="absolute right-2 top-[3px]"><WindowControls /></div>
    </div>
  {/if}

  <div class="p-4 pb-0 shrink-0 {$frameless ? 'pt-0' : ''}" data-tauri-drag-region="deep">
    <div
      class="relative rounded border border-border overflow-hidden flex flex-col items-start justify-end p-4 min-h-[110px]"
      style="background: linear-gradient(rgba(20, 20, 22, 0.25), rgba(20, 20, 22, 0.82)), url('/assets/hero/plymouth-manager-background.jpg'); background-size: cover; background-position: center;"
    >
      <div class="absolute top-3 right-3">
        <select
          value={$locale}
          on:change={onLanguageChange}
          class="appearance-none bg-primary/20 backdrop-blur-sm border border-primary/30 text-primary rounded px-2 py-1 text-[0.7rem] focus:outline-none"
          title={$_("sidebar.language")}
        >
          {#each SUPPORTED_LOCALES as code}
            <option value={code}>{LOCALE_NAMES[code]}</option>
          {/each}
        </select>
      </div>

      <p class="text-white font-semibold text-[0.95rem]">{$_("hero.title")}</p>
      <p class="text-[0.75rem] text-white/85 mt-0.5">{$_("hero.description")}</p>
    </div>
  </div>

  <div class="flex-1 overflow-y-auto content-scroll p-4">
    <Themes bind:this={gallery} bind:canSingle bind:chosenCount bind:loading />
  </div>

  <!-- The buttons of the 2.x program, under the gallery. -->
  <div class="flex shrink-0 flex-wrap items-center justify-center gap-3 px-4 pb-3 pt-1">
    <button class={primaryButton} disabled={!canSingle || $busy} on:click={() => gallery.apply()}>
      <Check size={15} />{$_("themes.apply")}
    </button>
    <button class={secondaryButton} disabled={!canSingle || $busy} on:click={() => gallery.preview()}>
      <Play size={15} />{$_("themes.preview")}
    </button>
    <button class={secondaryButton} disabled={$busy} on:click={() => gallery.install()}>
      <Download size={15} />{$_("themes.install")}
    </button>
    <button class={secondaryButton} disabled={chosenCount === 0 || $busy} on:click={() => gallery.remove()}>
      <Trash2 size={15} />{chosenCount > 1 ? $_("themes.removeSelected", { values: { count: chosenCount } }) : $_("themes.remove")}
    </button>
    <button class={secondaryButton} disabled={loading || $busy} on:click={() => gallery.refresh()}>
      <RefreshCw size={15} />{$_("themes.refresh")}
    </button>
  </div>

  <footer class="flex items-center justify-between px-4 py-1.5 border-t border-border text-[0.7rem] text-text-muted shrink-0">
    <div class="flex items-center gap-1.5">
      <button
        class="p-1 -m-1 rounded text-text-muted hover:bg-surface hover:text-text-bright"
        on:click={toggleTheme}
        title={$theme === "dark" ? $_("sidebar.switchToLight") : $_("sidebar.switchToDark")}
      >
        <svelte:component this={$theme === "dark" ? Moon : Sun} size={13} />
      </button>
      <span>
        {$_(`environment.${environment.desktopEnvironment}`)} · {$_(`environment.${environment.displayProtocol}`)}
      </span>
    </div>
    <button
      class="text-text-muted hover:text-text-bright hover:underline"
      on:click={() => (aboutOpen = true)}
      title={$_("sidebar.aboutTooltip")}
    >
      {cleanVersion}
    </button>
  </footer>

  <!-- The progress bar, as in the Welcome, the Store and the GRUB Editor: the
       last thing of the column, along the bottom edge of the window. Its track
       is only drawn while a job runs. -->
  <div class="h-1.5 bg-surface-2 overflow-hidden shrink-0" class:invisible={!$busy}>
    {#if $busy}
      <div class="h-full w-1/3 bg-primary progress-slide"></div>
    {/if}
  </div>
</div>

{#if aboutOpen}
  <AboutDialog onClose={() => (aboutOpen = false)} />
{/if}

<DialogHost />
</div>
