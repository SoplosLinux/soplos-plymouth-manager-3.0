<script lang="ts">
  import Modal from "./Modal.svelte";
  import { dialog } from "../lib/dialogs";

  // Draws the dialog that lib/dialogs.ts is asking for.
  $: current = $dialog;
</script>

{#if current}
  <Modal
    variant={current.kind === "danger" ? "danger" : current.kind === "confirm" ? "confirm" : "info"}
    title={current.title}
    confirmLabel={current.confirmLabel}
    maxWidth={current.details ? 560 : 440}
    onClose={() => current?.resolve(false)}
    onConfirm={() => current?.resolve(true)}
  >
    <p class="whitespace-pre-line">{current.body}</p>
    {#if current.details}
      <pre class="mt-3 max-h-[200px] overflow-auto content-scroll rounded border border-border bg-panel p-2 text-[0.72rem] text-text whitespace-pre-wrap break-words">{current.details}</pre>
    {/if}
  </Modal>
{/if}
