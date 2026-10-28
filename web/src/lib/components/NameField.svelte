<script lang="ts">
  import { api } from "#lib/api.ts";
  import Glyph from "./Glyph.svelte";
  let { id, label, kind, group, current, value = $bindable(""), available = $bindable(false) }: {
    id: string; label: string; kind: "user" | "group"; group?: string;
    /** The name as it is now, which needs no word about it. */
    current?: string;
    value: string; available?: boolean;
  } = $props();
  let reason = $state<string | null>(null);
  let checking = $state(false);
  $effect(() => {
    const name = value;
    const scope = group;
    let active = true;
    available = false;
    checking = true;
    const timer = setTimeout(() => {
      api.nameAvailability(kind, name, scope).then((result) => {
        if (!active) return;
        available = result.available;
        reason = result.reason;
      }).catch(() => { if (active) reason = "Could not check this name. Try again."; })
        .finally(() => { if (active) checking = false; });
    }, 200);
    return () => { active = false; clearTimeout(timer); };
  });
  const unchanged = $derived(value === current);
</script>

<div class="field">
  <label class="field__label" for={id}>{label}</label>
  <input {id} class="input" bind:value required maxlength={kind === "user" ? 32 : 40} aria-describedby="{id}-availability" aria-invalid={!checking && !available} autocapitalize="none" spellcheck="false" />
  <span id="{id}-availability" aria-live="polite" class="field__hint" class:field__success={!checking && available && !unchanged} class:field__error={!checking && !available}>
    {#if checking}
      {unchanged ? "" : "Checking…"}
    {:else if !available}
      <Glyph name="alert" small />{reason ?? "This name is taken."}
    {:else if !unchanged}
      <Glyph name="check" small />This name is available.
    {/if}
  </span>
</div>
