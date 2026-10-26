<script lang="ts">
  import { api, type Paper, type Thread } from "#lib/api.ts";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import type { Recipe } from "#lib/workshop.ts";

  let { group, recipe, inputs = $bindable({}) }: { group: string; recipe: Recipe; inputs: Record<string, string> } =
    $props();
  let papers = $state<Paper[]>([]);
  let threads = $state<Thread[]>([]);
  let declarations = $state<string[]>([]);

  $effect(() => {
    api
      .papers(group)
      .then((found) => {
        papers = found;
      })
      .catch(() => {});
    api
      .threads(group, { limit: 200 })
      .then((found) => {
        threads = found;
      })
      .catch(() => {});
  });

  let lookup = 0;
  async function search(query: string) {
    const turn = ++lookup;
    try {
      const found = await api.declarations(group, { q: query, limit: 20 });
      if (turn === lookup) declarations = found.map((d) => d.name);
    } catch {
      // A declaration typed in full is still checked by the hub.
    }
  }
</script>

{#each Object.entries(recipe.front.inputs) as [name, input] (name)}
  {@const required = input.required || input.kind === "paper"}
  <div class="field">
    <label class="field__label" for="recipe-input-{name}">{input.description}{required ? "" : " (optional)"}</label>
    {#if input.kind === "text"}
      <textarea id="recipe-input-{name}" class="input" rows="5" bind:value={inputs[name]} {required}></textarea>
    {:else if input.kind === "paper"}
      <stemma-select
        ><select use:scriptedSelect={[inputs[name], papers]} id="recipe-input-{name}" class="input" bind:value={inputs[name]} {required}>
          <option value="">Choose a paper</option>
          {#each papers as paper (paper.slug)}<option value={paper.slug}>{paper.title}</option>{/each}
        </select></stemma-select
      >
    {:else if input.kind === "thread"}
      <stemma-select
        ><select use:scriptedSelect={[inputs[name], threads]} id="recipe-input-{name}" class="input" bind:value={inputs[name]} {required}>
          <option value="">Choose a thread</option>
          {#each threads as thread (thread.number)}
            <option value="t{thread.number}">t{thread.number}, {thread.title}</option>
          {/each}
        </select></stemma-select
      >
    {:else}
      <input
        id="recipe-input-{name}"
        class="input input--mono"
        list="recipe-declarations-{name}"
        bind:value={inputs[name]}
        {required}
        oninput={(event) => search(event.currentTarget.value)}
        placeholder="Declaration name"
      />
      <datalist id="recipe-declarations-{name}">
        {#each declarations as declaration (declaration)}<option value={declaration}></option>{/each}
      </datalist>
    {/if}
  </div>
{/each}
