<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, type Member } from "#lib/api.ts";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { toastError } from "#lib/toast.ts";
  import { runnerName, type Ask, type RecipeFile } from "#lib/workshop.ts";
  import RecipeInputs from "./RecipeInputs.svelte";

  let { group, recipes, oncancel }: { group: string; recipes: RecipeFile[]; oncancel: () => void } = $props();

  let selected = $state("");
  let free = $state(true);
  let title = $state("");
  let details = $state("");
  let inputs = $state<Record<string, string>>({});
  let who = $state("mine");
  let member = $state("");
  let members = $state<Member[]>([]);
  let runner = $state("claude");
  let model = $state("");
  let when = $state("now");
  let at = $state("");
  let frequency = $state("day");
  let hours = $state(24);
  let until = $state("");
  let wait = $state(24);
  let saving = $state(false);
  let error = $state("");

  const recipe = $derived(recipes.find((r) => r.name === selected)?.recipe);
  const runners = $derived(recipe?.front.runners.length ? recipe.front.runners : ["claude", "codex", "opencode"]);
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;

  $effect(() => {
    api
      .members(group)
      .then((found) => {
        members = found.filter((m) => m.role !== "guest");
      })
      .catch(toastError);
  });

  // The first valid recipe, until one is chosen.
  $effect(() => {
    if (selected) return;
    const first = recipes.find((r) => r.recipe);
    if (first) {
      selected = first.name;
      if (!free) choose();
    }
  });

  function choose() {
    inputs = {};
    title = recipe?.front.title ?? "";
    runner = recipe?.front.runners[0] ?? "claude";
  }

  function schedule(): Ask["schedule"] {
    if (when === "now") return { at: null, timezone, every: null, until: null };
    const every: Ask["schedule"]["every"] =
      when !== "every"
        ? null
        : frequency === "hours"
          ? { kind: "hours", hours }
          : { kind: frequency === "week" ? "week" : "day" };
    return {
      at: new Date(at).getTime(),
      timezone,
      every,
      until: every && until ? new Date(`${until}T23:59:59`).getTime() : null,
    };
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    error = "";
    if (!free && !recipe) return;
    if (!free && when !== "now" && !at) {
      error = "Choose the first date and time.";
      return;
    }
    saving = true;
    try {
      const ask: Ask = {
        recipe: free ? "" : selected,
        title,
        inputs: free ? (details.trim() ? { details } : {}) : inputs,
        assignment:
          who === "member"
            ? { kind: "member", user: Number(member) }
            : { kind: who === "anyone" ? "anyone" : "mine" },
        runner,
        model: model.trim() || null,
        schedule: free ? { at: null, timezone, every: null, until: null } : schedule(),
        wait_hours: wait,
      };
      const request = await api.askWorkshop(group, ask);
      await goto(`/g/${group}/t/${request.thread.number}`);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<form class="card card__body stack" onsubmit={submit}>
  <h2>Ask the workshop</h2>
  <stemma-segmented label="How to ask">
    <div class="segmented">
      <button type="button" aria-pressed={free} onclick={() => (free = true)}>In plain words</button>
      <button
        type="button"
        aria-pressed={!free}
        onclick={() => {
          if (free) choose();
          free = false;
        }}
      >
        With a recipe
      </button>
    </div>
  </stemma-segmented>
  {#if free}
    <div class="field">
      <label class="field__label" for="ask-title">What should the workshop do?</label>
      <input
        id="ask-title"
        class="input"
        bind:value={title}
        required
        maxlength="200"
        placeholder="Prove lemma 3.2 with Claude"
      />
    </div>
    <div class="field">
      <label class="field__label" for="ask-details">Details (optional)</label>
      <textarea id="ask-details" class="input" rows="4" bind:value={details}></textarea>
      <span class="field__hint">
        The workshop chooses recipes and machines, with the group's model. Their owners approve each run.
      </span>
    </div>
  {:else}
    <div class="field">
      <label class="field__label" for="ask-recipe">Recipe</label>
      <stemma-select>
        <select
          use:scriptedSelect={[selected, recipes]}
          id="ask-recipe"
          class="input"
          bind:value={selected}
          onchange={choose}
          required
        >
          {#each recipes.filter((r) => r.recipe) as r (r.name)}
            <option value={r.name}>{r.recipe?.front.title}</option>
          {/each}
        </select>
      </stemma-select>
    </div>
    {#if recipe}
      <p class="quiet">{recipe.front.summary}</p>
      <div class="field">
        <label class="field__label" for="ask-title">Title</label>
        <input
          id="ask-title"
          class="input"
          bind:value={title}
          required
          maxlength="200"
          placeholder={recipe.front.title}
        />
      </div>
      <RecipeInputs {group} {recipe} bind:inputs />
      <fieldset class="fieldset stack">
        <legend>Who runs it</legend>
        <stemma-select>
          <select class="input" aria-label="Whose machines" bind:value={who}>
            <option value="mine">My machines</option>
            <option value="member">A member's machines</option>
            <option value="anyone">Anyone's machine</option>
          </select>
        </stemma-select>
        {#if who === "member"}
          <stemma-select>
            <select
              use:scriptedSelect={[member, members]}
              class="input"
              aria-label="Member"
              bind:value={member}
              required
            >
              <option value="">Choose a member</option>
              {#each members as m (m.id)}
                <option value={String(m.id)}>{m.display_name}</option>
              {/each}
            </select>
          </stemma-select>
        {/if}
        <div class="field">
          <label class="field__label" for="ask-runner">Agent</label>
          <stemma-select>
            <select
              use:scriptedSelect={[runner, recipe]}
              id="ask-runner"
              class="input"
              bind:value={runner}
            >
              {#each runners as r (r)}
                <option value={r}>{runnerName(r)}</option>
              {/each}
            </select>
          </stemma-select>
        </div>
        <div class="field">
          <label class="field__label" for="ask-model">Model (optional)</label>
          <input id="ask-model" class="input" bind:value={model} placeholder="The machine's own" />
        </div>
        <span class="field__hint">
          Others approve on their own machines: the request waits until they do.
        </span>
      </fieldset>
      <fieldset class="fieldset stack">
        <legend>When</legend>
        <stemma-select>
          <select class="input" aria-label="When to run" bind:value={when}>
            <option value="now">Now</option>
            <option value="at">At a date and time</option>
            <option value="every">Every so often</option>
          </select>
        </stemma-select>
        {#if when !== "now"}
          <div class="field">
            <label class="field__label" for="ask-at">{when === "every" ? "First run" : "Date and time"}</label>
            <stemma-date-picker placeholder="Choose when">
              <input id="ask-at" class="input" type="datetime-local" bind:value={at} required />
            </stemma-date-picker>
          </div>
        {/if}
        {#if when === "every"}
          <stemma-select>
            <select class="input" aria-label="Repeat every" bind:value={frequency}>
              <option value="day">Every day, at this time</option>
              <option value="week">Every week, at this time</option>
              <option value="hours">Every few hours</option>
            </select>
          </stemma-select>
          {#if frequency === "hours"}
            <div class="field">
              <label class="field__label" for="ask-hours">Hours between runs</label>
              <input id="ask-hours" class="input" type="number" min="1" bind:value={hours} required />
            </div>
          {/if}
          <div class="field">
            <label class="field__label" for="ask-until">Until (optional)</label>
            <stemma-date-picker placeholder="Until it is stopped">
              <input id="ask-until" class="input" type="date" bind:value={until} />
            </stemma-date-picker>
          </div>
        {/if}
        <span class="field__hint">In {timezone}: daily and weekly requests keep this local time.</span>
      </fieldset>
      <div class="field">
        <label class="field__label" for="ask-wait">Wait for a machine</label>
        <stemma-select>
          <select id="ask-wait" class="input" bind:value={wait}>
            <option value={1}>An hour</option>
            <option value={24}>A day</option>
            <option value={168}>A week</option>
          </select>
        </stemma-select>
      </div>
    {/if}
  {/if}
  {#if error}
    <p class="message message--danger" role="alert">{error}</p>
  {/if}
  <div class="row">
    <button class="button button--primary" type="submit" disabled={saving || (!free && !recipe)}>
      {saving ? "Asking…" : "Ask"}
    </button>
    <button class="button button--quiet" type="button" onclick={oncancel}>Cancel</button>
  </div>
</form>
