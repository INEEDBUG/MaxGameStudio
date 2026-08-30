import { describe, expect, it } from "vitest";
import {
  SENSITIVITY_TRIAL_SCHEDULE,
  classifyFlickClick,
  cs2CursorGain,
  makeFlickTrialResult,
  makeTrackingTrialResult,
  pointDistance,
  sensitivityToCm360,
} from "./sensitivityLab";

describe("sensitivityLab", () => {
  it("covers flick and tracking at every candidate multiplier", () => {
    for (const multiplier of [0.8, 1, 1.2]) {
      const kinds = SENSITIVITY_TRIAL_SCHEDULE
        .filter((trial) => trial.multiplier === multiplier)
        .map((trial) => trial.kind);
      expect(kinds).toEqual(["flick", "tracking"]);
    }
  });

  it("calculates touch-trigger flick metrics", () => {
    const result = makeFlickTrialResult({
      multiplier: 1,
      durationMs: 15_000,
      reactions: [300, 500],
      efficiencies: [0.8, 1],
      overshoots: 1,
    });
    expect(result.hits).toBe(2);
    expect(result.targets).toBe(3);
    expect(result.average_reaction_ms).toBe(400);
    expect(result.path_efficiency).toBeCloseTo(0.9);
  });

  it("classifies click geometry without treating lateral error as sensitivity", () => {
    const base = { start: { x: 0, y: 0 }, target: { x: 100, y: 0 }, targetRadius: 10 };
    expect(classifyFlickClick({ ...base, cursor: { x: 75, y: 0 } }).direction).toBe("underflick");
    expect(classifyFlickClick({ ...base, cursor: { x: 125, y: 0 } }).direction).toBe("overflick");
    expect(classifyFlickClick({ ...base, cursor: { x: 100, y: 25 } }).direction).toBe("off_axis");
    expect(classifyFlickClick({ ...base, cursor: { x: 100, y: 0 } })).toMatchObject({ hit: true, direction: "hit" });
    expect(classifyFlickClick({ ...base, cursor: { x: 98, y: 0 }, overshot: true })).toMatchObject({ hit: true, direction: "overflick" });
  });

  it("returns aggregated click evidence for the backend", () => {
    const result = makeFlickTrialResult({
      multiplier: 1,
      durationMs: 15_000,
      reactions: [320, 400],
      efficiencies: [0.8, 0.5, 0.7],
      overshoots: 1,
      clicks: 3,
      misses: 1,
      underflicks: 1,
      overflicks: 0,
      offAxisMisses: 0,
      clickErrors: [0.02, 0.18, 0.04],
    });
    expect(result).toMatchObject({
      hits: 2,
      targets: 3,
      clicks: 3,
      misses: 1,
      underflicks: 1,
      off_axis_misses: 0,
    });
    expect(result.average_click_error_ratio).toBeCloseTo(0.08);
  });

  it("clamps tracking ratios and distance scoring", () => {
    const result = makeTrackingTrialResult({
      multiplier: 1.2,
      durationMs: 10_000,
      onTargetMs: 12_000,
      distanceSamples: [0.1, 0.3],
      overshoots: 2,
    });
    expect(result.on_target_ratio).toBe(1);
    expect(result.path_efficiency).toBeCloseTo(0.8);
    expect(pointDistance({ x: 0, y: 0 }, { x: 3, y: 4 })).toBe(5);
  });

  it("uses the configured CS2 sensitivity and m_yaw for arena gain", () => {
    expect(cs2CursorGain(0.35, 0.022, 1)).toBeCloseTo(0.35);
    expect(cs2CursorGain(0.35, 0.044, 1.2)).toBeCloseTo(0.84);
  });

  it("calculates cm/360 with the configured m_yaw", () => {
    expect(sensitivityToCm360(800, 1, 0.022)).toBeCloseTo(51.9545, 4);
    expect(sensitivityToCm360(800, 1, 0.044)).toBeCloseTo(25.9773, 4);
  });
});
