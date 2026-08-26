import { describe, expect, it } from "vitest";
import {
  DEFAULT_CROSSHAIR_PROFILES,
  VALORANT_RESOLUTION_PRESETS,
  isDisplayStatusReady,
  normalizeCrosshairProfiles,
  parseCrosshairCode,
  serializeCrosshairCode,
} from "./valorantLab.js";

describe("valorantLab helpers", () => {
  it("keeps the community resolution first", () => {
    expect(VALORANT_RESOLUTION_PRESETS[0]).toMatchObject({ id: "1568x1080", width: 1568, height: 1080 });
  });

  it("does not treat unknown display state as executable", () => {
    expect(isDisplayStatusReady(undefined)).toBe(false);
    expect(isDisplayStatusReady({ gpu: { status: "ready" }, monitor: { status: "ready" }, refreshRate: { status: "unknown" } })).toBe(false);
    expect(isDisplayStatusReady({ overall: "unknown", gpu: { status: "ready" }, monitor: { status: "ready" }, refreshRate: { status: "ready" } })).toBe(false);
    expect(isDisplayStatusReady({ overall: "ready", gpu: { status: "ready" }, monitor: { status: "ready" }, refreshRate: { status: "ready" } })).toBe(true);
  });

  it("round-trips the native VALORANT crosshair code and fills missing profiles", () => {
    const profiles = normalizeCrosshairProfiles({ P: { color: "red", innerLines: false } });
    const code = serializeCrosshairCode(profiles);
    const parsed = parseCrosshairCode(code);
    expect(code).toMatch(/^0;s;1;P;/);
    expect(code).toContain(";A;");
    expect(code).toContain(";S;");
    expect(parsed.P).toMatchObject({ color: "red", innerLines: false });
    expect(parsed.A).toEqual(DEFAULT_CROSSHAIR_PROFILES.A);
    expect(parsed.S).toMatchObject({ color: DEFAULT_CROSSHAIR_PROFILES.S.color, centerDot: DEFAULT_CROSSHAIR_PROFILES.S.centerDot, innerLines: false, outerLines: false });
    expect(serializeCrosshairCode(parsed)).toBe(code);
  });
});
