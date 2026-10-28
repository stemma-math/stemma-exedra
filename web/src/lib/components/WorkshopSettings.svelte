<script lang="ts">
  import { api } from "#lib/api.ts";
  import { ask } from "#lib/ask.svelte.ts";
  import { exactly } from "#lib/format.ts";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { toastError } from "#lib/toast.ts";
  import type { WorkshopModel } from "#lib/workshop.ts";
  import Glyph from "./Glyph.svelte";

  let { group }: { group: string } = $props();
  let settings = $state<WorkshopModel | null>(null);
  let provider = $state<WorkshopModel["provider"]>("anthropic");
  let model = $state("");
  let key = $state("");
  let cap = $state(1_000_000);
  let steps = $state(20);
  let saving = $state(false);
  /** Whether the key's form is open over a key that is set. */
  let changing = $state(false);

  const PROVIDERS: Record<WorkshopModel["provider"], string> = {
    anthropic: "Anthropic",
    openai: "OpenAI",
    openrouter: "OpenRouter",
  };

  /** A few models per provider; any other identifier works too. */
  const SUGGESTIONS: Record<WorkshopModel["provider"], string[]> = {
    anthropic: ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"],
    openai: ["gpt-6.1-sol", "gpt-5.6-sol"],
    openrouter: ["anthropic/claude-sonnet-5.5", "openai/gpt-6.1-sol"],
  };

  const used = $derived(settings ? settings.input_tokens + settings.output_tokens : 0);

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

  /** Saves the key and its model, or, with `null`, the limits alone; an
   * empty key removes it. */
  async function save(apiKey: string | null) {
    if (!settings) return;
    saving = true;
    try {
      const keyed = apiKey !== null;
      show(
        await api.updateWorkshopModel(group, {
          provider: keyed ? provider : settings.provider,
          model: keyed ? model : settings.model,
          api_key: apiKey,
          monthly_cap: cap,
          step_cap: steps,
        }),
      );
      key = "";
      changing = false;
    } catch (error) {
      toastError(error);
    } finally {
      saving = false;
    }
  }

  async function removeKey() {
    const yes = await ask({
      title: "Remove the workshop's key?",
      text: "The workshop stops thinking until an admin gives it another. Members can still choose recipes themselves.",
      action: "Remove the key",
      danger: true,
    });
    if (yes) await save("");
  }

  function cancel() {
    if (settings) show(settings);
    key = "";
    changing = false;
  }
</script>

<section class="stack stack--loose" aria-label="Workshop">
  <p>
    The workshop thinks with the group's model: it turns requests in plain words into processes, and reviews and
    merges their pull requests. It needs an API key, never a subscription's sign-in.
  </p>
  {#if settings}
    <section class="stack">
      <h2>Model</h2>
      {#if settings.has_key && !changing}
        <ul class="card row-list">
          <li class="row-list__item">
            <span class="row-list__text">
              <span class="row-list__name">
                {PROVIDERS[settings.provider]}, <span class="identifier">{settings.model}</span>
                <span class="chip chip--small chip--success"><Glyph name="lock" small />Key set</span>
              </span>
              <span class="row-list__description">
                Ending in <span class="identifier">{settings.last_four}</span>, set by
                {settings.set_by?.display_name ?? "a deleted account"}{#if settings.set_at !== null}, {exactly(settings.set_at)}{/if}
              </span>
            </span>
            <span class="row-list__control">
              <button type="button" class="button button--small" disabled={saving} onclick={() => (changing = true)}>Change</button>
              <button type="button" class="button button--quiet button--small button--danger" disabled={saving} onclick={removeKey}>Remove</button>
            </span>
          </li>
        </ul>
      {:else}
        {#if !settings.has_key}
          <p class="message message--info">
            <Glyph name="info" />
            <span class="message__text">The workshop has no key yet, so it cannot think. Members can still choose recipes themselves.</span>
          </p>
        {/if}
        <form
          class="form"
          onsubmit={(event) => {
            event.preventDefault();
            void save(key);
          }}
        >
          <div class="field">
            <label class="field__label" for="workshop-provider">Provider</label>
            <stemma-select>
              <select id="workshop-provider" class="input" use:scriptedSelect={[provider]} bind:value={provider}>
                {#each Object.entries(PROVIDERS) as [value, name] (value)}
                  <option {value}>{name}</option>
                {/each}
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
              spellcheck="false"
              required
              bind:value={model}
            />
            <datalist id="workshop-models">
              {#each SUGGESTIONS[provider] as suggestion (suggestion)}
                <option value={suggestion}></option>
              {/each}
            </datalist>
            <span class="field__hint">The provider's name for it, such as {SUGGESTIONS[provider][0]}.</span>
          </div>
          <div class="field">
            <label class="field__label" for="workshop-key">API key</label>
            <input id="workshop-key" class="input" type="password" autocomplete="off" required bind:value={key} />
            <span class="field__hint">
              Make one for this, with a spending limit at the provider. The hub keeps it encrypted and never shows it
              again, but whoever runs the hub could read it.
            </span>
          </div>
          <div class="row row--end">
            {#if settings.has_key}
              <button class="button button--quiet" type="button" disabled={saving} onclick={cancel}>Cancel</button>
            {/if}
            <button class="button button--primary" type="submit" disabled={saving}>
              {#if saving}<span class="spinner" aria-hidden="true"></span>{/if}Save the key
            </button>
          </div>
        </form>
      {/if}
    </section>

    <section class="stack">
      <h2>Limits</h2>
      <div>
        <div class="progress-label">
          <span>This month, {settings.month} in UTC</span>
          <span>
            {used.toLocaleString("en")} of {settings.monthly_cap.toLocaleString("en")} tokens{#if settings.cost_usd !== null},
              ${settings.cost_usd.toFixed(2)}{/if}
          </span>
        </div>
        <progress
          class={["progress", settings.stopped && "progress--danger"]}
          aria-label="Tokens used this month"
          value={Math.min(used, settings.monthly_cap)}
          max={Math.max(settings.monthly_cap, 1)}
        ></progress>
      </div>
      {#if settings.stopped}
        <p class="message message--warning" role="status">
          <Glyph name="warning" />
          <span class="message__text">The workshop reached this month's cap. It stops thinking until next month or a higher cap.</span>
        </p>
      {/if}
      <form
        class="form"
        onsubmit={(event) => {
          event.preventDefault();
          void save(null);
        }}
      >
        <div class="field">
          <label class="field__label" for="workshop-cap">Tokens per month</label>
          <input id="workshop-cap" class="input" type="number" min="0" step="1" required bind:value={cap} />
          <span class="field__hint">Input and output together, as the provider reports them.</span>
        </div>
        <div class="field">
          <label class="field__label" for="workshop-steps">Steps per turn</label>
          <input id="workshop-steps" class="input" type="number" min="1" max="100" step="1" required bind:value={steps} />
          <span class="field__hint">How many calls the workshop makes each time it wakes, at most.</span>
        </div>
        <div class="row row--end">
          <button
            class="button"
            type="submit"
            disabled={saving || (cap === settings.monthly_cap && steps === settings.step_cap)}>Save the limits</button
          >
        </div>
      </form>
    </section>
  {/if}
</section>
