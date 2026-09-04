export const LEAGUE_STARTUP_MODE_STORAGE_KEY = "maxgamestudio.league.startup-mode";
export const LEAGUE_STARTUP_REMEMBER_STORAGE_KEY = "maxgamestudio.league.startup-mode.remember";
export const LEAGUE_STARTUP_ADMINISTRATOR_STORAGE_KEY = "maxgamestudio.league.startup-mode.administrator";

export const LEAGUE_STARTUP_MODES = Object.freeze([
  {
    id: "ask",
    titleKey: "settings.leagueStartupModeAsk",
    descriptionKey: "settings.leagueStartupModeAskHint",
  },
  {
    id: "memory",
    titleKey: "settings.leagueStartupModeMemory",
    descriptionKey: "settings.leagueStartupModeMemoryHint",
  },
  {
    id: "parallel",
    titleKey: "settings.leagueStartupModeParallel",
    descriptionKey: "settings.leagueStartupModeParallelHint",
  },
]);

export function isLeagueStartupMode(value) {
  return LEAGUE_STARTUP_MODES.some((item) => item.id === value);
}

export function readLeagueStartupPreference(storage = globalThis.localStorage) {
  try {
    const mode = storage?.getItem(LEAGUE_STARTUP_MODE_STORAGE_KEY);
    if (!isLeagueStartupMode(mode)) return null;
    return {
      mode,
      remembered: storage.getItem(LEAGUE_STARTUP_REMEMBER_STORAGE_KEY) !== "false",
      administrator: storage.getItem(LEAGUE_STARTUP_ADMINISTRATOR_STORAGE_KEY) === "true",
    };
  } catch {
    return null;
  }
}

export function writeLeagueStartupPreference(mode, remember = true, storage = globalThis.localStorage, { administrator = false } = {}) {
  if (!isLeagueStartupMode(mode)) return false;
  try {
    if (!remember) {
      storage?.removeItem(LEAGUE_STARTUP_MODE_STORAGE_KEY);
      storage?.removeItem(LEAGUE_STARTUP_REMEMBER_STORAGE_KEY);
      storage?.removeItem(LEAGUE_STARTUP_ADMINISTRATOR_STORAGE_KEY);
      return true;
    }
    storage?.setItem(LEAGUE_STARTUP_MODE_STORAGE_KEY, mode);
    storage?.setItem(LEAGUE_STARTUP_REMEMBER_STORAGE_KEY, "true");
    if (mode !== "ask" && administrator === true) storage?.setItem(LEAGUE_STARTUP_ADMINISTRATOR_STORAGE_KEY, "true");
    else storage?.removeItem(LEAGUE_STARTUP_ADMINISTRATOR_STORAGE_KEY);
    return true;
  } catch {
    return false;
  }
}

export function clearLeagueStartupPreference(storage = globalThis.localStorage) {
  try {
    storage?.removeItem(LEAGUE_STARTUP_MODE_STORAGE_KEY);
    storage?.removeItem(LEAGUE_STARTUP_REMEMBER_STORAGE_KEY);
    storage?.removeItem(LEAGUE_STARTUP_ADMINISTRATOR_STORAGE_KEY);
    return true;
  } catch {
    return false;
  }
}
