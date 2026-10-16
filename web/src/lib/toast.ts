// Toasts, in the region the layout holds.

import type { StemmaToastRegion } from "@stemma-math/design-elements";

type Intent = "neutral" | "success" | "warning" | "danger" | "info";

export function toast(text: string, intent: Intent = "neutral") {
  const region = document.querySelector<StemmaToastRegion>("stemma-toast-region");
  region?.show({ text, intent });
}

/** Says what went wrong, from an error of any kind. */
export function toastError(error: unknown) {
  toast(error instanceof Error ? error.message : String(error), "danger");
}
