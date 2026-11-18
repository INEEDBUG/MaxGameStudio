import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, test, vi } from "vitest";

const { bridge } = vi.hoisted(() => ({
  bridge: { getSteamAccounts: vi.fn(), switchSteamAccount: vi.fn() },
}));

vi.mock("../desktop/desktopBridge.js", () => ({ desktopBridge: bridge }));
vi.mock("../i18n/localeStore.js", () => ({ useLocaleStore: (select) => select({ effectiveLocale: "zh" }) }));

import SteamAccountsPanel from "./SteamAccountsPanel.jsx";

const first = { steam_id: "76561197960265730", persona_name: "测试一", remembered: true };
const second = { steam_id: "76561197960265731", persona_name: "测试二", remembered: true };

function renderPanel() {
  return render(<MemoryRouter><SteamAccountsPanel /></MemoryRouter>);
}

describe("Steam account switching", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    bridge.getSteamAccounts.mockResolvedValue({
      installed: true, running: true, current_steam_id: first.steam_id, accounts: [first, second],
    });
  });

  test("renders an honest missing-Steam state", async () => {
    bridge.getSteamAccounts.mockResolvedValue({ installed: false, accounts: [] });
    renderPanel();
    expect(await screen.findByText("未检测到 Steam")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "切换账号" })).toBeNull();
  });

  test("one click sends the selected ID and relaunch alone does not show it as logged in", async () => {
    let finishSwitch;
    bridge.switchSteamAccount.mockReturnValue(new Promise((resolve) => { finishSwitch = resolve; }));
    renderPanel();
    const button = await screen.findByRole("button", { name: "切换账号" });
    expect(screen.getByText("当前登录")).toBeTruthy();
    fireEvent.click(button);
    fireEvent.click(button);
    expect(button.disabled).toBe(true);
    expect(bridge.switchSteamAccount).toHaveBeenCalledTimes(1);
    expect(bridge.switchSteamAccount).toHaveBeenCalledWith(second.steam_id);
    bridge.getSteamAccounts.mockResolvedValue({ installed: true, running: true, accounts: [first, second] });
    act(() => finishSwitch({ status: "waiting_login", steam_id: second.steam_id }));
    expect(await screen.findByText("Steam 已启动，请在 Steam 窗口查看登录状态。")).toBeTruthy();
    await waitFor(() => expect(screen.queryByText("当前登录")).toBeNull());
  });
});
