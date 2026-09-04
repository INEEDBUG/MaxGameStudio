import { desktopBridge, isDesktopApp } from "../desktop/desktopBridge.js";

export const LEAGUE_RUNTIME_HANDLED_SESSION_KEY = "maxgamestudio.league.runtime.handled-session.v1";
const HANDLED_SESSION_MAX_AGE_MS = 12 * 60 * 60 * 1000;
const LAUNCH_MODES = new Set(["memory", "parallel"]);

let launchInFlight = null;

export function leagueClientSessionId(status) {
  if (!status || typeof status !== "object") return "";
  const pid = Number(status.client_pid);
  if (status.connected === true && Number.isInteger(pid) && pid > 0) return `pid:${pid}`;
  if (status.client_window_detected === true) return "window";
  return "";
}

export function readHandledLeagueSession(storage = globalThis.localStorage, now = Date.now()) {
  try {
    const parsed = JSON.parse(storage?.getItem(LEAGUE_RUNTIME_HANDLED_SESSION_KEY) || "null");
    const handledAt = Number(parsed?.handledAt);
    if (typeof parsed?.sessionId !== "string" || !parsed.sessionId || !Number.isFinite(handledAt)) return null;
    if (now - handledAt > HANDLED_SESSION_MAX_AGE_MS || handledAt - now > 60_000) {
      storage?.removeItem(LEAGUE_RUNTIME_HANDLED_SESSION_KEY);
      return null;
    }
    return { sessionId: parsed.sessionId, handledAt };
  } catch {
    return null;
  }
}

export function isLeagueSessionHandled(sessionId, storage = globalThis.localStorage, now = Date.now()) {
  if (!sessionId) return false;
  const handled = readHandledLeagueSession(storage, now);
  return handled?.sessionId === "*" || handled?.sessionId === sessionId;
}

export function markLeagueSessionHandled(sessionId = "*", storage = globalThis.localStorage, now = Date.now()) {
  try {
    storage?.setItem(LEAGUE_RUNTIME_HANDLED_SESSION_KEY, JSON.stringify({ sessionId: sessionId || "*", handledAt: now }));
    return true;
  } catch {
    return false;
  }
}

export function clearHandledLeagueSession(storage = globalThis.localStorage) {
  try {
    storage?.removeItem(LEAGUE_RUNTIME_HANDLED_SESSION_KEY);
    return true;
  } catch {
    return false;
  }
}

export async function launchLeagueRuntimeCoordinated(mode, {
  force = false,
  sessionId = "*",
  administrator = false,
  storage = globalThis.localStorage,
} = {}) {
  if (!LAUNCH_MODES.has(mode)) throw new Error("League runtime mode must be memory or parallel");
  if (!isDesktopApp || !desktopBridge?.launchLeagueRuntime) throw new Error("League runtime is available only in the desktop app");
  const normalizedSessionId = sessionId || "*";
  if (!force && isLeagueSessionHandled(normalizedSessionId, storage)) {
    return { launched: false, reason: "handled" };
  }
  if (launchInFlight) {
    if (launchInFlight.mode !== mode || launchInFlight.administrator !== administrator) {
      return {
        launched: false,
        reason: "in-flight",
        mode: launchInFlight.mode,
        administrator: launchInFlight.administrator,
      };
    }
    return launchInFlight.promise;
  }

  markLeagueSessionHandled(normalizedSessionId, storage);
  const pending = Promise.resolve()
    .then(() => administrator
      ? desktopBridge.launchLeagueRuntime(mode, { administrator: true })
      : desktopBridge.launchLeagueRuntime(mode))
    .then(() => ({ launched: true, reason: "started" }))
    .catch((error) => {
      const handled = readHandledLeagueSession(storage);
      if (handled?.sessionId === normalizedSessionId) clearHandledLeagueSession(storage);
      throw error;
    })
    .finally(() => {
      if (launchInFlight?.promise === pending) launchInFlight = null;
    });
  launchInFlight = { mode, administrator, promise: pending };
  return pending;
}
