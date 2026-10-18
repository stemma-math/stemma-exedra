<script lang="ts">
  import type { Paper } from "#lib/api.ts";
  import { when } from "#lib/format.ts";
  import BuildChip from "./BuildChip.svelte";

  let { papers, group }: { papers: Paper[]; group: string } = $props();
</script>

<div class="card">
  <div class="table papers">
    {#each papers as paper (paper.slug)}
      <a class="table__row" href="/g/{group}/p/{paper.slug}">
        <span class="stack stack--tight">
          <strong>{paper.title}</strong>
          <span class="quiet">
            <span class="identifier">p/{paper.slug}</span> · by {paper.authors.map((a) => a.display_name).join(", ")}
          </span>
        </span>
        <span class="quiet when">{paper.pdf ? `PDF ${when(paper.pdf.built_at)}` : "No PDF yet"}</span>
        {#if paper.latest}<BuildChip build={paper.latest} />{:else}<span></span>{/if}
      </a>
    {/each}
  </div>
</div>
