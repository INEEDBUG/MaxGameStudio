import { beforeEach, describe, expect, test, vi } from "vitest";
import {
  clearHandledLeagueSession,
  isLeagueSessionHandled,
  launchLeagueRuntimeCoordinated,
  leagueClientSessionId,
  markLeagueSessionHandled,
  readHandledLeagueSession,
} from "./leagueRuntimeLaunchCoordinator.js";

const { launch } = vi.hoisted(() => ({ launch: vi.fn() }));
vi.mock("../desktop/desktopBridge.js", () => ({
  isDesktopApp: true,
  desktopBridge: { launchLeagueRuntime: launch },
}));

describe("League runtime launch coordinator", () => {
  beforeEach(() => {
    window.localStorage.clear();
    launch.mockReset();
    launch.mockResolvedValue(undefined);
  });

  test("derives a non-sensitive client session key and supports window-only detection", () => {
    expect(leagueClientSessionId({ connected: true, client_pid: 1234 })).toBe("pid:1234");
    expect(leagueClientSessionId({ connected: false, client_window_detected: true })).toBe("window");
    expect(leagueClientSessionId({ connected: false, client_window_detected: false })).toBe("");
  });

  test("expires stale handled sessions and lets a wildcard suppress the restored host", () => {
    markLeagueSessionHandled("*", localStorage, 1_000);
    expect(isLeagueSessionHandled("pid:9", localStorage, 2_000)).toBe(true);
    expect(readHandledLeagueSession(localStorage, 1_000 + 12 * 60 * 60 * 1000 + 1)).toBeNull();
  });

  test("deduplicates concurrent launches and suppresses the same client session", async () => {
    let resolveLaunch;
    launch.mockReturnValueOnce(new Promise((resolve) => { resolveLaunch = resolve; }));
    const first = launchLeagueRuntimeCoordinated("memory", { sessionId: "pid:7" });
    const second = launchLeagueRuntimeCoordinated("memory", { sessionId: "pid:7", force: true });
    await Promise.resolve();
    expect(launch).toHaveBeenCalledTimes(1);
    resolveLaunch();
    await Promise.all([first, second]);
    await expect(launchLeagueRuntimeCoordinated("memory", { sessionId: "pid:7" })).resolves.toEqual({ launched: false, reason: "handled" });
  });

  test("returns an explicit in-flight result for a different mode", async () => {
    let resolveLaunch;
    launch.mockReturnValueOnce(new Promise((resolve) => { resolveLaunch = resolve; }));
    const first = launchLeagueRuntimeCoordinated("memory", { sessionId: "pid:10" });
    await Promise.resolve();
    await expect(launchLeagueRuntimeCoordinated("parallel", { sessionId: "pid:10", force: true }))
      .resolves.toEqual({ launched: false, reason: "in-flight", mode: "memory", administrator: false });
    resolveLaunch();
    await first;
  });

  test("clears the handled marker when native launch fails so an explicit retry remains possible", async () => {
    launch.mockRejectedValueOnce(new Error("spawn failed"));
    await expect(launchLeagueRuntimeCoordinated("parallel", { sessionId: "pid:8" })).rejects.toThrow("spawn failed");
    expect(readHandledLeagueSession()).toBeNull();
    clearHandledLeagueSession();
  });
});
