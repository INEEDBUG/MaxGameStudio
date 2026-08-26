import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueEncounteredGames from "./LeagueEncounteredGames";
import { deleteLeaguePlayerEncounter, fetchLeaguePlayerEncounters } from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  deleteLeaguePlayerEncounter: vi.fn(),
  fetchLeaguePlayerEncounters: vi.fn(),
}));

describe("LeagueEncounteredGames", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeaguePlayerEncounters.mockResolvedValue({
      total: 1,
      page: 1,
      page_size: 10,
      games: [{ game_id: 7, self_puuid: "self", game_mode: "CLASSIC", target: { champion_id: 2, kills: 3, deaths: 4, assists: 5 }, self: { champion_id: 1, kills: 9, deaths: 2, assists: 8 } }],
    });
    deleteLeaguePlayerEncounter.mockResolvedValue({ removed: true });
  });

  it("shows and removes a persisted shared match", async () => {
    render(<LeagueEncounteredGames puuid="target" selfPuuid="self" />);
    expect(await screen.findByText("共同对局（1）")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "移除共同对局 7" }));
    await waitFor(() => expect(deleteLeaguePlayerEncounter).toHaveBeenCalledWith("target", 7));
  });
});
