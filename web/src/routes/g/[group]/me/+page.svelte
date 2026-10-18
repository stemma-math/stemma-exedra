<script lang="ts">
  import PersonalSettings from "#lib/components/PersonalSettings.svelte";
  import { groupContext } from "#lib/group.svelte.ts";

  const context = groupContext();
  const group = $derived(context.group);
</script>

<svelte:head><title>Personal settings · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack stack--loose">
  <div class="page__head">
    <h1>Personal settings</h1>
  </div>
  <p class="quiet">Yours, in every group you are in: your agents, the machines that run them, and your tokens.</p>
  <PersonalSettings />
</main>
