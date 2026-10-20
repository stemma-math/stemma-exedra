<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import type { StemmaLight } from "@stemma-math/design-elements/light.js";
  import { doorway } from "../doorway.svelte.ts";
  import AccountFrame from "./AccountFrame.svelte";

  let { children }: { children: Snippet } = $props();
  let light = $state<StemmaLight>();
  const front = $derived(page.url.pathname === "/");
  let entering = false;

  onMount(() => {
    if (light) light.progress = front ? 0 : 1;
    doorway.enter = async (href) => {
      entering = true;
      const withdrawing = light?.withdraw();
      await goto(href);
      await withdrawing;
      entering = false;
    };
  });
  $effect(() => {
    if (!light || entering) return;
    if (front) void light.restore();
    else light.progress = 1;
  });
</script>

<div class="public-frame">
  <stemma-light bind:this={light} preset="hero" progress="1" class="public-frame__light"></stemma-light>
  <div class="public-frame__content">
    {#if front}{@render children()}{:else}<AccountFrame>{@render children()}</AccountFrame>{/if}
  </div>
</div>
