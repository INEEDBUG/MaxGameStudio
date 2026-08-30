import { useCallback, useEffect, useRef, useState } from "react";
import { Crosshair, MousePointer2, Play, RotateCcw } from "lucide-react";
import {
  clamp,
  classifyFlickClick,
  cs2CursorGain,
  makeFlickTrialResult,
  makeTrackingTrialResult,
  pointDistance,
  sensitivityToCm360,
} from "../../utils/sensitivityLab";
import { useT } from "../../i18n/useT.js";

const COUNTDOWN_MS = 3_000;
const TARGET_RADIUS = 27;

function randomTarget(width, height, avoid = null) {
  const margin = TARGET_RADIUS + 18;
  const minimumTravel = Math.min(width, height) * 0.22;
  let candidate = null;
  for (let attempt = 0; attempt < 10; attempt += 1) {
    candidate = {
      x: margin + Math.random() * Math.max(1, width - margin * 2),
      y: margin + Math.random() * Math.max(1, height - margin * 2),
    };
    if (!avoid || pointDistance(candidate, avoid) >= minimumTravel) return candidate;
  }
  return candidate;
}

export default function SensitivityAimArena({ trial, setup, index, total, durationMs = 15_000, onComplete, onCancel }) {
  const t = useT();
  const roundDurationMs = durationMs > 0 ? durationMs : null;
  const currentSensitivity = Number(setup?.current_sensitivity || 1);
  const dpi = Number(setup?.dpi || 800);
  const mYaw = Number(setup?.m_yaw || 0.022);
  const candidateSensitivity = currentSensitivity * trial.multiplier;
  const candidateEdpi = dpi * candidateSensitivity;
  const candidateCm360 = sensitivityToCm360(dpi, candidateSensitivity, mYaw);
  const cursorGain = cs2CursorGain(currentSensitivity, mYaw, trial.multiplier);
  const canvasRef = useRef(null);
  const frameRef = useRef(0);
  const stateRef = useRef(null);
  const [phase, setPhase] = useState("ready");
  const [displayTime, setDisplayTime] = useState(roundDurationMs ?? 0);
  const [pointerLocked, setPointerLocked] = useState(false);

  const requestPointerLock = useCallback(() => {
    try {
      const pending = canvasRef.current?.requestPointerLock?.();
      if (pending && typeof pending.catch === "function") pending.catch(() => setPointerLocked(false));
    } catch {
      setPointerLocked(false);
    }
  }, []);

  const resizeCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const width = Math.max(1, Math.round(rect.width));
    const height = Math.max(1, Math.round(rect.height));
    if (canvas.width !== Math.round(width * dpr) || canvas.height !== Math.round(height * dpr)) {
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
    }
    const context = canvas.getContext("2d");
    context.setTransform(dpr, 0, 0, dpr, 0, 0);
    return { canvas, context, width, height };
  }, []);

  const finishRound = useCallback(() => {
    cancelAnimationFrame(frameRef.current);
    const state = stateRef.current;
    if (!state || state.finished) return;
    state.finished = true;
    if (document.pointerLockElement) document.exitPointerLock();
    const durationMs = Math.max(1, performance.now() - state.startAt);
    const result = trial.kind === "flick"
      ? makeFlickTrialResult({
          multiplier: trial.multiplier,
          durationMs,
          reactions: state.reactions,
          efficiencies: state.efficiencies,
          overshoots: state.overshoots,
          clicks: state.clicks,
          misses: state.misses,
          underflicks: state.underflicks,
          overflicks: state.overflicks,
          offAxisMisses: state.offAxisMisses,
          clickErrors: state.clickErrors,
        })
      : makeTrackingTrialResult({
          multiplier: trial.multiplier,
          durationMs,
          onTargetMs: state.onTargetMs,
          distanceSamples: state.distanceSamples,
          overshoots: state.overshoots,
        });
    setPhase("done");
    onComplete(result);
  }, [onComplete, trial]);

  const draw = useCallback((now) => {
    const surface = resizeCanvas();
    const state = stateRef.current;
    if (!surface || !state || state.finished) return;
    const { context, width, height } = surface;
    const remaining = state.startAt - now;
    const running = remaining <= 0;
    context.clearRect(0, 0, width, height);

    const gradient = context.createRadialGradient(width / 2, height / 2, 20, width / 2, height / 2, width * 0.7);
    gradient.addColorStop(0, "#17202b");
    gradient.addColorStop(1, "#080c12");
    context.fillStyle = gradient;
    context.fillRect(0, 0, width, height);
    context.strokeStyle = "rgba(255,255,255,0.035)";
    context.lineWidth = 1;
    for (let x = 0; x < width; x += 42) {
      context.beginPath(); context.moveTo(x, 0); context.lineTo(x, height); context.stroke();
    }
    for (let y = 0; y < height; y += 42) {
      context.beginPath(); context.moveTo(0, y); context.lineTo(width, y); context.stroke();
    }

    if (running) {
      if (!state.running) {
        state.running = true;
        setPhase("running");
        state.lastFrameAt = now;
        state.targetSpawnAt = now;
        state.target = randomTarget(width, height, state.cursor);
        state.targetStartCursor = { ...state.cursor };
        state.directDistance = pointDistance(state.cursor, state.target);
      }
      const dt = Math.min(50, now - state.lastFrameAt);
      state.lastFrameAt = now;
      const elapsed = now - state.startAt;
      if (trial.kind === "tracking") {
        const radiusX = width * 0.27;
        const radiusY = height * 0.22;
        state.target = {
          x: width / 2 + Math.sin(elapsed / 820) * radiusX + Math.sin(elapsed / 310) * 35,
          y: height / 2 + Math.cos(elapsed / 1070) * radiusY,
        };
        const distance = pointDistance(state.cursor, state.target);
        const isOnTarget = distance <= TARGET_RADIUS;
        if (isOnTarget) state.onTargetMs += dt;
        if (state.wasOnTarget && !isOnTarget) state.overshoots += 1;
        state.wasOnTarget = isOnTarget;
        state.distanceSamples.push(clamp(distance / Math.max(width, height), 0, 1));
      }

      context.beginPath();
      context.arc(state.target.x, state.target.y, TARGET_RADIUS + 7, 0, Math.PI * 2);
      context.fillStyle = "rgba(255, 154, 61, 0.14)";
      context.fill();
      context.beginPath();
      context.arc(state.target.x, state.target.y, TARGET_RADIUS, 0, Math.PI * 2);
      context.fillStyle = trial.kind === "tracking" && state.wasOnTarget ? "#5ee6a8" : "#ff9a3d";
      context.fill();
      context.beginPath();
      context.arc(state.target.x, state.target.y, 5, 0, Math.PI * 2);
      context.fillStyle = "#071018";
      context.fill();

      const displayMs = roundDurationMs == null ? elapsed : Math.max(0, state.endAt - now);
      if (now - state.lastDisplayAt > 60) {
        state.lastDisplayAt = now;
        setDisplayTime(displayMs);
      }
      if (roundDurationMs != null && now >= state.endAt) {
        finishRound();
        return;
      }
    } else {
      context.fillStyle = "rgba(3,7,12,0.58)";
      context.fillRect(0, 0, width, height);
      context.fillStyle = "#f6f7f9";
      context.font = "700 64px Rajdhani, sans-serif";
      context.textAlign = "center";
      context.textBaseline = "middle";
      context.fillText(String(Math.max(1, Math.ceil(remaining / 1000))), width / 2, height / 2);
    }

    const cursor = state.cursor;
    context.strokeStyle = "#ffffff";
    context.lineWidth = 2;
    context.beginPath();
    context.moveTo(cursor.x - 11, cursor.y); context.lineTo(cursor.x - 3, cursor.y);
    context.moveTo(cursor.x + 3, cursor.y); context.lineTo(cursor.x + 11, cursor.y);
    context.moveTo(cursor.x, cursor.y - 11); context.lineTo(cursor.x, cursor.y - 3);
    context.moveTo(cursor.x, cursor.y + 3); context.lineTo(cursor.x, cursor.y + 11);
    context.stroke();
    context.beginPath(); context.arc(cursor.x, cursor.y, 1.5, 0, Math.PI * 2);
    context.fillStyle = "#ff9a3d"; context.fill();
    frameRef.current = requestAnimationFrame(draw);
  }, [finishRound, resizeCanvas, roundDurationMs, trial.kind]);

  const start = useCallback(() => {
    const surface = resizeCanvas();
    if (!surface) return;
    stateRef.current = {
      cursor: { x: surface.width / 2, y: surface.height / 2 },
      target: { x: surface.width / 2, y: surface.height / 2 },
      targetStartCursor: { x: surface.width / 2, y: surface.height / 2 },
      startAt: 0,
      endAt: 0,
      lastFrameAt: 0,
      lastDisplayAt: 0,
      targetSpawnAt: 0,
      directDistance: 1,
      pathDistance: 0,
      minDistance: Infinity,
      overshootForTarget: false,
      reactions: [],
      efficiencies: [],
      overshoots: 0,
      clicks: 0,
      misses: 0,
      underflicks: 0,
      overflicks: 0,
      offAxisMisses: 0,
      clickErrors: [],
      onTargetMs: 0,
      distanceSamples: [],
      wasOnTarget: false,
      running: false,
      timerStarted: false,
      pausedAt: null,
      finished: false,
    };
    setDisplayTime(roundDurationMs ?? 0);
    setPhase("awaiting-lock");
    requestPointerLock();
    cancelAnimationFrame(frameRef.current);
  }, [requestPointerLock, resizeCanvas, roundDurationMs]);

  useEffect(() => {
    function onPointerLockChange() {
      const locked = document.pointerLockElement === canvasRef.current;
      const state = stateRef.current;
      setPointerLocked(locked);
      if (!state || state.finished) return;
      const now = performance.now();
      if (locked && !state.timerStarted) {
        state.timerStarted = true;
        state.startAt = now + COUNTDOWN_MS;
        state.endAt = roundDurationMs == null ? null : state.startAt + roundDurationMs;
        state.lastFrameAt = now;
        state.targetSpawnAt = now;
        setPhase("countdown");
        cancelAnimationFrame(frameRef.current);
        frameRef.current = requestAnimationFrame(draw);
      } else if (locked && state.pausedAt !== null) {
        const pausedFor = now - state.pausedAt;
        state.startAt += pausedFor;
        if (state.endAt != null) state.endAt += pausedFor;
        state.lastFrameAt = now;
        state.pausedAt = null;
        cancelAnimationFrame(frameRef.current);
        frameRef.current = requestAnimationFrame(draw);
      } else if (!locked && state.timerStarted && state.pausedAt === null) {
        state.pausedAt = now;
        cancelAnimationFrame(frameRef.current);
      }
    }
    function onMouseMove(event) {
      const state = stateRef.current;
      const surface = resizeCanvas();
      if (!state || !surface || state.finished || !state.running) return;
      const previous = { ...state.cursor };
      state.cursor.x = clamp(state.cursor.x + event.movementX * cursorGain, 0, surface.width);
      state.cursor.y = clamp(state.cursor.y + event.movementY * cursorGain, 0, surface.height);
      const moved = pointDistance(previous, state.cursor);
      if (trial.kind === "flick") {
        state.pathDistance += moved;
        const distance = pointDistance(state.cursor, state.target);
        if (distance < state.minDistance) state.minDistance = distance;
        else if (
          !state.overshootForTarget &&
          state.minDistance <= TARGET_RADIUS &&
          distance > state.minDistance + TARGET_RADIUS * 0.55
        ) {
          state.overshoots += 1;
          state.overshootForTarget = true;
        }
      }
    }
    function onMouseDown(event) {
      const state = stateRef.current;
      const surface = resizeCanvas();
      if (
        event.button !== 0 ||
        trial.kind !== "flick" ||
        document.pointerLockElement !== canvasRef.current ||
        !state ||
        !surface ||
        state.finished ||
        !state.running
      ) return;
      event.preventDefault();
      const outcome = classifyFlickClick({
        start: state.targetStartCursor,
        target: state.target,
        cursor: state.cursor,
        targetRadius: TARGET_RADIUS,
        overshot: state.overshootForTarget,
      });
      state.clicks += 1;
      state.clickErrors.push(outcome.errorRatio);
      state.efficiencies.push(
        clamp(state.directDistance / Math.max(state.directDistance, state.pathDistance), 0, 1),
      );
      if (outcome.hit) {
        state.reactions.push(performance.now() - state.targetSpawnAt);
      } else {
        state.misses += 1;
      }
      if (outcome.direction === "underflick") state.underflicks += 1;
      else if (outcome.direction === "overflick") state.overflicks += 1;
      else if (outcome.direction === "off_axis") state.offAxisMisses += 1;

      state.target = randomTarget(surface.width, surface.height, state.cursor);
      state.targetSpawnAt = performance.now();
      state.targetStartCursor = { ...state.cursor };
      state.directDistance = pointDistance(state.cursor, state.target);
      state.pathDistance = 0;
      state.minDistance = state.directDistance;
      state.overshootForTarget = false;
    }
    document.addEventListener("pointerlockchange", onPointerLockChange);
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mousedown", onMouseDown);
    return () => {
      document.removeEventListener("pointerlockchange", onPointerLockChange);
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mousedown", onMouseDown);
      cancelAnimationFrame(frameRef.current);
      if (document.pointerLockElement === canvasRef.current) document.exitPointerLock();
    };
  }, [cursorGain, draw, resizeCanvas, roundDurationMs, trial.kind]);

  return (
    <section className="overflow-hidden rounded-2xl border border-cs2-border bg-cs2-bg-card shadow-2xl shadow-black/25">
      <header className="flex items-center justify-between border-b border-cs2-border px-4 py-3">
        <div className="flex items-center gap-2">
          {trial.kind === "flick" ? <Crosshair className="h-4 w-4 text-cs2-orange" /> : <MousePointer2 className="h-4 w-4 text-cs2-orange" />}
          <span className="text-sm font-semibold text-cs2-text-primary">
            {trial.kind === "flick" ? t("training.flickTitle") : t("training.trackingTitle")}
          </span>
          <span className="rounded bg-white/5 px-2 py-0.5 font-mono text-[11px] text-cs2-text-muted">
            {index + 1}/{total} · ×{trial.multiplier.toFixed(1)}
          </span>
          <span className="hidden rounded border border-cs2-accent/20 bg-cs2-accent/[0.08] px-2 py-0.5 font-mono text-[11px] text-cs2-accent sm:inline">
            sensitivity {candidateSensitivity.toFixed(4)} · {Math.round(candidateEdpi)} eDPI · m_yaw {mYaw}
          </span>
        </div>
        <div className="font-mono text-lg font-bold tabular-nums text-cs2-text-primary">
          {roundDurationMs == null ? `∞ · ${(displayTime / 1000).toFixed(1)}s` : `${(displayTime / 1000).toFixed(1)}s`}
        </div>
      </header>
      <div className="relative h-[min(58vh,560px)] min-h-[390px] bg-black">
        <canvas ref={canvasRef} className="h-full w-full cursor-none" aria-label={t("training.arenaLabel")} />
        {phase === "ready" && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/60 px-6 backdrop-blur-sm">
            <div className="max-w-md text-center">
              <h2 className="text-xl font-bold text-white">
                {trial.kind === "flick" ? t("training.flickReady") : t("training.trackingReady")}
              </h2>
              <p className="mt-2 text-sm leading-6 text-zinc-300">
                {trial.kind === "flick" ? t("training.flickHelp") : t("training.trackingHelp")}
              </p>
              <div className="mt-4 grid grid-cols-3 gap-2 text-left">
                {[
                  ["Sensitivity", candidateSensitivity.toFixed(4)],
                  ["eDPI", Math.round(candidateEdpi)],
                  ["cm/360", candidateCm360.toFixed(1)],
                ].map(([label, value]) => (
                  <div key={label} className="rounded-lg border border-white/10 bg-black/25 px-2.5 py-2">
                    <div className="text-[9px] font-bold uppercase tracking-wider text-zinc-500">{label}</div>
                    <div className="mt-0.5 font-mono text-sm font-bold text-white">{value}</div>
                  </div>
                ))}
              </div>
              <button
                type="button"
                onClick={start}
                className="mt-5 inline-flex items-center gap-2 rounded-lg bg-cs2-orange px-5 py-2.5 text-sm font-bold text-black transition-transform duration-150 active:scale-[0.97]"
              >
                <Play className="h-4 w-4 fill-current" />
                {t("training.startRound")}
              </button>
            </div>
          </div>
        )}
        {phase !== "ready" && phase !== "done" && !pointerLocked && (
          <button
            type="button"
            onClick={requestPointerLock}
            className="absolute bottom-4 left-1/2 -translate-x-1/2 rounded-lg border border-amber-400/40 bg-amber-400/15 px-3 py-2 text-xs font-semibold text-amber-200"
          >
            {t("training.relockPointer")}
          </button>
        )}
      </div>
      <footer className="flex items-center justify-between border-t border-cs2-border px-4 py-3 text-xs text-cs2-text-muted">
        <span>{t("training.noClickHint")}</span>
        <div className="flex items-center gap-2">
          {roundDurationMs == null && phase === "running" && (
            <button type="button" disabled={displayTime < 3_000} onClick={finishRound} className="rounded-lg bg-cs2-orange px-3 py-1.5 font-bold text-black transition-transform duration-150 active:scale-[0.97] disabled:opacity-40">
              {t("training.finishRound")}
            </button>
          )}
          <button type="button" onClick={onCancel} className="flex items-center gap-1.5 rounded px-2 py-1 transition-colors duration-150 hover:bg-white/5 hover:text-cs2-text-primary active:scale-[0.97]">
            <RotateCcw className="h-3.5 w-3.5" /> {t("training.exitTest")}
          </button>
        </div>
      </footer>
    </section>
  );
}
