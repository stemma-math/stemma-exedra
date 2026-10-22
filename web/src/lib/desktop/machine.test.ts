import { describe, expect, it, vi } from "vitest";
import { MachineControls } from "./machine.svelte.ts";
import { snapshot, type Snapshot } from "./api.ts";
vi.mock("./api.ts", () => ({ snapshot: vi.fn(), action: vi.fn() }));
const data = (username: string) => ({ username, hub: "https://hub.test" }) as Snapshot;

describe("machine loading", () => {
  it("shows loading immediately and ignores an older scope's late snapshot", async () => {
    const controls = new MachineControls();
    let resolve!: (value: Snapshot) => void;
    vi.mocked(snapshot).mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const first = controls.load();
    expect(controls.loading).toBe(true);
    controls.scope = "algebra";
    vi.mocked(snapshot).mockResolvedValueOnce(data("new scope"));
    await controls.load();
    resolve(data("old scope"));
    await first;
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
});
