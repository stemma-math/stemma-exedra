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
  /** The agents signed in for their containers. */
  signed_in: string[];
  checks: Check[];
  /** The name of Stemma's own agents' image; another is the person's own. */
  stemma_image: string;
}
export const snapshot = (group: string | null = null) => invoke<Snapshot>("snapshot", { group });
export const action = (action: object) => invoke<Explained | null>("action", { action });
export const login = (url: string, username: string, password: string) =>
  invoke<void>("login", { url, username, password });
export const signOut = () => invoke<string | null>("sign_out");

export const diagnostics = (group: string | null = null) =>
  invoke<Pick<Snapshot, "checks" | "service" | "secrets" | "signed_in">>("diagnostics", { group });
/** Asks an agent, in a container as an answer's, for one word; what it said. */
export const testSignIn = (agent: string, group: string | null) =>
  invoke<string>("test_sign_in", { agent, group });
export const openExternal = (url: string) => invoke<void>("open_external", { url });

export interface WorkshopState {
  store: {
    pending: {
      hub: string;
      owner: number;
      decision: string;
      offer: import("../workshop.ts").ProcessOffer;
    }[];
    approvals: {
      id: number;
      hub: string;
      group: number;
      recipe: string;
      version: string;
      requester: number;
      request: number | null;
      until: number | null;
      max_runs: number | null;
      minutes_per_run: number | null;
      runs: number;
      revoked: boolean;
    }[];
  };
  log: string;
}
export const workshopState = () => invoke<WorkshopState>("workshop_state");
export const workshopDecision = (
  mode: string,
  process: number | null,
  approval: number | null,
  limits: object = {},
) => invoke<void>("workshop_decision", { mode, process, approval, limits });
