import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueMatchReplayActions from "./LeagueMatchReplayActions";
import { downloadLeagueReplay, fetchLeagueReplay, watchLeagueReplay } from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  downloadLeagueReplay: vi.fn(),
  fetchLeagueReplay: vi.fn(),
  watchLeagueReplay: vi.fn(),
}));

describe("LeagueMatchReplayActions", () => {
  beforeEach(() => vi.clearAllMocks());

  it("downloads a replay with the selected match metadata", async () => {
    const match = { game_id: 42, queue_id: 420, game_type: "MATCHED_GAME", played_at: 1000, duration_seconds: 60 };
    fetchLeagueReplay.mockResolvedValue({ enabled: true, metadata: { state: "download", downloadProgress: 0 } });
    downloadLeagueReplay.mockResolvedValue({ state: "downloading" });
    render(<LeagueMatchReplayActions match={match} />);

    fireEvent.click(await screen.findByRole("button", { name: "下载回放 42" }));
    await waitFor(() => expect(downloadLeagueReplay).toHaveBeenCalledWith(42, match));
  });

  it("plays an already downloaded replay", async () => {
    fetchLeagueReplay.mockResolvedValue({ enabled: true, metadata: { state: "watch", downloadProgress: 100 } });
    watchLeagueReplay.mockResolvedValue({ state: "watching" });
    render(<LeagueMatchReplayActions match={{ game_id: 99 }} />);

    fireEvent.click(await screen.findByRole("button", { name: "播放回放 99" }));
    await waitFor(() => expect(watchLeagueReplay).toHaveBeenCalledWith(99));
  });
});
