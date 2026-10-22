<script lang="ts">
  import { api } from "#lib/api.ts";
  let { id, label, kind, group, value = $bindable(""), available = $bindable(false) }: {
    id: string; label: string; kind: "user" | "group"; group?: string;
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
</script>

<div class="field">
  <label class="field__label" for={id}>{label}</label>
  <input {id} class="input" bind:value required maxlength={kind === "user" ? 32 : 40} aria-describedby="{id}-availability" aria-invalid={!checking && !available} autocapitalize="none" spellcheck="false" />
  <p id="{id}-availability" class="field__hint" aria-live="polite">{checking ? "Checking availability…" : reason ?? "This name is available to you."}</p>
</div>
