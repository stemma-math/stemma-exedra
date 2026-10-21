import { action, type Snapshot, snapshot } from "./api.ts";

export class MachineControls {
  data = $state<Snapshot | null>(null);
  scope = $state("");
  settingUp = $state(false);
  busy = $state(false);
  error = $state("");
  notice = $state("");
  url = $state("");
  username = $state("");
  password = $state("");
  async load() {
    this.data = await snapshot(this.scope || null);
    this.url = this.data.hub ?? "";
    this.username = this.data.username ?? "";
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
    } finally {
      this.busy = false;
    }
  }
  save(command: object, message = "Saved.") {
    return this.run(() => action(command), message);
  }
  refresh(message = "") {
    return this.run(() => this.load(), message, false);
  }
}
