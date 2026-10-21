<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { session } from "#lib/session.svelte.ts";
  import { connection } from "#lib/transport.svelte.ts";
  import { clampScale, MAX_SCALE, MIN_SCALE, readScale, SCALE_STEP, widthKey, writeScale } from "#lib/sidebar-width.ts";

  let { scale = $bindable(MIN_SCALE), dragging = $bindable(false) }: { scale?: number; dragging?: boolean } = $props();
  let handle: HTMLDivElement;
  let minimum = $state(0);
  let start: { pointer: number; x: number; scale: number } | null = null;
  const key = $derived(widthKey(connection.hub || page.url.origin, session.me?.id ?? 0));

  $effect(() => { scale = readScale(key); });

  onMount(() => {
    const sidebar = handle.parentElement?.querySelector(".sidebar");
    if (!sidebar) return;
    const observer = new ResizeObserver(() => {
      minimum = sidebar.getBoundingClientRect().width / scale;
    });
    observer.observe(sidebar);
    return () => { observer.disconnect(); dragging = false; };
  });

  function resize(value: number) {
    scale = clampScale(value);
    writeScale(key, scale);
  }

  function begin(event: PointerEvent) {
    if (event.button !== 0 || !event.isPrimary || !minimum) return;
    event.preventDefault();
    handle.focus();
    handle.setPointerCapture(event.pointerId);
    start = { pointer: event.pointerId, x: event.clientX, scale };
    dragging = true;
  }

  function move(event: PointerEvent) {
    if (!start || event.pointerId !== start.pointer) return;
    resize(start.scale + (event.clientX - start.x) / minimum);
  }

  function end(event: PointerEvent) {
    if (start?.pointer !== event.pointerId) return;
    cancel();
  }

  function cancel() {
    const pointer = start?.pointer;
    start = null;
    dragging = false;
    if (pointer !== undefined && handle.hasPointerCapture(pointer)) handle.releasePointerCapture(pointer);
  }

  function keydown(event: KeyboardEvent) {
    const value = event.key === "ArrowLeft" ? scale - SCALE_STEP
      : event.key === "ArrowRight" ? scale + SCALE_STEP
      : event.key === "Home" ? MIN_SCALE
      : event.key === "End" ? MAX_SCALE : null;
    if (value === null) return;
    event.preventDefault();
    resize(value);
  }
</script>

<svelte:window
  onpointermove={move}
  onpointerup={end}
  onpointercancel={end}
  onblur={cancel}
  onkeydown={(event) => { if (event.key === "Escape") cancel(); }}
/>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<!-- biome-ignore lint/a11y/useSemanticElements: A focusable window splitter is an adjustable widget, not a thematic break. -->
<div
  bind:this={handle}
  class="sidebar-resize"
  role="separator"
  aria-label="Sidebar width"
  aria-controls="sidebar"
  aria-orientation="vertical"
  aria-valuemin={Math.round(minimum)}
  aria-valuemax={Math.round(minimum * MAX_SCALE)}
  aria-valuenow={Math.round(minimum * scale)}
  tabindex="0"
  onpointerdown={begin}
  onlostpointercapture={end}
  onkeydown={keydown}
  ondblclick={() => resize(MIN_SCALE)}
></div>
