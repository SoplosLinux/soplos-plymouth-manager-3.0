<script lang="ts">
  import { _ } from "svelte-i18n";
  import { ImageOff, Check, Type } from "lucide-svelte";
  import type { ThemeInfo } from "../lib/themes";

  // One theme of the gallery, as in the 2.x program: a bordered card with the
  // image of the theme and its name under it. A click selects it alone, shown
  // with the ring the lists of the other Soplos programs use; the box in the
  // corner, or a click with Ctrl or Shift, selects several to remove them
  // together.
  export let theme: ThemeInfo;
  export let preview: string | null = null;
  export let selected = false;
  export let inUse = false;
  export let onClick: (event: MouseEvent) => void;
  export let onToggle: () => void;
</script>

<div
  class="group relative rounded border border-border bg-surface p-2.5 pb-2 cursor-pointer select-none transition-colors hover:bg-surface-2
    {selected ? 'ring-1 ring-primary' : ''}"
  role="button"
  tabindex="0"
  aria-pressed={selected}
  title={theme.description}
  on:click={onClick}
  on:keydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onToggle())}
>
  <button
    class="absolute left-3.5 top-3.5 z-10 w-[18px] h-[18px] rounded border grid place-items-center transition-opacity
      {selected ? 'bg-primary border-primary opacity-100' : 'bg-black/60 border-white/50 opacity-0 group-hover:opacity-100 focus-visible:opacity-100'}"
    role="checkbox"
    aria-checked={selected}
    aria-label={$_("themes.cardSelect", { values: { name: theme.name } })}
    on:click|stopPropagation={onToggle}
  >
    {#if selected}<Check size={12} class="text-white" strokeWidth={3} />{/if}
  </button>

  <div class="aspect-[16/10] rounded-sm bg-black grid place-items-center overflow-hidden">
    {#if preview}
      <img src={preview} alt={theme.name} class="w-full h-full object-cover" />
    {:else if !theme.isGraphical}
      <Type size={26} class="text-text-muted" />
    {:else}
      <ImageOff size={26} class="text-text-muted" />
    {/if}
  </div>

  <div class="mt-2 flex flex-col items-center gap-1">
    <span class="max-w-full truncate text-[0.82rem] font-semibold text-text-bright">{theme.name}</span>
    {#if inUse}
      <span class="flex items-center gap-1 rounded-full bg-success/15 px-2 py-0.5 text-[0.62rem] font-semibold text-success"><Check size={10} />{$_("themes.inUse")}</span>
    {:else if !theme.isGraphical}
      <span class="rounded-full bg-surface-2 px-2 py-0.5 text-[0.62rem] font-semibold text-text-muted">{$_("themes.textOnly")}</span>
    {/if}
  </div>
</div>
