import { invoke } from "@tauri-apps/api/core";
import type { Me } from "../api.ts";
export interface Explained {
  key: string;
  value: string | number | boolean | unknown[];
  reset?: Explained;
  source: { kind: "default" | "file" | "group" | "environment" | "command_line"; name?: string };
}
export interface Check {
  name: string;
  status: "success" | "warning" | "danger";
  detail: string;
  purpose: string | null;
  remedy: string | null;
}
export interface Snapshot {
  hub: string | null;
  username: string | null;
  me: Me | null;
  hub_error: string | null;
  service: {
    installed: boolean;
    running: boolean;
    connected: boolean | null;
    signed_out: boolean;
    logs: string;
  };
  settings: Explained[];
  /** The names of the containers' sign-ins kept, never their values. */
  secrets: string[];
  checks: Check[];
}
export const snapshot = (group: string | null = null) => invoke<Snapshot>("snapshot", { group });
export const action = (action: object) => invoke<Explained | null>("action", { action });
export const login = (url: string, username: string, password: string) =>
  invoke<void>("login", { url, username, password });
export const signOut = () => invoke<string | null>("sign_out");

export const diagnostics = (group: string | null = null) =>
  invoke<Pick<Snapshot, "checks" | "service" | "secrets">>("diagnostics", { group });
export const openExternal = (url: string) => invoke<void>("open_external", { url });
