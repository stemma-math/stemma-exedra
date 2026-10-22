import { beforeEach, describe, expect, it, vi } from "vitest";
import { MachineControls } from "./machine.svelte.ts";
import { action, diagnostics, snapshot, type Explained, type Snapshot } from "./api.ts";
vi.mock("./api.ts", () => ({ snapshot: vi.fn(), action: vi.fn(), diagnostics: vi.fn() }));
const setting = (key = "agents.concurrency", value: Explained["value"] = 1): Explained => ({
  key,
  value,
  source: { kind: "default" },
});
const data = (username: string): Snapshot => ({
  username,
  hub: "https://hub.test",
  me: null,
  hub_error: null,
  settings: [setting(), setting("agents.priority", "normal")],
  secrets: [],
  checks: [],
  stemma_image: "stemma-agents:0.1.1",
  signed_in: [],
  service: { installed: false, running: false, connected: false, signed_out: false, logs: "" },
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(diagnostics).mockResolvedValue({
    checks: [],
    secrets: [],
    signed_in: [],
    service: data("").service,
  });
});
describe("machine controls", () => {
  it("shows loading immediately and ignores an older scope's late snapshot", async () => {
    const controls = new MachineControls();
    const first = deferred<Snapshot>();
    vi.mocked(snapshot).mockReturnValueOnce(first.promise);
    const loading = controls.load();
    expect(controls.loading).toBe(true);
    controls.scope = "algebra";
    vi.mocked(snapshot).mockResolvedValueOnce(data("new scope"));
    await controls.load();
    first.resolve(data("old scope"));
    await loading;
    expect(controls.data?.username).toBe("new scope");
    expect(controls.loading).toBe(false);
  });
  it("clears loading and displays an action's actual error", async () => {
    const controls = new MachineControls();
    vi.mocked(snapshot).mockRejectedValueOnce(
      new Error("Could not reach https://hub.test: connection refused"),
    );
    await controls.refresh();
    expect(controls.loading).toBe(false);
    expect(controls.busy).toBe(false);
    expect(controls.error).toBe("Could not reach https://hub.test: connection refused");
  });
  it("updates immediately, permits independent saves, and reconciles without a snapshot or doctor", async () => {
    const controls = new MachineControls();
    controls.data = data("Ada");
    const first = deferred<Explained>();
    vi.mocked(action)
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce({ ...setting("agents.priority", "high"), source: { kind: "file" } });
    const save = controls.save({ kind: "set", key: "agents.concurrency", value: "3" });
    expect(controls.data.settings[0].value).toBe(3);
    expect(controls.saving("agents.concurrency")).toBe(true);
    expect(controls.saving("agents.priority")).toBe(false);
    await controls.save({ kind: "set", key: "agents.priority", value: "high" });
    expect(controls.data.settings[1].value).toBe("high");
    first.resolve({ ...setting("agents.concurrency", 3), source: { kind: "file" } });
    await save;
    expect(controls.data.settings[0].source.kind).toBe("file");
    expect(snapshot).not.toHaveBeenCalled();
    expect(diagnostics).not.toHaveBeenCalled();
  });
  it("resets immediately and restores only the failed setting", async () => {
    const controls = new MachineControls();
    controls.scope = "algebra";
    controls.data = data("Ada");
    controls.data.settings[0] = {
      ...setting("agents.concurrency", 4),
      source: { kind: "group" },
      reset: setting("agents.concurrency", 2),
    };
    const pending = deferred<Explained>();
    vi.mocked(action).mockReturnValueOnce(pending.promise);
    const reset = controls.save({ kind: "unset", key: "agents.concurrency", group: "algebra" });
    expect(controls.data.settings[0].value).toBe(2);
    controls.data.settings[1].value = "high";
    pending.reject(new Error("Could not save"));
    await reset;
    expect(controls.data.settings[0].value).toBe(4);
    expect(controls.data.settings[1].value).toBe("high");
    expect(controls.error).toBe("Could not save");
  });
  it("does not wait for diagnostics after an isolation save or let an older check overwrite it", async () => {
    const controls = new MachineControls();
    controls.data = data("Ada");
    controls.data.settings.push(setting("agents.isolation", "none"));
    const old = deferred<Awaited<ReturnType<typeof diagnostics>>>();
    vi.mocked(diagnostics)
      .mockReturnValueOnce(old.promise)
      .mockResolvedValueOnce({
        service: data("").service,
        secrets: ["NEW"],
        signed_in: [],
        checks: [],
      });
    const checking = controls.check();
    vi.mocked(action).mockResolvedValueOnce(setting("agents.isolation", "docker"));
    await controls.save({ kind: "set", key: "agents.isolation", value: "docker" });
    expect(controls.saving("agents.isolation")).toBe(false);
    old.resolve({ service: data("").service, secrets: ["OLD"], signed_in: [], checks: [] });
    await checking;
    expect(controls.data.secrets).toEqual(["NEW"]);
  });
});

it("keeps an overlapping refresh from losing a pending save or its returned source", async () => {
  const controls = new MachineControls();
  controls.data = data("Ada");
  const saving = deferred<Explained>();
  const refreshing = deferred<Snapshot>();
  vi.mocked(action).mockReturnValueOnce(saving.promise);
  vi.mocked(snapshot).mockReturnValueOnce(refreshing.promise);
  const refresh = controls.refresh();
  const save = controls.save({ kind: "set", key: "agents.concurrency", value: "3" });
  refreshing.resolve(data("Ada"));
  await refresh;
  expect(controls.data.settings[0].value).toBe(3);
  saving.resolve({ ...setting("agents.concurrency", 3), source: { kind: "file" } });
  await save;
  expect(controls.data.settings[0].source.kind).toBe("file");
});
