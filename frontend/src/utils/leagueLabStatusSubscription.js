import { fetchLeagueLabStatus } from "../api/leagueLabApi";

// League's desktop helpers all need the same small status snapshot. Keeping
// one poller here prevents the Mini, shortcut, and auxiliary managers from
// opening four independent request loops while preserving the Mini's 1.5 s
// freshness requirement.
const POLL_INTERVAL_MS = 1500;

let snapshot = null;
let pollTimer = null;
let inFlight = null;
let generation = 0;
const listeners = new Set();

function notify(next) {
  for (const listener of [...listeners]) {
    try {
      listener(next);
    } catch {
      // A manager must not be able to break the shared poller.
    }
  }
}

async function poll() {
  const currentGeneration = generation;
  if (inFlight?.generation === currentGeneration) return inFlight.promise;
  const promise = fetchLeagueLabStatus()
    .then((next) => {
      if (currentGeneration !== generation || listeners.size === 0) return null;
      snapshot = next;
      notify(next);
      return next;
    })
    .catch(() => {
      if (currentGeneration !== generation || listeners.size === 0) return null;
      snapshot = null;
      notify(null);
      return null;
    })
    .finally(() => {
      if (inFlight?.promise === promise) inFlight = null;
    });
  inFlight = { generation: currentGeneration, promise };
  return promise;
}

function ensureStarted() {
  if (pollTimer || listeners.size === 0) return;
  generation += 1;
  void poll();
  pollTimer = window.setInterval(() => { void poll(); }, POLL_INTERVAL_MS);
}

function stopIfUnused() {
  if (listeners.size !== 0) return;
  generation += 1;
  if (pollTimer) window.clearInterval(pollTimer);
  pollTimer = null;
  // Do not retain a League status object after the last desktop manager is
  // gone. The next mount performs a fresh read instead of replaying stale
  // account/game data from a previous auxiliary-window session.
  snapshot = null;
}

export function subscribeLeagueLabStatus(listener) {
  if (typeof listener !== "function") return () => {};
  listeners.add(listener);
  if (snapshot !== null) {
    try {
      listener(snapshot);
    } catch {
      // Keep subscription setup resilient to a consumer's first callback.
    }
  }
  ensureStarted();
  return () => {
    listeners.delete(listener);
    stopIfUnused();
  };
}

export function getLeagueLabStatusSnapshot() {
  return snapshot;
}
