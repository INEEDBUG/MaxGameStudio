import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueMasteryCatalog from "./LeagueMasteryCatalog";
import { fetchLeaguePlayerMastery } from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({ fetchLeaguePlayerMastery: vi.fn() }));

describe("LeagueMasteryCatalog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeaguePlayerMastery.mockResolvedValue({
      mastery: [
        { championId: 1, championName: "Annie", championPoints: 1000, championLevel: 7 },
        { championId: 2, championName: "Olaf", championPoints: 900, championLevel: 6 },
      ],
    });
  });

  it("loads and searches the full mastery catalog on demand", async () => {
    render(<LeagueMasteryCatalog puuid="player" initialRows={[{ championId: 1, championName: "Annie" }]} />);
    fireEvent.click(screen.getByRole("button", { name: /查看全部/ }));
    await waitFor(() => expect(fetchLeaguePlayerMastery).toHaveBeenCalledWith("player"));
    fireEvent.change(screen.getByPlaceholderText("搜索英雄"), { target: { value: "Olaf" } });
    expect(screen.getByText("Olaf")).toBeTruthy();
    expect(screen.queryByText("Annie")).toBeNull();
  });
});
