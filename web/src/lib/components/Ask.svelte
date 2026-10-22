<script lang="ts">
  import type { StemmaDialog } from "@stemma-math/design-elements/dialog.js";
  import { asking } from "#lib/ask.svelte.ts";

  let dialog = $state<StemmaDialog>();
  let typed = $state("");
  const ready = $derived(!asking.question?.typed || typed.trim() === asking.question.typed.value);

  $effect(() => {
    if (!dialog || !asking.question || dialog.isOpen) return;
    typed = "";
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
        {#if asking.question.typed}
          <div class="field">
            <label class="field__label" for="ask-typed">{asking.question.typed.label}</label>
            <input id="ask-typed" class="input input--mono" autocomplete="off" autocapitalize="none" bind:value={typed} />
          </div>
        {/if}
        <div class="dialog__actions">
          <button type="submit" class="button" value="">{asking.question.dismiss ?? "Cancel"}</button>
          <button type="submit" class="button button--primary" class:button--danger={asking.question.danger} value="yes" disabled={!ready}>
            {asking.question.action}
          </button>
        </div>
      </form>
    {/if}
  </dialog>
</stemma-dialog>
