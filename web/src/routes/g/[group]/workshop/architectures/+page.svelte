<script lang="ts">
  import { api, type Architecture } from "#lib/api.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let architectures = $state<Architecture[] | null>(null);

  $effect(() => {
    api
      .architectures()
      .then((found) => {
        architectures = found;
      })
      .catch((error) => {
        toastError(error);
        architectures = [];
      });
  });

  /** What an architecture works on, in words. */
  function worksOn(target: string): string {
    return target === "paper" ? "the task's paper" : `the group's ${target}`;
  }
</script>

<svelte:head><title>Architectures – Workshop – {group.name} – Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/workshop">Workshop</a></li>
      <li><span aria-current="page">Architectures</span></li>
    </ol>
  </nav>
  <h1>Architectures</h1>
  <p class="quiet">
    Each kind of work the workshop knows is an architecture: a folder of Stemma's
    <code>workshop/architectures/</code>, with its steps and its agent's instructions. A task names one, or the
    workshop chooses it by what the task changes.
  </p>

  {#if architectures === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else}
    {#each architectures as architecture (architecture.name)}
      <section class="card card__body stack">
        <div class="thread-head">
          <h2 class="identifier">{architecture.name}</h2>
          <span class="chip">On {worksOn(architecture.works_on)}</span>
        </div>
        <p>{architecture.description}</p>
        <div class="stack stack--tight">
          <h3 class="label">Steps</h3>
          <ol class="architecture-steps">
            {#each architecture.steps as step (step.name)}
              <li><span class="identifier">{step.name}</span>: {step.description}</li>
            {/each}
          </ol>
        </div>
        {#if architecture.big.length}
          <div class="stack stack--tight">
            <h3 class="label">A big change touches</h3>
            <p class="row">
              {#each architecture.big as file (file)}<span class="chip chip--outline identifier">{file}</span>{/each}
            </p>
            <p class="quiet">Or deletes or renames a file; an admin approves it too.</p>
          </div>
        {/if}
        {#if architecture.instructions.length}
          <p class="quiet">
            Its instructions:
            {#each architecture.instructions as file, i (file)}{i ? ", " : ""}<span class="identifier"
                >workshop/architectures/{architecture.name}/{file}</span
              >{/each}.
          </p>
        {/if}
      </section>
    {/each}
  {/if}
</main>
