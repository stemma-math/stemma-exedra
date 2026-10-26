<script lang="ts">
  import { api } from "#lib/api.ts";
  import { exactly } from "#lib/format.ts";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { toastError } from "#lib/toast.ts";
  import type { WorkshopModel } from "#lib/workshop.ts";

  let { group }: { group: string } = $props();
  let settings = $state<WorkshopModel | null>(null);
  let provider = $state<WorkshopModel["provider"]>("anthropic");
  let model = $state("");
  let key = $state("");
  let cap = $state(1_000_000);
  let steps = $state(20);
  let saving = $state(false);

  /** A few models per provider; any other identifier works too. */
  const SUGGESTIONS: Record<WorkshopModel["provider"], string[]> = {
    anthropic: ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"],
    openai: ["gpt-6.1-sol", "gpt-5.6-sol"],
    openrouter: ["anthropic/claude-sonnet-5.5", "openai/gpt-6.1-sol"],
  };

  $effect(() => {
    api.workshopModel(group).then(show).catch(toastError);
  });

  function show(value: WorkshopModel) {
    settings = value;
    provider = value.provider;
    model = value.model;
    cap = value.monthly_cap;
    steps = value.step_cap;
  }

  /** Saves the form; the key only when one is typed, or removes it. */
  async function save(remove = false) {
    saving = true;
    try {
      const body = {
        provider,
        model,
        api_key: remove ? "" : key || null,
        monthly_cap: cap,
        step_cap: steps,
      };
      show(await api.updateWorkshopModel(group, body));
      key = "";
    } catch (error) {
      toastError(error);
    } finally {
      saving = false;
    }
  }
</script>

<section class="stack" aria-label="Workshop">
  <h2>Workshop</h2>
  <p>
    The workshop thinks with the group's model: it turns requests in plain words into processes, and reviews
    and merges their pull requests. It needs an API key, never a subscription's sign-in. Use a key made for
    this, with a spending limit at the provider: whoever runs the hub could read it.
  </p>
  {#if settings}
    <dl class="request-facts">
      <dt class="label">API key</dt>
      <dd>
        {#if settings.has_key}
          Ending in <code>{settings.last_four}</code>, set by {settings.set_by?.display_name ?? "a deleted account"}{#if settings.set_at !== null},
            {exactly(settings.set_at)}{/if}
        {:else}
          None yet. Members can still choose recipes themselves.
        {/if}
      </dd>
      <dt class="label">This month</dt>
      <dd>
        {(settings.input_tokens + settings.output_tokens).toLocaleString("en")} of
        {settings.monthly_cap.toLocaleString("en")} tokens{#if settings.cost_usd !== null}, ${settings.cost_usd.toFixed(2)}{/if}
        <span class="quiet">({settings.month}, UTC)</span>
      </dd>
    </dl>
    {#if settings.stopped}
      <p class="message message--warning" role="status">
        The workshop reached this month's cap. It stops thinking until next month or a higher cap.
      </p>
    {/if}
    <form
      class="form"
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <div class="field">
        <label class="field__label" for="workshop-provider">Provider</label>
        <stemma-select>
          <select id="workshop-provider" class="input" use:scriptedSelect={[provider]} bind:value={provider}>
            <option value="anthropic">Anthropic</option>
            <option value="openai">OpenAI</option>
            <option value="openrouter">OpenRouter</option>
          </select>
        </stemma-select>
      </div>
      <div class="field">
        <label class="field__label" for="workshop-model">Model</label>
        <input
          id="workshop-model"
          class="input"
          list="workshop-models"
          maxlength="200"
          autocomplete="off"
          required
          bind:value={model}
        />
        <datalist id="workshop-models">
          {#each SUGGESTIONS[provider] as suggestion (suggestion)}
            <option value={suggestion}></option>
          {/each}
        </datalist>
        <p class="field__hint">Whoever provides the key chooses the model.</p>
      </div>
      <div class="field">
        <label class="field__label" for="workshop-key">API key</label>
        <input
          id="workshop-key"
          class="input"
          type="password"
          autocomplete="new-password"
          placeholder={settings.has_key ? "Leave empty to keep the current key" : ""}
          bind:value={key}
        />
        <p class="field__hint">Written, never read back. Changing the provider or model asks for it again.</p>
      </div>
      <div class="field">
        <label class="field__label" for="workshop-cap">Tokens per month</label>
        <input id="workshop-cap" class="input" type="number" min="0" step="1" required bind:value={cap} />
        <p class="field__hint">Input and output together, as the provider reports them.</p>
      </div>
      <div class="field">
        <label class="field__label" for="workshop-steps">Steps per turn</label>
        <input id="workshop-steps" class="input" type="number" min="1" max="100" step="1" required bind:value={steps} />
        <p class="field__hint">How many calls the workshop makes each time it wakes, at most.</p>
      </div>
      <div class="row">
        <button class="button button--primary" type="submit" disabled={saving}>Save</button>
        {#if settings.has_key}
          <button class="button button--danger" type="button" disabled={saving} onclick={() => save(true)}>
            Remove key
          </button>
        {/if}
      </div>
    </form>
  {/if}
</section>
