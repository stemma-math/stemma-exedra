<script lang="ts">
  import { agentMark } from "#lib/agents.ts";
  import { initials } from "#lib/format.ts";

  let {
    name,
    runner = null,
    appearance = null,
    small = false,
  }: { name: string; runner?: string | null; appearance?: "light" | "dark" | null; small?: boolean } = $props();
  const mark = $derived(agentMark(runner, name));
</script>

<span class="avatar" class:avatar--small={small} class:agent-avatar={Boolean(mark)} title={name} aria-hidden="true">
  {#if mark === "claude"}
    <img src="/agents/claude.svg" alt="" />
  {:else if mark && appearance}
    <!-- On a surface of a fixed appearance, its mark for that appearance. -->
    <img src="/agents/{mark}-{appearance}.svg" alt="" />
  {:else if mark}
    <img class="agent-mark--light" src="/agents/{mark}-light.svg" alt="" />
    <img class="agent-mark--dark" src="/agents/{mark}-dark.svg" alt="" />
  {:else}
    {initials(name)}
  {/if}
</span>

<style>
  .agent-avatar {
    background: var(--surface-card);
  }

  .agent-avatar > img {
    box-sizing: border-box;
    object-fit: contain;
    padding: var(--space-2);
  }

  .avatar--small.agent-avatar > img {
    padding: var(--space-1);
  }

  .agent-mark--dark {
    display: none;
  }

  :global(:root[data-theme="dark"]) .agent-mark--dark {
    display: block;
  }

  :global(:root[data-theme="dark"]) .agent-mark--light {
    display: none;
  }

  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .agent-mark--dark {
      display: block;
    }

    :global(:root:not([data-theme="light"])) .agent-mark--light {
      display: none;
    }
  }
</style>
