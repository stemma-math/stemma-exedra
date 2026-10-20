<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import { onNavigate } from "$app/navigation";
  import { page } from "$app/state";
  import type { StemmaLight } from "@stemma-math/design-elements/light.js";
  import { passage } from "../passage.ts";
  import AccountFrame from "./AccountFrame.svelte";
  import Front from "./Front.svelte";

  let { children }: { children: Snippet } = $props();
  let light = $state<StemmaLight>();
  const front = $derived(page.url.pathname === "/");
  let progress = $state(page.url.pathname === "/" ? 0 : 1);
  let reduced = $state(false);
  let moving = $state(false);
  let cancel = () => {};
  const view = $derived(passage(progress, reduced));

  function move(to: number): Promise<void> {
    cancel();
    const from = progress;
    const styles = getComputedStyle(document.documentElement);
    const duration = Number.parseFloat(
      styles.getPropertyValue(reduced ? "--dur-ui" : "--dur-long"),
    );
    const since = performance.now();
    moving = true;
    return new Promise((resolve) => {
      let frame = 0;
      const finish = () => {
        moving = false;
        resolve();
      };
      cancel = () => {
        cancelAnimationFrame(frame);
        finish();
      };
      const draw = (now: number) => {
        const k = Math.min((now - since) / duration, 1);
        const point = from + (to - from) * (1 - (1 - k) ** 3);
        if (light) light.progress = reduced ? 0 : point;
        progress = reduced ? point : (light?.progress ?? point);
        if (k < 1) frame = requestAnimationFrame(draw);
        else finish();
      };
      frame = requestAnimationFrame(draw);
    });
  }

  onNavigate((navigation) => {
    if (navigation.from?.url.pathname === "/" && navigation.to?.url.pathname !== "/")
      return () => {
        void move(1);
      };
    if (navigation.from?.url.pathname !== "/" && navigation.to?.url.pathname === "/")
      return move(0);
  });

  onMount(() => {
    if (light) light.progress = reduced ? 0 : progress;
    const query = matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => {
      reduced = query.matches;
    };
    update();
    query.addEventListener("change", update);
    return () => {
      cancel();
      query.removeEventListener("change", update);
    };
  });
</script>

<div class="public-frame">
  <stemma-light
    bind:this={light}
    preset="hero"
    {progress}
    class="public-frame__light"
  ></stemma-light>
  <div
    class="public-frame__view public-frame__front"
    inert={!front || moving}
    style:opacity={view.frontOpacity}
    style:transform="scale({view.frontScale})"
    style:visibility={progress === 1 ? "hidden" : "visible"}
  >
    <Front />
    {#if front}
      {@render children()}
    {/if}
  </div>
  <div
    class="public-frame__view public-frame__inside"
    inert={front || moving}
    style:opacity={view.insideOpacity}
    style:transform="translateY(calc(var(--space-4) * {view.insideRise}))"
    style:visibility={progress === 0 ? "hidden" : "visible"}
  >
    {#if !front}
      <AccountFrame>{@render children()}</AccountFrame>
    {/if}
  </div>
</div>
