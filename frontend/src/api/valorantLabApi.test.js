import { beforeEach, describe, expect, it, vi } from "vitest";
import API from "./api.js";
import {
  VALORANT_LAB_API_CONTRACT,
  applyValorantStretch,
  decodeValorantCrosshair,
  encodeValorantCrosshair,
  fetchValorantCrosshair,
  fetchValorantDisplayStatus,
  restoreValorantStretchCfg,
  saveValorantCrosshair,
  unlockValorantStretchCfg,
} from "./valorantLabApi.js";

const PROFILES = {
  P: { color: "red" },
  A: { color: "cyan" },
  S: { color: "green" },
};

describe("Valorant Lab crosshair API contract", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("declares strict encode/decode endpoints under the shared API base", () => {
    expect(VALORANT_LAB_API_CONTRACT.encodeCrosshair).toEqual({ method: "POST", path: "/valorant-lab/crosshair/encode" });
    expect(VALORANT_LAB_API_CONTRACT.decodeCrosshair).toEqual({ method: "POST", path: "/valorant-lab/crosshair/decode" });
  });

  it("declares CFG lock actions and forwards lock_cfg with the stretch request", async () => {
    expect(VALORANT_LAB_API_CONTRACT.unlockStretchCfg).toEqual({ method: "POST", path: "/valorant-lab/stretch/cfg/unlock" });
    expect(VALORANT_LAB_API_CONTRACT.restoreStretchCfg).toEqual({ method: "POST", path: "/valorant-lab/stretch/cfg/restore" });
    vi.spyOn(API, "get").mockResolvedValue({ data: { overall: "ready", cfg_status: { status: "locked" } } });
    vi.spyOn(API, "post")
      .mockResolvedValueOnce({ data: { cfg_status: { status: "locked" } } })
      .mockResolvedValueOnce({ data: { cfg_status: { status: "unlocked" } } })
      .mockResolvedValueOnce({ data: { cfg_status: { status: "restored" } } });

    await expect(fetchValorantDisplayStatus()).resolves.toMatchObject({ cfg_status: { status: "locked" } });
    await expect(applyValorantStretch({ width: 1568, height: 1080, lock_cfg: true })).resolves.toMatchObject({ cfg_status: { status: "locked" } });
    expect(API.post).toHaveBeenNthCalledWith(1, "/valorant-lab/stretch/apply", expect.objectContaining({ lock_cfg: true }));
    await expect(unlockValorantStretchCfg()).resolves.toMatchObject({ cfg_status: { status: "unlocked" } });
    expect(API.post).toHaveBeenNthCalledWith(2, "/valorant-lab/stretch/cfg/unlock");
    await expect(restoreValorantStretchCfg()).resolves.toMatchObject({ cfg_status: { status: "restored" } });
    expect(API.post).toHaveBeenNthCalledWith(3, "/valorant-lab/stretch/cfg/restore");
  });

  it("keeps backend code responses instead of reducing them to UI profiles", async () => {
    const code = "0;P;c;7;future;keep-me";
    vi.spyOn(API, "get").mockResolvedValue({ data: { profiles: PROFILES, code } });
    vi.spyOn(API, "post")
      .mockResolvedValueOnce({ data: { code: "encoded-by-backend", format: "valorant-native-v0" } })
      .mockResolvedValueOnce({ data: { code, profiles: PROFILES, format: "valorant-native-v0" } });
    vi.spyOn(API, "put").mockResolvedValue({ data: { code: "saved-by-backend", profiles: PROFILES, saved: true } });

    await expect(fetchValorantCrosshair()).resolves.toMatchObject({ code, profiles: expect.objectContaining({ P: expect.any(Object) }) });
    await expect(encodeValorantCrosshair(PROFILES)).resolves.toMatchObject({ code: "encoded-by-backend" });
    expect(API.post).toHaveBeenNthCalledWith(1, "/valorant-lab/crosshair/encode", expect.objectContaining({ profiles: expect.any(Object) }));
    await expect(decodeValorantCrosshair(code)).resolves.toMatchObject({ code });
    expect(API.post).toHaveBeenNthCalledWith(2, "/valorant-lab/crosshair/decode", { code });
    await expect(saveValorantCrosshair(PROFILES)).resolves.toMatchObject({ code: "saved-by-backend" });
    expect(API.put).toHaveBeenCalledWith("/valorant-lab/crosshair", { profiles: expect.any(Object) });
  });
});
