export const SENSITIVITY_TRIAL_SCHEDULE = [
  { kind: "flick", multiplier: 0.8 },
  { kind: "tracking", multiplier: 0.8 },
  { kind: "flick", multiplier: 1.0 },
  { kind: "tracking", multiplier: 1.0 },
  { kind: "flick", multiplier: 1.2 },
  { kind: "tracking", multiplier: 1.2 },
];

export const DEFAULT_CS2_YAW = 0.022;

export function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

export function pointDistance(a, b) {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

export function classifyFlickClick({ start, target, cursor, targetRadius, overshot = false }) {
  const axisX = target.x - start.x;
  const axisY = target.y - start.y;
  const axisLength = Math.max(1, Math.hypot(axisX, axisY));
  const clickX = cursor.x - start.x;
  const clickY = cursor.y - start.y;
  const projection = (clickX * axisX + clickY * axisY) / (axisLength * axisLength);
  const lateralDistance = Math.abs(axisX * clickY - axisY * clickX) / axisLength;
  const targetDistance = pointDistance(cursor, target);
  const hit = targetDistance <= targetRadius;
  const projectionTolerance = targetRadius / axisLength;

  let direction = "hit";
  if (!hit) {
    if (lateralDistance > targetRadius) direction = "off_axis";
    else if (projection < 1 - projectionTolerance) direction = "underflick";
    else if (projection > 1 + projectionTolerance || overshot) direction = "overflick";
    else direction = "off_axis";
  } else if (overshot) {
    direction = "overflick";
  }

  return {
    hit,
    direction,
    errorRatio: clamp(targetDistance / axisLength, 0, 1),
  };
}

export function sensitivityToCm360(dpi, sensitivity, mYaw = DEFAULT_CS2_YAW) {
  return (360 * 2.54) / (Number(dpi) * Number(sensitivity) * Number(mYaw));
}

export function cs2CursorGain(currentSensitivity, mYaw, multiplier) {
  // Pointer Lock supplies the incoming mouse delta; apply the same relative
  // sensitivity/yaw gain used by the configured CS2 baseline and candidates.
  const relativeYaw = Number(mYaw) / DEFAULT_CS2_YAW;
  return clamp(Number(currentSensitivity) * relativeYaw * Number(multiplier), 0.08, 4);
}

export function makeFlickTrialResult({
  multiplier,
  durationMs,
  reactions,
  efficiencies,
  overshoots,
  clicks = null,
  misses = 0,
  underflicks = 0,
  overflicks = 0,
  offAxisMisses = 0,
  clickErrors = [],
}) {
  const hits = reactions.length;
  const clickCount = clicks == null ? hits + 1 : Math.max(0, clicks);
  return {
    kind: "flick",
    multiplier,
    duration_ms: Math.round(durationMs),
    hits,
    targets: clickCount,
    average_reaction_ms: hits ? reactions.reduce((sum, value) => sum + value, 0) / hits : 0,
    path_efficiency: efficiencies.length
      ? efficiencies.reduce((sum, value) => sum + value, 0) / efficiencies.length
      : 0,
    overshoots,
    on_target_ratio: 0,
    clicks: clickCount,
    misses,
    underflicks,
    overflicks,
    off_axis_misses: offAxisMisses,
    average_click_error_ratio: clickErrors.length
      ? clickErrors.reduce((sum, value) => sum + value, 0) / clickErrors.length
      : 0,
  };
}

export function makeTrackingTrialResult({ multiplier, durationMs, onTargetMs, distanceSamples, overshoots }) {
  const averageDistanceRatio = distanceSamples.length
    ? distanceSamples.reduce((sum, value) => sum + value, 0) / distanceSamples.length
    : 1;
  return {
    kind: "tracking",
    multiplier,
    duration_ms: Math.round(durationMs),
    hits: 0,
    targets: distanceSamples.length,
    average_reaction_ms: 0,
    path_efficiency: clamp(1 - averageDistanceRatio, 0, 1),
    overshoots,
    on_target_ratio: clamp(onTargetMs / Math.max(1, durationMs), 0, 1),
  };
}
