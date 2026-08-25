import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueSettingsTransfer, { buildLeagueSettingsExport, parseLeagueSettingsImport } from "./LeagueSettingsTransfer";

describe("League settings transfer", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("exports settings without credential-like fields", () => {
    expect(buildLeagueSettingsExport({ automation_enabled: true, token: "secret", nested: { api_key: "secret", value: 2 } }, "2026-08-15T00:00:00.000Z")).toEqual({
      format: "max-game-studio/league-settings",
      schema_version: 1,
      exported_at: "2026-08-15T00:00:00.000Z",
      settings: { automation_enabled: true, nested: { value: 2 } },
    });
  });

  it("sanitizes a compatible settings envelope", () => {
    const imported = parseLeagueSettingsImport(JSON.stringify({ format: "cs2-ultimate-insight-studio/league-settings", schema_version: 1, settings: { automation_enabled: true, auto_accept_enabled: true, auto_select_enabled: true, auto_honor_enabled: true, toolkit_account_actions_enabled: true, mini_enabled: false, auto_select_profiles: { aram: { pick: { enabled: true, bench_handle_trade_enabled: true }, ban: { enabled: true } } }, unknown_field: "ignored", access_token: "removed" } }), { automation_enabled: false, auto_accept_enabled: false, auto_select_enabled: false, auto_honor_enabled: false, toolkit_account_actions_enabled: false, mini_enabled: true, auto_select_profiles: {} });
    expect(imported).toMatchObject({ automation_enabled: false, auto_accept_enabled: false, auto_select_enabled: false, auto_honor_enabled: false, toolkit_account_actions_enabled: false, mini_enabled: false, auto_select_profiles: { aram: { pick: { enabled: false, bench_handle_trade_enabled: false }, ban: { enabled: false } } } });
    expect(imported).not.toHaveProperty("unknown_field");
    expect(imported).not.toHaveProperty("access_token");
  });

  it("imports LeagueAkari database exports from data[]", () => {
    const imported = parseLeagueSettingsImport(JSON.stringify({
      databaseVersion: 15,
      type: "league-akari-settings",
      data: [
        { key: "window-manager-main/aux-window/pinned", value: false },
        { key: "window-manager-main/ongoing-game-window/pinned", value: false },
        { key: "window-manager-main/cd-timer-window/pinned", value: true },
        { key: "unknown/setting", value: "ignored" },
      ],
    }), {
      mini_pinned: true,
      ongoing_pinned: true,
      cooldown_pinned: false,
    });
    expect(imported).toMatchObject({
      mini_pinned: false,
      ongoing_pinned: false,
      cooldown_pinned: true,
    });
  });

  it("rejects a future LeagueAkari database export", () => {
    expect(() => parseLeagueSettingsImport(JSON.stringify({
      databaseVersion: 99,
      type: "league-akari-settings",
      data: [],
    }), { mini_pinned: true })).toThrow("更新版本");
  });

  it("rejects invalid and future-version files", () => {
    expect(() => parseLeagueSettingsImport("not-json", { mini_enabled: true })).toThrow("有效的 JSON");
    expect(() => parseLeagueSettingsImport(JSON.stringify({ format: "cs2-ultimate-insight-studio/league-settings", schema_version: 99, settings: { mini_enabled: false } }), { mini_enabled: true })).toThrow("更新版本");
  });

  it("imports a selected JSON file after confirmation", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const onImport = vi.fn().mockResolvedValue(undefined);
    render(<LeagueSettingsTransfer settings={{ automation_enabled: false, mini_enabled: true }} onImport={onImport}/>);
    const file = new File([JSON.stringify({ mini_enabled: false, automation_enabled: true })], "settings.json", { type: "application/json" });
    Object.defineProperty(file, "text", { value: vi.fn().mockResolvedValue(JSON.stringify({ mini_enabled: false, automation_enabled: true })) });
    fireEvent.change(screen.getByLabelText("选择 League 设置文件"), { target: { files: [file] } });
    await waitFor(() => expect(onImport).toHaveBeenCalledWith(expect.objectContaining({ mini_enabled: false, automation_enabled: false })));
    expect(screen.getByRole("status").textContent).toContain("仍为关闭状态");
  });
});
