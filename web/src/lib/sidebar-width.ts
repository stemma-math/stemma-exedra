/** A proportion of the sidebar's existing, token-based width. */
export const MIN_SCALE = 1;
export const MAX_SCALE = 4 / 3;
export const SCALE_STEP = 1 / 32;

export function clampScale(value: number): number {
  return Number.isFinite(value) ? Math.min(MAX_SCALE, Math.max(MIN_SCALE, value)) : MIN_SCALE;
}

export function widthKey(hub: string, viewer: number): string {
  return `stemma-sidebar-width:${hub}:${viewer}`;
}

export function readScale(key: string): number {
  try {
    return clampScale(Number(localStorage.getItem(key) ?? MIN_SCALE));
  } catch {
    return MIN_SCALE;
  }
}

export function writeScale(key: string, value: number) {
  try {
    localStorage.setItem(key, String(clampScale(value)));
  } catch {
    // Resizing works without storage; only remembering it is optional.
  }
}
