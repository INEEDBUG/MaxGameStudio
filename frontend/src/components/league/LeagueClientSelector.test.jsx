import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueClientSelector from "./LeagueClientSelector";
import { fetchLeagueClients, selectLeagueClient } from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  fetchLeagueClients: vi.fn(),
  selectLeagueClient: vi.fn(),
}));

describe("LeagueClientSelector", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeagueClients.mockResolvedValue({
      selected_pid: 101,
      clients: [
        { pid: 101, game_name: "Alpha", tag_line: "CN1", phase: "Lobby", selected: true },
        { pid: 202, game_name: "Beta", tag_line: "CN2", phase: "ChampSelect", selected: false },
      ],
    });
    selectLeagueClient.mockResolvedValue({ connected: true });
  });

  it("switches the exact detected client", async () => {
    const onSelected = vi.fn();
    render(<LeagueClientSelector onSelected={onSelected} />);
    fireEvent.click(await screen.findByText("Beta#CN2"));

    await waitFor(() => expect(selectLeagueClient).toHaveBeenCalledWith(202));
    expect(onSelected).toHaveBeenCalledTimes(1);
  });

  it("masks Riot IDs in streamer mode", async () => {
    render(<LeagueClientSelector streamerMode />);
    expect(await screen.findByText("League 客户端 · PID 101")).toBeTruthy();
    expect(screen.queryByText("Alpha#CN1")).toBeNull();
  });
});
