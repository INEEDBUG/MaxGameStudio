import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchLeagueLabStatus } from "../api/leagueLabApi";
import { getLeagueLabStatusSnapshot, subscribeLeagueLabStatus } from "./leagueLabStatusSubscription";

vi.mock("../api/leagueLabApi", () => ({
  fetchLeagueLabStatus: vi.fn(),
}));

describe("leagueLabStatusSubscription", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    fetchLeagueLabStatus.mockReset();
    fetchLeagueLabStatus.mockResolvedValue({ connected: true, phase: "Lobby" });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shares one 1.5 second poller across all desktop managers", async () => {
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = subscribeLeagueLabStatus(first);
    const unsubscribeSecond = subscribeLeagueLabStatus(second);

    await Promise.resolve();
    await Promise.resolve();
    expect(fetchLeagueLabStatus).toHaveBeenCalledTimes(1);
    expect(first).toHaveBeenCalledWith(expect.objectContaining({ phase: "Lobby" }));
    expect(second).toHaveBeenCalledWith(expect.objectContaining({ phase: "Lobby" }));

    await vi.advanceTimersByTimeAsync(1500);
    expect(fetchLeagueLabStatus).toHaveBeenCalledTimes(2);

    unsubscribeFirst();
    unsubscribeSecond();
  });

  it("ignores a response that arrives after the final subscriber leaves", async () => {
    let resolveRequest;
    fetchLeagueLabStatus.mockReturnValueOnce(new Promise((resolve) => { resolveRequest = resolve; }));
    const listener = vi.fn();
    const unsubscribe = subscribeLeagueLabStatus(listener);

    unsubscribe();
    resolveRequest({ connected: true, phase: "ChampSelect" });
    await Promise.resolve();
    await Promise.resolve();

    expect(listener).not.toHaveBeenCalled();
    expect(getLeagueLabStatusSnapshot()).toBeNull();
  });

  it("publishes a null snapshot after a polling failure instead of retaining stale state", async () => {
    const listener = vi.fn();
    const unsubscribe = subscribeLeagueLabStatus(listener);
    await Promise.resolve();
    await Promise.resolve();
    expect(listener).toHaveBeenLastCalledWith(expect.objectContaining({ phase: "Lobby" }));

    fetchLeagueLabStatus.mockRejectedValueOnce(new Error("offline"));
    await vi.advanceTimersByTimeAsync(1500);

    expect(listener).toHaveBeenLastCalledWith(null);
    expect(getLeagueLabStatusSnapshot()).toBeNull();
    unsubscribe();
  });
});
