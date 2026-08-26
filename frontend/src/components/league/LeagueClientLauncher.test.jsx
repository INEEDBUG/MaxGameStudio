import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueClientLauncher from "./LeagueClientLauncher";
import { fetchLeagueClientInstallations, launchLeagueClient } from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  fetchLeagueClientInstallations: vi.fn(),
  launchLeagueClient: vi.fn(),
}));

describe("LeagueClientLauncher", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeagueClientInstallations.mockResolvedValue({
      installations: [{ kind: "tcls", label: "腾讯 TCLS", path: "D:\\League\\Client.exe" }],
    });
    launchLeagueClient.mockResolvedValue({ started: true, label: "腾讯 TCLS" });
  });

  it("launches only after the user clicks a detected entry", async () => {
    render(<LeagueClientLauncher />);
    expect(launchLeagueClient).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByText("腾讯 TCLS"));
    await waitFor(() => expect(launchLeagueClient).toHaveBeenCalledWith("tcls"));
    expect(await screen.findByText("已启动 腾讯 TCLS")).toBeTruthy();
  });
});
