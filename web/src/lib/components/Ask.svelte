<script lang="ts">
  import type { StemmaDialog } from "@stemma-math/design-elements/dialog.js";
  import { asking } from "#lib/ask.svelte.ts";

  let dialog = $state<StemmaDialog>();

  $effect(() => {
    if (!dialog || !asking.question || dialog.isOpen) return;
    void dialog.open().then((value) => {
      asking.answer?.(value === "yes");
      asking.question = null;
      asking.answer = null;
    });
  });
</script>

<stemma-dialog bind:this={dialog}>
  <dialog class="dialog" aria-labelledby="ask-title">
    {#if asking.question}
      <form class="dialog__body" method="dialog">
        <h2 class="dialog__title" id="ask-title">{asking.question.title}</h2>
        {#if asking.question.text}<p class="dialog__text">{asking.question.text}</p>{/if}
        <div class="dialog__actions">
          <button type="submit" class="button" value="">{asking.question.dismiss ?? "Cancel"}</button>
          <button type="submit" class="button button--primary" class:button--danger={asking.question.danger} value="yes">
            {asking.question.action}
          </button>
        </div>
      </form>
    {/if}
  </dialog>
</stemma-dialog>
