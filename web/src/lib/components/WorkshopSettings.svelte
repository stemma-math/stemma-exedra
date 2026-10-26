<script lang="ts">
  import { api } from "#lib/api.ts";
  import { exactly } from "#lib/format.ts";
  import { toastError } from "#lib/toast.ts";
  import type { WorkshopModel } from "#lib/workshop.ts";
  import { scriptedSelect } from "#lib/scripted-select.ts";

  let { group }: { group: string } = $props();
  let settings = $state<WorkshopModel | null>(null);
  let provider = $state<WorkshopModel["provider"]>("anthropic");
  let model = $state("");
  let key = $state("");
  let cap = $state(1_000_000);
  let steps = $state(20);
  let saving = $state(false);
  const suggestions = $derived(
    provider === "anthropic"
      ? ["claude-sonnet-4-6", "claude-opus-4-6"]
      : provider === "openai"
        ? ["gpt-5.4", "gpt-5.4-mini"]
        : ["anthropic/claude-sonnet-4.6", "openai/gpt-5.4"],
  );
  function show(value: WorkshopModel) {
    settings = value;
    provider = value.provider;
    model = value.model;
    cap = value.monthly_cap;
    steps = value.step_cap;
  }
  $effect(() => { api.workshopModel(group).then(show).catch(toastError); });
  async function save(event?: SubmitEvent, remove = false) {
    event?.preventDefault();
    saving = true;
    try {
      show(await api.updateWorkshopModel(group, {
        provider, model, monthly_cap: cap, step_cap: steps,
        api_key: remove ? "" : key || null,
      }));
      key = "";
    } catch (error) { toastError(error); }
    finally { saving = false; }
  }
</script>

<section class="stack" aria-label="Workshop model">
  <h2>Workshop</h2>
  <p>
    The group's orchestrators use an API key, separate from anyone's subscription sign-in.
    Use a key made for this workshop and set a spending limit at the provider.
    Any group admin may replace or remove it. Whoever runs the hub could read it.
  </p>
  {#if settings}
    <dl class="request-facts">
      <dt class="label">API key</dt>
      <dd>
        {#if settings.last_four}
          Ending in <code>{settings.last_four}</code>, set by {settings.set_by?.display_name ?? "a deleted account"}
          {#if settings.set_at !== null}on {exactly(settings.set_at)}{/if}.
        {:else}
          No key set. People can still choose recipes themselves.
        {/if}
      </dd>
      <dt class="label">This month ({settings.month}, UTC)</dt>
      <dd>
        {(settings.input_tokens + settings.output_tokens).toLocaleString("en")} of {settings.monthly_cap.toLocaleString("en")} tokens
        {#if settings.cost_usd !== null}, ${settings.cost_usd.toFixed(4)} reported by the provider{/if}.
      </dd>
    </dl>
    {#if settings.stopped}
      <p class="message message--warning" role="status">The monthly token cap has stopped thinking until next month or a higher cap.</p>
    {/if}
    <form class="form" onsubmit={save}>
      <div class="field">
        <label class="field__label" for="workshop-provider">Provider</label>
        <stemma-select>
          <select id="workshop-provider" class="input" bind:value={provider} use:scriptedSelect={[provider]}>
            <option value="anthropic">Anthropic</option>
            <option value="openai">OpenAI</option>
            <option value="openrouter">OpenRouter</option>
          </select>
        </stemma-select>
      </div>
      <div class="field">
        <label class="field__label" for="workshop-model">Model</label>
        <input id="workshop-model" class="input" bind:value={model} list="workshop-models" maxlength="200" required />
        <datalist id="workshop-models">
          {#each suggestions as suggestion (suggestion)}
            <option value={suggestion}></option>
          {/each}
        </datalist>
        <p class="field__hint">The person providing the key chooses its model. You may enter another model identifier.</p>
      </div>
      <div class="field">
        <label class="field__label" for="workshop-key">API key</label>
        <input id="workshop-key" class="input" type="password" bind:value={key} autocomplete="new-password" placeholder="Leave empty to keep the current key" />
        <p class="field__hint">Written once, never read back. The hub encrypts it in its database.</p>
      </div>
      <div class="field">
        <label class="field__label" for="workshop-cap">Monthly token cap (input plus output)</label>
        <input id="workshop-cap" class="input" type="number" min="0" step="1" bind:value={cap} required />
      </div>
      <div class="field">
        <label class="field__label" for="workshop-steps">Steps per orchestrator turn</label>
        <input id="workshop-steps" class="input" type="number" min="1" max="100" step="1" bind:value={steps} required />
      </div>
      <div class="row">
        <button class="button button--primary" type="submit" disabled={saving}>Save</button>
        {#if settings.last_four}
          <button class="button button--danger" type="button" disabled={saving} onclick={() => save(undefined, true)}>Remove key</button>
        {/if}
      </div>
    </form>
  {/if}
</section>
