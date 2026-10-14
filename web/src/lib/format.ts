// How times, sizes and people are written.

const dateTime = new Intl.DateTimeFormat("en", { dateStyle: "medium", timeStyle: "short" });
const time = new Intl.DateTimeFormat("en", { timeStyle: "short" });
const day = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const relative = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

/** A moment, briefly: "14:05" today, "Oct 3" before. */
export function when(iso: string, now = new Date()): string {
  const date = new Date(iso);
  const seconds = (now.getTime() - date.getTime()) / 1000;
  if (seconds < 60) return "just now";
  if (seconds < 3600) return relative.format(-Math.floor(seconds / 60), "minute");
  if (date.toDateString() === now.toDateString()) return time.format(date);
  if (seconds < 6 * 86400) return relative.format(-Math.max(1, Math.round(seconds / 86400)), "day");
  return day.format(date);
}

/** A moment, in full. */
export function exactly(iso: string): string {
  return dateTime.format(new Date(iso));
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let size = bytes / 1024;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit++;
  }
  return `${size < 10 ? size.toFixed(1) : Math.round(size)} ${units[unit]}`;
}

/** The categorical tones that tell people apart. */
const TONES = ["indigo", "teal", "violet", "ochre", "magenta", "olive", "cerulean", "rust"];

export function toneOf(id: number): string {
  return TONES[Math.abs(id) % TONES.length];
}

export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? [words[0], words[words.length - 1]] : words;
  return letters.map((w) => [...w][0]?.toUpperCase() ?? "").join("") || "?";
}
