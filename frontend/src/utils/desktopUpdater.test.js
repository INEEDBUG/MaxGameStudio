import { beforeEach, describe, expect, it, vi } from "vitest";

const updaterMocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: updaterMocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: updaterMocks.relaunch }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: updaterMocks.invoke }));

import {
  createDesktopUpdateCheck,
  normalizeUpdateMode,
  normalizeUserReleaseNotes,
} from "./desktopUpdater.js";

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

describe("normalizeUserReleaseNotes", () => {
  it("keeps only non-empty plain-language entries", () => {
    expect(normalizeUserReleaseNotes({ fixed: [" 修复崩溃 ", ""], added: null })).toEqual({
      fixed: ["修复崩溃"],
      added: [],
      optimized: [],
    });
    expect(normalizeUserReleaseNotes({ fixed: [], added: [], optimized: [] })).toBeNull();
  });
});

describe("createDesktopUpdateCheck", () => {
  it("forwards categorized user release notes from the updater manifest", async () => {
    const update = makeUpdate({
      rawJson: {
        update_mode: "normal",
        user_release_notes: {
          fixed: ["修复启动崩溃"],
          added: ["新增快速预览"],
          optimized: ["降低内存占用"],
        },
      },
    });
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    const controller = createDesktopUpdateCheck((state) => states.push(state));
    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));
    controller.confirm();
    await run;

    expect(states.find((state) => state.status === "available")?.user_release_notes).toEqual({
      fixed: ["修复启动崩溃"],
      added: ["新增快速预览"],
      optimized: ["降低内存占用"],
    });
  });

  it("does not download until the user explicitly confirms, even with legacy auto-install options", async () => {
    const update = makeUpdate();
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    const controller = createDesktopUpdateCheck((state) => states.push(state), {
      autoInstall: true,
      autoInstallGraceMs: 0,
    });
    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));

    expect(update.download).not.toHaveBeenCalled();
    expect(states.at(-1)).toMatchObject({ auto_install: false, awaiting_choice: true });
    controller.confirm();
    await run;

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
    expect(states.at(-1)).toMatchObject({ auto_install: false, latest_version: "2.5.13" });
  });

  it("never starts installation when the download fails", async () => {
    const update = makeUpdate({ download: vi.fn(async () => { throw new Error("network down"); }) });
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    const controller = createDesktopUpdateCheck((state) => states.push(state));
    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));
    controller.confirm();
    await run;

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

    const controller = createDesktopUpdateCheck((state) => states.push(state));
    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));
    controller.confirm();
    await run;

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

  it("ignores the legacy grace window and records a deferred version", async () => {
    const update = makeUpdate();
    updaterMocks.check.mockResolvedValue(update);
    const states = [];
    const controller = createDesktopUpdateCheck((state) => states.push(state), {
      autoInstall: true,
      autoInstallGraceMs: 10,
    });

    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));
    expect(states.at(-1)).toMatchObject({ awaiting_choice: true, auto_install: false });

    controller.defer();
    await run;

    expect(update.download).not.toHaveBeenCalled();
    expect(update.install).not.toHaveBeenCalled();
    expect(update.close).toHaveBeenCalledOnce();
    expect(states.at(-1)).toMatchObject({ status: "cancelled", skipped_version: "2.5.13" });
  });

  it("does not download a version remembered as skipped", async () => {
    const update = makeUpdate();
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    await createDesktopUpdateCheck((state) => states.push(state), {
      skipVersion: "2.5.13",
      autoInstallGraceMs: 0,
    }).start();

    expect(update.close).toHaveBeenCalledOnce();
    expect(update.download).not.toHaveBeenCalled();
    expect(states.at(-1)).toMatchObject({ status: "skipped", skipped_version: "2.5.13" });
  });

  it("never lets a remembered skip bypass a force update, but still requires confirmation", async () => {
    const update = makeUpdate({ rawJson: { update_mode: "force" } });
    updaterMocks.check.mockResolvedValue(update);
    const states = [];

    const controller = createDesktopUpdateCheck((state) => states.push(state), {
      skipVersion: "2.5.13",
      autoInstallGraceMs: 10_000,
    });
    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));

    expect(update.download).not.toHaveBeenCalled();
    expect(states.some((state) => state.status === "skipped")).toBe(false);
    expect(states.find((state) => state.status === "available")).toMatchObject({
      update_mode: "force",
      awaiting_choice: true,
    });
    expect(controller.defer()).toBe(false);
    controller.confirm();
    await run;
    expect(update.download).toHaveBeenCalledOnce();
    expect(update.install).toHaveBeenCalledOnce();
  });

  it("allows an internal cancellation to release a force-update wait", async () => {
    const update = makeUpdate({ rawJson: { update_mode: "force" } });
    updaterMocks.check.mockResolvedValue(update);
    const states = [];
    const controller = createDesktopUpdateCheck((state) => states.push(state));

    const run = controller.start();
    await vi.waitFor(() => expect(states.at(-1)?.status).toBe("available"));
    expect(controller.defer()).toBe(false);
    expect(controller.cancel()).toBe(true);
    await run;

    expect(update.close).toHaveBeenCalledOnce();
    expect(update.download).not.toHaveBeenCalled();
    expect(states.at(-1)).toMatchObject({ status: "cancelled", update_mode: "force" });
  });

  it("keeps waiting indefinitely until confirm or defer", async () => {
    vi.useFakeTimers();
    const update = makeUpdate();
    updaterMocks.check.mockResolvedValue(update);
    const states = [];
    const controller = createDesktopUpdateCheck((state) => states.push(state), {
      autoInstallGraceMs: 1000,
    });

    const run = controller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(states.at(-1)).toMatchObject({ status: "available", awaiting_choice: true });

    await vi.advanceTimersByTimeAsync(10_000);
    expect(states.at(-1)).toMatchObject({ status: "available", awaiting_choice: true });
    expect(update.download).not.toHaveBeenCalled();
    expect(controller.defer()).toBe(true);
    await run;
    expect(update.close).toHaveBeenCalledOnce();
    vi.useRealTimers();
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
