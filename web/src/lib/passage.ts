/** The content follows the same progress that withdraws the light. */
function smooth(from: number, to: number, point: number) {
  const k = Math.min(Math.max((point - from) / (to - from), 0), 1);
  return k * k * (3 - 2 * k);
}

export function passage(progress: number, reduced: boolean) {
  const gone = reduced ? progress : smooth(0, 0.22, progress);
  const come = reduced ? progress : smooth(0.45, 0.85, progress);
  return {
    frontOpacity: 1 - gone,
    frontScale: reduced ? 1 : 1 - 0.035 * gone,
    insideOpacity: come,
    insideRise: reduced ? 0 : 1 - come,
  };
}
