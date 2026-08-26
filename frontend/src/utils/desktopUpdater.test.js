import { beforeEach, describe, expect, it, vi } from "vitest";

const updaterMocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: updaterMocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: updaterMocks.relaunch }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: updaterMocks.invoke }));

import { createDesktopUpdateCheck, normalizeUpdateMode } from "./desktopUpdater.js";

function makeUpdate(overrides = {}) {
  return {
    version: "2.5.13",
    body: "修复自动更新安装",
    rawJson: { update_mode: "normal" },
    download: vi.fn(async (onEvent) => {
      onEvent({ event: "Started", data: { contentLength: 100 } });
      onEvent({ event: "Progress", data: { chunkLength: 40 } });
      onEvent({ event: "Progress", data: { chunkLength: 60 } });
      onEvent({ event: "Finished", data: {} });
    }),
    install: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  updaterMocks.relaunch.mockResolvedValue(undefined);
  updaterMocks.invoke.mockResolvedValue(undefined);
});

describe("normalizeUpdateMode", () => {
  it("defaults to normal", () => {
    expect(normalizeUpdateMode(undefined)).toBe("normal");
    expect(normalizeUpdateMode("")).toBe("normal");
    expect(normalizeUpdateMode("NORMAL")).toBe("normal");
    expect(normalizeUpdateMode("other")).toBe("normal");
  });

  it("accepts force", () => {
    expect(normalizeUpdateMode("force")).toBe("force");
    expect(normalizeUpdateMode(" Force ")).toBe("force");
  });
});

describe("createDesktopUpdateCheck", () => {
  it("automatically downloads, installs, and relaunches an available update", async () => {
    const update = makeUpdate();
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    await createDesktopUpdateCheck((state) => states.push(state)).start();

    expect(update.download).toHaveBeenCalledOnce();
    expect(updaterMocks.invoke).toHaveBeenCalledWith("persist_desktop_window_state");
    expect(update.install).toHaveBeenCalledOnce();
    expect(updaterMocks.relaunch).toHaveBeenCalledOnce();
    expect(states.map((state) => state.status)).toEqual([
      "checking",
      "available",
      "downloading",
      "downloading",
      "downloading",
      "installing",
    ]);
    expect(states.at(-1)).toMatchObject({ auto_install: true, latest_version: "2.5.13" });
  });

  it("never starts installation when the download fails", async () => {
    const update = makeUpdate({ download: vi.fn(async () => { throw new Error("network down"); }) });
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    await createDesktopUpdateCheck((state) => states.push(state)).start();

    expect(update.install).not.toHaveBeenCalled();
    expect(updaterMocks.relaunch).not.toHaveBeenCalled();
    expect(states.at(-1)).toMatchObject({
      status: "error",
      error_stage: "download",
      error: "network down",
    });
  });

  it("reports an installation launch failure without pretending the update completed", async () => {
    const update = makeUpdate({ install: vi.fn(async () => { throw new Error("installer blocked"); }) });
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    await createDesktopUpdateCheck((state) => states.push(state)).start();

    expect(update.download).toHaveBeenCalledOnce();
    expect(update.install).toHaveBeenCalledOnce();
    expect(updaterMocks.relaunch).not.toHaveBeenCalled();
    expect(states.at(-1)).toMatchObject({
      status: "error",
      error_stage: "install",
      error: "installer blocked",
    });
  });

  it("still supports an explicit confirmation flow when automatic install is disabled", async () => {
    const update = makeUpdate();
    updaterMocks.check.mockResolvedValue(update);
    const states = [];
    const controller = createDesktopUpdateCheck((state) => states.push(state), { autoInstall: false });

    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));
    expect(update.download).not.toHaveBeenCalled();

    controller.confirm();
    await run;

    expect(update.download).toHaveBeenCalledOnce();
    expect(update.install).toHaveBeenCalledOnce();
  });

  it("fails open when the update endpoint does not answer in time", async () => {
    vi.useFakeTimers();
    updaterMocks.check.mockImplementation(() => new Promise(() => {}));
    const states = [];
    const controller = createDesktopUpdateCheck((state) => states.push(state), {
      checkTimeoutMs: 1000,
    });

    const run = controller.start();
    await vi.advanceTimersByTimeAsync(1000);
    await run;

    expect(states.map((state) => state.status)).toEqual(["checking", "error"]);
    expect(states.at(-1)?.error).toContain("检查更新超时");
    vi.useRealTimers();
  });
});
