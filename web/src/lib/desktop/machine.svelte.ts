import { action, type Snapshot, snapshot } from "./api.ts";

export class MachineControls {
  data = $state<Snapshot | null>(null);
  scope = $state("");
  settingUp = $state(false);
  busy = $state(false);
  loading = $state(false);
  working = $state("");
  private revision = 0;
  error = $state("");
  notice = $state("");
  url = $state("");
  username = $state("");
  password = $state("");
  async load() {
    const revision = ++this.revision;
    const scope = this.scope;
    this.loading = true;
    try {
      const data = await snapshot(scope || null);
      if (revision !== this.revision || scope !== this.scope) return;
      this.data = data;
      this.url = data.hub ?? "";
      this.username = data.username ?? "";
    } finally {
      if (revision === this.revision) this.loading = false;
    }
  }
  changeScope(scope: string) {
    this.scope = scope;
    return this.refresh();
  }
  async run(work: () => Promise<void>, message: string, reload = true, working = "refresh") {
    this.busy = true;
    this.working = working;
    this.error = "";
    this.notice = "";
    try {
      await work();
      if (reload) await this.load();
      this.notice = message;
    } catch (error) {
      this.error = error instanceof Error ? error.message : String(error);
    } finally {
      this.busy = false;
      this.working = "";
    }
  }
  save(
    command: { kind: string; key?: string; name?: string; [key: string]: unknown },
    message = "",
  ) {
    return this.run(
      () => action(command),
      message,
      true,
      `${command.kind}:${command.key ?? command.name ?? ""}`,
    );
  }
  refresh(message = "") {
    return this.run(() => this.load(), message, false);
  }
}
