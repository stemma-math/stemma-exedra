/** A chat's title, from the question it starts with: its first line,
 * without Markdown's emphasis (underscores stay, for formulas), cut at a
 * word if long. */
export function titleOf(question: string, max = 120): string {
  const line =
    question
      .split("\n")
      .map((l) => l.replace(/^\s*(#+|>|[-*+]|\d+\.)\s+/, "").trim())
      .find((l) => l !== "") ?? "";
  const plain = line.replace(/[*`]+/g, "").replace(/\s+/g, " ").trim();
  if (plain.length <= max) return plain;
  const cut = plain.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return `${(space > max / 2 ? cut.slice(0, space) : cut).trimEnd()}…`;
}
