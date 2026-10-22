import { setSession } from "../session.svelte.ts";
import { action, diagnostics, type Explained, type Snapshot, snapshot } from "./api.ts";

type Command = { kind: string; key?: string; name?: string; [key: string]: unknown };

export class MachineControls {
  data = $state<Snapshot | null>(null);
  scope = $state("");
  settingUp = $state(false);
  busy = $state(false);
  loading = $state(false);
  checking = $state(false);
  pending = $state<string[]>([]);
  private revision = 0;
  private checkRevision = 0;
  private dataScope = "";
  private editRevision = 0;
  private edits = new Map<string, number>();
  error = $state("");
  notice = $state("");
  url = $state("");
  username = $state("");
  password = $state("");

  saving(key: string) {
    return this.pending.includes(key);
  }
  async load(check = true) {
    const revision = ++this.revision;
    const scope = this.scope;
    const edits = this.editRevision;
    this.loading = true;
    try {
      const data = await snapshot(scope || null);
      if (revision !== this.revision || scope !== this.scope) return;
      // A refresh may overlap independent saves. Keep both pending edits
      // and saves completed since this snapshot started, on the same object
      // so their responses still reconcile against the visible panel.
      if (this.data && this.dataScope === scope) {
        const current = this.data;
        data.settings = data.settings.map((setting) =>
          this.saving(setting.key) || (this.edits.get(setting.key) ?? 0) > edits
            ? (current.settings.find((item) => item.key === setting.key) ?? setting)
            : setting,
        );
        if (this.saving("service") || (this.edits.get("service") ?? 0) > edits)
          data.service = current.service;
        Object.assign(current, data);
      } else this.data = data;
      this.dataScope = scope;
      setSession(data.me);
      this.url = data.hub ?? "";
      this.username = data.username ?? "";
      if (check) void this.check();
    } finally {
      if (revision === this.revision) this.loading = false;
    }
  }
  async check() {
    const revision = ++this.checkRevision;
    const scope = this.scope;
    const data = this.data;
    this.checking = true;
    try {
      const result = await diagnostics(scope || null);
      if (revision === this.checkRevision && scope === this.scope && data === this.data && data)
        Object.assign(data, result);
    } catch (error) {
      if (revision === this.checkRevision) this.error = this.message(error);
    } finally {
      if (revision === this.checkRevision) this.checking = false;
    }
  }
  private message(error: unknown) {
    return error instanceof Error ? error.message : String(error);
  }
  changeScope(scope: string) {
    this.scope = scope;
    this.error = "";
    this.notice = "";
    return this.refresh();
  }
  async run(work: () => Promise<unknown>, message: string, reload = true, working = "refresh") {
    this.pending = [...this.pending, working];
    this.busy = true;
    this.error = "";
    this.notice = "";
    try {
      await work();
      if (reload) await this.load(false);
      this.notice = message;
    } catch (error) {
      this.error = this.message(error);
    } finally {
      this.pending = this.pending.filter((key) => key !== working);
      this.busy = this.pending.length > 0;
    }
  }
  async save(command: Command, message = "", optimisticValue?: Explained["value"]) {
    const scope = this.scope;
    const data = this.data;
    const key = command.key ?? `${command.kind}:${command.name ?? ""}`;
    if (this.saving(key)) return;
    const index = data?.settings.findIndex((s) => s.key === command.key) ?? -1;
    const old = index >= 0 && data ? data.settings[index] : undefined;
    if (command.key) this.edits.set(command.key, ++this.editRevision);
    let optimistic: Explained | undefined;
    if (old && command.kind === "set") {
      let value = command.value as Explained["value"];
      if (optimisticValue !== undefined) value = optimisticValue;
      else if (command.key === "agents.priority")
        value = /^[-+]?\d+$/.test(String(value)) ? Number(value) : value;
      else if (typeof old.value === "boolean") value = value === "true";
      else if (typeof old.value === "number") value = Number(value);
      else if (Array.isArray(old.value)) {
        try {
          value = JSON.parse(String(value));
        } catch {
          value = old.value;
        }
      }
      optimistic = { ...old, value, source: { kind: scope ? "group" : "file", name: scope } };
    } else if (old && command.kind === "unset" && old.reset) {
      optimistic = { ...old.reset, reset: old.reset };
    }
    if (data && optimistic) data.settings[index] = optimistic;
    this.pending = [...this.pending, key];
    this.busy = true;
    this.error = "";
    this.notice = "";
    try {
      const result = await action(command);
      if (command.key) this.edits.set(command.key, ++this.editRevision);
      if (scope === this.scope && data === this.data && data) {
        if (result && index >= 0) data.settings[index] = result;
        if (command.kind === "secret" && command.name) {
          data.secrets = data.secrets.filter((name) => name !== command.name);
          if (command.value) data.secrets.push(command.name);
        }
        this.notice = message;
      }
      if (
        command.kind === "image" ||
        command.kind === "secret" ||
        /^(agents\.(isolation|image|workspace)|agents\.[^.]+\.(command|container\..+))$/.test(
          command.key ?? "",
        )
      )
        void this.check();
    } catch (error) {
      if (scope === this.scope && data === this.data) {
        if (data && old) data.settings[index] = old;
        this.error = this.message(error);
      }
    } finally {
      this.pending = this.pending.filter((item) => item !== key);
      this.busy = this.pending.length > 0;
    }
  }
  async turn(enabled: boolean) {
    const data = this.data;
    if (!data || this.saving("service")) return;
    const old = data.service.running;
    const setting = data.settings.find((s) => s.key === "agents.enabled");
    const index = setting ? data.settings.indexOf(setting) : -1;
    const enableSetting =
      enabled && setting?.value === false && setting.source.kind !== "environment";
    this.edits.set("service", ++this.editRevision);
    data.service.running = enabled;
    if (setting && enableSetting)
      data.settings[index] = { ...setting, value: true, source: { kind: "file" } };
    await this.run(
      async () => {
        try {
          if (enableSetting) {
            const result = await action({
              kind: "set",
              key: "agents.enabled",
              value: "true",
              group: null,
            });
            if (result) data.settings[index] = result;
          }
          await action({ kind: "service", enabled });
          this.edits.set("service", ++this.editRevision);
        } catch (error) {
          data.service.running = old;
          if (setting) data.settings[index] = setting;
          throw error;
        } finally {
          void this.check();
        }
      },
      "",
      false,
      "service",
    );
  }
  refresh(message = "") {
    return this.run(() => this.load(), message, false);
  }
}
