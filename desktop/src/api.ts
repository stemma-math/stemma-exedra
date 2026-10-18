import { invoke } from "@tauri-apps/api/core";
import type { Machine, Me, WorkshopAgent } from "../../web/src/lib/api.ts";

export { RUNNER_NAMES, RUNNERS } from "../../web/src/lib/api.ts";
export type { WorkshopAgent };
export interface Credentials {
  env?: string[];
  files?: { from: string; to?: string }[];
}
export interface Snapshot {
  hub: string | null;
  username: string | null;
  me: Me | null;
  hub_error: string | null;
  registered: boolean;
  machine: Machine | null;
  name: string;
  work_dir: string;
  personal: boolean;
  workshop: boolean;
  serving: boolean;
  agents: { runner: string; name: string; installed: boolean; sign_in: string }[];
  elan: string | null;
  latex: string | null;
  runtime: string;
  image: string;
  credentials: Record<string, Credentials>;
}
export const snapshot = () => invoke<Snapshot>("snapshot");
export const action = (action: object) => invoke<void>("action", { action });
export const login = (url: string, username: string, password: string) =>
  invoke<void>("login", { url, username, password });
export const openHub = () => invoke<void>("open_hub");
