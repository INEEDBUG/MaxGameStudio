import { afterEach, describe, expect, test, vi } from "vitest";


describe("desktop backend asset URLs", () => {
  afterEach(() => {
    delete window.__TAURI_INTERNALS__;
    vi.resetModules();
  });

  test("uses the Vite proxy in browser mode", async () => {
    delete window.__TAURI_INTERNALS__;
    vi.resetModules();
    const { getDemoRadarMapUrl, getDemoUtilityMaskUrl, getLeagueClientAssetUrl } = await import("./api.js");

    expect(getDemoRadarMapUrl("de_mirage")).toBe("/api/demo/radar-map/de_mirage");
    expect(getDemoRadarMapUrl("de_nuke", "lower")).toBe("/api/demo/radar-map/de_nuke?layer=lower");
    expect(getDemoUtilityMaskUrl("de_mirage")).toBe("/api/demo/utility-mask/de_mirage");
    expect(getDemoUtilityMaskUrl("de_nuke", "lower")).toBe("/api/demo/utility-mask/de_nuke?layer=lower");
    expect(getLeagueClientAssetUrl("/lol-game-data/assets/map icon.png")).toBe("/api/league-lab/assets/client?path=%2Flol-game-data%2Fassets%2Fmap%20icon.png");
  });

  test("targets the bundled backend in Tauri mode", async () => {
    window.__TAURI_INTERNALS__ = {};
    vi.resetModules();
    const { getDemoRadarMapUrl, getDemoUtilityMaskUrl, getLeagueClientAssetUrl } = await import("./api.js");

    expect(getDemoRadarMapUrl("de_mirage")).toBe("http://127.0.0.1:19871/api/demo/radar-map/de_mirage");
    expect(getDemoUtilityMaskUrl("de_mirage")).toBe("http://127.0.0.1:19871/api/demo/utility-mask/de_mirage");
    expect(getLeagueClientAssetUrl("/lol-game-data/assets/map icon.png")).toBe("http://127.0.0.1:19871/api/league-lab/assets/client?path=%2Flol-game-data%2Fassets%2Fmap%20icon.png");
  });

  test("adds the ephemeral desktop token to browser-owned resource URLs", async () => {
    window.__TAURI_INTERNALS__ = {};
    vi.resetModules();
    const {
      getDemoRadarMapUrl,
      getLiteCutAssetStreamUrl,
      getLeagueChampionIconUrl,
      getLeagueClientAssetUrl,
      getLeagueItemIconUrl,
      getLeaguePerkIconUrl,
      getLeagueProfileIconUrl,
      setDesktopSessionToken,
    } = await import("./api.js");

    setDesktopSessionToken("session-123");

    expect(getDemoRadarMapUrl("de_nuke", "lower")).toBe(
      "http://127.0.0.1:19871/api/demo/radar-map/de_nuke?layer=lower&_session=session-123",
    );
    expect(getLiteCutAssetStreamUrl(7, "ready")).toBe(
      "http://127.0.0.1:19871/api/lite-cut/assets/7/stream?preview=ready&_session=session-123",
    );
    expect(getLeagueChampionIconUrl(22)).toBe(
      "http://127.0.0.1:19871/api/league-lab/assets/champions/22.png?_session=session-123",
    );
    expect(getLeagueClientAssetUrl("/lol-game-data/assets/map icon.png")).toBe(
      "http://127.0.0.1:19871/api/league-lab/assets/client?path=%2Flol-game-data%2Fassets%2Fmap%20icon.png&_session=session-123",
    );
    expect(getLeagueItemIconUrl(3089)).toBe(
      "http://127.0.0.1:19871/api/league-lab/assets/items/3089.png?_session=session-123",
    );
    expect(getLeaguePerkIconUrl(8010)).toBe(
      "http://127.0.0.1:19871/api/league-lab/assets/perks/8010.png?_session=session-123",
    );
    expect(getLeagueProfileIconUrl(29)).toBe(
      "http://127.0.0.1:19871/api/league-lab/assets/profile-icons/29.jpg?_session=session-123",
    );
  });

  test("adds the ephemeral desktop token to axios headers", async () => {
    window.__TAURI_INTERNALS__ = {};
    vi.resetModules();
    const { default: API, setDesktopSessionToken } = await import("./api.js");
    setDesktopSessionToken("session-123");
    let requestConfig;

    await API.get("/config", {
      adapter: async (config) => {
        requestConfig = config;
        return {
          data: {},
          status: 200,
          statusText: "OK",
          headers: {},
          config,
          request: {},
        };
      },
    });

    expect(requestConfig.headers.get("X-CS2-Insight-Token")).toBe("session-123");
  });
});
