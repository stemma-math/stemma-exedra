import { action, type Snapshot, snapshot, type WorkshopAgent } from "./api.ts";

/** Shared snapshot and drafts, refreshed together after a successful command. */
export class MachineControls {
  data = $state<Snapshot | null>(null);
  busy = $state(false);
  error = $state("");
  notice = $state("");
  url = $state("");
  username = $state("");
  password = $state("");
  name = $state("This machine");
  folder = $state("");
  group = $state("");
  runtime = $state("auto");
  image = $state("");
  source = $state("git");
  agents = $state<WorkshopAgent[]>([]);
  runner = $state<string>("claude-code");
  credentialRunner = $state<string>("claude-code");
  env = $state("");
  files = $state("");

  async load() {
    this.data = await snapshot();
    this.url = this.data.hub ?? "";
    this.username = this.data.username ?? "";
    this.name = this.data.name;
    this.folder = this.data.work_dir;
    this.runtime = this.data.runtime;
    this.image = this.data.image;
    this.agents = structuredClone($state.snapshot(this.data.machine?.workshop_agents ?? []));
    if (!this.data.me?.groups.some((g) => g.slug === this.group)) {
      this.group = this.data.me?.groups[0]?.slug ?? "";
    }
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

  save(command: object, message = "Saved.") {
    return this.run(() => action(command), message);
  }

  refresh(message = "Refreshed.") {
    return this.run(() => this.load(), message, false);
  }
}
