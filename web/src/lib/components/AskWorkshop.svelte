<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, type Member } from "#lib/api.ts";
  import type { Ask, RecipeFile } from "#lib/workshop.ts";
  import { toastError } from "#lib/toast.ts";
  import RecipeInputs from "./RecipeInputs.svelte";
  let { group, recipes, oncancel }: { group: string; recipes: RecipeFile[]; oncancel: () => void } = $props();
  let selected = $state("");
  const file = $derived(recipes.find(r => r.name === selected));
  const recipe = $derived(file?.recipe);
  let title = $state("");
  let inputs = $state<Record<string,string>>({});
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
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  $effect(() => {api.members(group).then(m => members = m.filter(p => p.role !== "guest")).catch(toastError);});
  $effect(() => {
    if (!selected) {
      const first = recipes.find(r => r.recipe);
      if (first?.recipe) { selected = first.name; title = first.recipe.front.title; runner = first.recipe.front.runners[0] ?? "claude"; }
    }
  });
  function choose() { inputs = {}; title = recipe?.front.title ?? ""; runner = recipe?.front.runners[0] ?? "claude"; }
  async function submit(event: SubmitEvent) {
    event.preventDefault(); error = "";
    if (!recipe) return;
    if (when !== "now" && !at) { error = "Choose the first date and time."; return; }
    saving = true;
    try {
      const ask: Ask = { recipe: selected, title, inputs,
        assignment: who === "member" ? {kind:"member",user:Number(member)} : {kind: who === "anyone" ? "anyone" : "mine"}, runner, model: model.trim() || null,
        schedule: { at: when === "now" ? null : new Date(at).getTime(), timezone,
          every: when === "every" ? frequency === "hours" ? {kind:"hours",hours} : {kind: frequency === "week" ? "week" : "day"} : null,
          until: until && when === "every" ? new Date(`${until}T23:59:59`).getTime() : null }, wait_hours:wait };
      const request = await api.askWorkshop(group,ask); await goto(`/g/${group}/t/${request.thread.number}`);
    } catch(e) { error = e instanceof Error ? e.message : String(e); } finally { saving = false; }
  }
</script>
<form class="card card__body stack" onsubmit={submit}>
  <h2>Ask the workshop</h2>
  <div class="field"><label class="field__label" for="ask-recipe">Recipe</label><stemma-select><select id="ask-recipe" class="input" bind:value={selected} onchange={choose} required>
    {#each recipes.filter(r => r.recipe) as r (r.name)}<option value={r.name}>{r.recipe?.front.title}</option>{/each}
  </select></stemma-select></div>
  {#if recipe}
    <p class="quiet">{recipe.front.summary}</p>
    <div class="field"><label class="field__label" for="ask-title">Request title</label><input id="ask-title" class="input" bind:value={title} required maxlength="200" placeholder={recipe.front.title} /></div>
    <RecipeInputs {group} {recipe} bind:inputs />
    <fieldset class="fieldset stack"><legend>Who runs it</legend>
      <stemma-select><select class="input" aria-label="Whose machines" bind:value={who}><option value="mine">My machines</option><option value="member">A member's machines</option><option value="anyone">Anyone's machine</option></select></stemma-select>
      {#if who === "member"}<stemma-select><select class="input" aria-label="Member" bind:value={member} required><option value="">Choose a member</option>{#each members as m (m.id)}<option value={String(m.id)}>{m.display_name}</option>{/each}</select></stemma-select>{/if}
      <div class="field"><label class="field__label" for="ask-runner">Runner</label><stemma-select><select id="ask-runner" class="input" bind:value={runner}>{#each (recipe.front.runners.length ? recipe.front.runners : ["claude","codex","opencode"]) as r (r)}<option value={r}>{r === "claude" ? "Claude" : r === "codex" ? "Codex" : "OpenCode"}</option>{/each}</select></stemma-select></div>
      <div class="field"><label class="field__label" for="ask-model">Model (optional)</label><input id="ask-model" class="input" bind:value={model} placeholder="The machine's configured model" /></div>
      <p class="quiet">Other people approve on their own machines. This request cannot use their models until they approve.</p>
    </fieldset>
    <fieldset class="fieldset stack"><legend>When</legend>
      <stemma-select><select class="input" aria-label="When to run" bind:value={when}><option value="now">Now</option><option value="at">At a date and time</option><option value="every">Every so often</option></select></stemma-select>
      {#if when !== "now"}<div class="field"><label class="field__label" for="ask-at">{when === "every" ? "First run" : "Date and time"}</label><input id="ask-at" class="input" type="datetime-local" bind:value={at} required /></div>{/if}
      {#if when === "every"}
        <stemma-select><select class="input" aria-label="Repeat every" bind:value={frequency}><option value="day">Day, at this time</option><option value="week">Week, at this time</option><option value="hours">N hours</option></select></stemma-select>
        {#if frequency === "hours"}<div class="field"><label class="field__label" for="ask-hours">Hours between runs</label><input id="ask-hours" class="input" type="number" min="1" bind:value={hours} required /></div>{/if}
        <div class="field"><label class="field__label" for="ask-until">Until (optional, otherwise until stopped)</label><input id="ask-until" class="input" type="date" bind:value={until} /></div>
      {/if}
      <p class="quiet">Time zone: {timezone}. Daily and weekly requests keep this local time.</p>
    </fieldset>
    <div class="field"><label class="field__label" for="ask-wait">Wait for a machine for at most</label><stemma-select><select id="ask-wait" class="input" bind:value={wait}><option value={1}>An hour</option><option value={24}>A day</option><option value={168}>A week</option></select></stemma-select></div>
    {#if error}<p class="message message--danger" role="alert">{error}</p>{/if}
    <div class="row"><button class="button button--primary" type="submit" disabled={saving}>{saving ? "Asking…" : "Ask"}</button><button class="button button--quiet" type="button" onclick={oncancel}>Cancel</button></div>
  {/if}
</form>
