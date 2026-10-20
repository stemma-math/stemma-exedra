import { action, type Snapshot, snapshot, type WorkshopAgent } from "./api.ts";

/** Shared snapshot and drafts, refreshed together after a successful command. */
export class MachineControls {
  data = $state<Snapshot | null>(null);
  settingUp = $state(false);
  busy = $state(false);
  preparing = $state(false);
  error = $state("");
  notice = $state("");
  url = $state("");
  username = $state("");
  password = $state("");
  name = $state("This machine");
  folder = $state("");
  runtime = $state("auto");
  image = $state("");
  source = $state("git");
  agents = $state<WorkshopAgent[]>([]);
  runner = $state<string>("claude-code");
  credentialRunner = $state<string>("claude-code");
  env = $state("");
  files = $state("");

  async load() {
    const preserveOffers =
      JSON.stringify(this.agents) !== JSON.stringify(this.data?.machine?.workshop_agents ?? []);
    this.data = await snapshot();
    this.url = this.data.hub ?? "";
    this.username = this.data.username ?? "";
    this.name = this.data.name;
    this.folder = this.data.work_dir;
    this.runtime = this.data.runtime;
    this.image = this.data.image;
    if (!preserveOffers)
      this.agents = structuredClone($state.snapshot(this.data.machine?.workshop_agents ?? []));
    const key = `stemma-machine-setup:${this.data.hub}:${this.data.username}`;
    if (this.data.me && !this.data.registered) localStorage.setItem(key, "pending");
    if (
      this.data.registered &&
      this.data.agents.some((agent) => agent.installed) &&
      this.data.service.running
    )
      localStorage.removeItem(key);
    this.settingUp = localStorage.getItem(key) === "pending";
    this.loadCredentials();
  }

  loadCredentials() {
    const credentials = this.data?.credentials[this.credentialRunner];
    this.env = credentials?.env?.join("\n") ?? "";
    this.files =
      credentials?.files?.map((f) => (f.to ? `${f.from}=${f.to}` : f.from)).join("\n") ?? "";
  }

  async run(work: () => Promise<void>, message: string, reload = true) {
    this.busy = true;
    this.error = "";
    this.notice = "";
    try {
      await work();
      if (reload) await this.load();
      this.notice = message;
    } catch (error) {
      this.error = String(error);
      try {
        this.data = await snapshot();
      } catch {
        // Keep the last known state when the backend is unavailable.
      }
    } finally {
      this.busy = false;
    }
  }

  async prepare() {
    this.preparing = true;
    try {
      await this.save({ kind: "build", source: this.source }, "The workshop image is ready.");
    } finally {
      this.preparing = false;
    }
  }

  save(command: object, message = "Saved.") {
    return this.run(() => action(command), message);
  }

  refresh(message = "Refreshed.") {
    return this.run(() => this.load(), message, false);
  }
}
