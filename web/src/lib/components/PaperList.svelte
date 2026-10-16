<script lang="ts">
  import type { Paper } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";
  import Attribution from "./Attribution.svelte";

  let { papers, group }: { papers: Paper[]; group: string } = $props();
</script>

<div class="card">
  <div class="table papers">
    {#each papers as paper (paper.slug)}
      <a class="table__row" href="/g/{group}/p/{paper.slug}">
        <span class="stack stack--tight">
          <span class="thread-row__title">
            <strong>{paper.title}</strong>
            {#if paper.last_read === null}
              <span class="chip chip--accent">New to you</span>
            {:else if paper.last_read < paper.revision}
              <span class="chip chip--accent">Changed since you read it</span>
            {/if}
          </span>
          <span class="quiet">
            <span class="identifier">p/{paper.slug}</span> · revision {paper.revision}, by
            <Attribution user={paper.updated_by} agent={paper.updated_agent} />
          </span>
        </span>
        <time class="quiet when" datetime={paper.updated_at} title={exactly(paper.updated_at)}>
          {when(paper.updated_at)}
        </time>
      </a>
    {/each}
  </div>
</div>
