// Keep design's scripted list in step with Svelte's native values and options.
import { tick } from "svelte";
import type { StemmaSelect } from "@stemma-math/design-elements/select.js";

export function scriptedSelect(select: HTMLSelectElement, _state: unknown) {
  const refresh = () => {
    void tick().then(() => select.closest<StemmaSelect>("stemma-select")?.refresh());
  };
  const observer = new MutationObserver(refresh);
  observer.observe(select, { attributes: true, childList: true, subtree: true });
  refresh();
  return { update: refresh, destroy: () => observer.disconnect() };
}
