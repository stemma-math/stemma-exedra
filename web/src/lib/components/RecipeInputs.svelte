<script lang="ts">
  import { api, type Paper, type Thread } from "#lib/api.ts";
  import type { Recipe } from "#lib/workshop.ts";
  let { group, recipe, inputs = $bindable({}) }: { group: string; recipe: Recipe; inputs: Record<string,string> } = $props();
  let papers = $state<Paper[]>([]);
  let threads = $state<Thread[]>([]);
  let declarations = $state<string[]>([]);
  $effect(() => {
    api.papers(group).then(p => papers = p).catch(() => {});
    api.threads(group, {limit:200}).then(t => threads = t).catch(() => {});
  });
  let lookup = 0;
  async function search(query: string) {
    const turn = ++lookup;
    try { const found = await api.declarations(group, {q:query,limit:20}); if (turn === lookup) declarations = found.map(d => d.name); } catch { /* A free-text declaration can still be validated by the hub. */ }
  }
</script>
{#each Object.entries(recipe.front.inputs) as [name, input] (name)}
  {@const required = input.required || input.kind === "paper"}
  <div class="field">
    <label class="field__label" for="recipe-input-{name}">{input.description}{required ? " (required)" : ""}</label>
    {#if input.kind === "text"}
      <textarea id="recipe-input-{name}" class="input" rows="5" bind:value={inputs[name]} {required}></textarea>
    {:else if input.kind === "paper"}
      <stemma-select><select id="recipe-input-{name}" class="input" bind:value={inputs[name]} {required}>
        <option value="">Choose a paper</option>
        {#each papers as paper (paper.slug)}<option value={paper.slug}>{paper.title} ({paper.slug})</option>{/each}
      </select></stemma-select>
    {:else if input.kind === "thread"}
      <stemma-select><select id="recipe-input-{name}" class="input" bind:value={inputs[name]} {required}>
        <option value="">Choose a thread</option>
        {#each threads as thread (thread.number)}<option value="t{thread.number}">t{thread.number}: {thread.title}</option>{/each}
      </select></stemma-select>
    {:else}
      <input id="recipe-input-{name}" class="input" list="recipe-declarations-{name}" bind:value={inputs[name]} {required} oninput={event => search(event.currentTarget.value)} placeholder="Declaration name" />
      <datalist id="recipe-declarations-{name}">{#each declarations as name (name)}<option value={name}></option>{/each}</datalist>
    {/if}
  </div>
{/each}
