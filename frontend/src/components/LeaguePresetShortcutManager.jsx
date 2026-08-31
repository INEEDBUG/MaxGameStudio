import { useEffect, useRef } from "react";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { fetchLeagueLabStatus, fetchLeagueOngoingGame, sendLeagueInGameLines, sendLeagueInGamePreset } from "../api/leagueLabApi";
import { subscribeLeagueLabStatus } from "../utils/leagueLabStatusSubscription";
import {
  buildLeaguePresetLines,
  getLeaguePresetOptions,
  selectLeaguePresetPlayers,
  shortcutSettingsKey,
} from "../utils/leagueChatPresets";

export default function LeaguePresetShortcutManager() {
  const registered = useRef(new Map());
  const syncing = useRef(false);
  const lastTriggered = useRef(new Map());

  useEffect(() => {
    if (!window.__TAURI_INTERNALS__) return undefined;
    let disposed = false;

    const sync = async (status) => {
      if (disposed || syncing.current) return;
      syncing.current = true;
      try {
        if (disposed || !status) return;
        const settings = status?.settings || {};
        const desired = new Map();
        if (settings.toolkit_account_actions_enabled && settings.in_game_send_enabled) {
          for (const preset of settings.in_game_fixed_presets || []) {
            const shortcut = String(preset?.shortcut || "").trim();
            if (preset?.id && shortcut && !desired.has(shortcut)) desired.set(shortcut, `fixed:${preset.id}`);
          }
          for (const kind of ["rating", "premade", "jungle"]) for (const target of ["friendly", "enemy", "all"]) {
            const shortcut = String(settings[shortcutSettingsKey(kind)]?.[target] || "").trim();
            if (shortcut && !desired.has(shortcut)) desired.set(shortcut, `generated:${kind}:${target}`);
          }
        }
        for (const [shortcut, presetId] of [...registered.current.entries()]) {
          if (desired.get(shortcut) === presetId) continue;
          await unregister(shortcut).catch(() => {});
          registered.current.delete(shortcut);
        }
        for (const [shortcut, action] of desired.entries()) {
          if (disposed || registered.current.get(shortcut) === action) continue;
          try {
            await register(shortcut, async (event) => {
              if (event?.state !== "Pressed") return;
              const now = Date.now();
              if (now - (lastTriggered.current.get(action) || 0) < 1000) return;
              lastTriggered.current.set(action, now);
              if (action.startsWith("fixed:")) {
                await sendLeagueInGamePreset(action.slice(6), "shortcut", "").catch(() => {});
                return;
              }
              const [, kind, target] = action.split(":");
              try {
                // A shortcut is an explicit user action, so refresh the
                // snapshot once for the current summoner/options. This is
                // separate from the shared 1.5 s background status poll.
                const [game, liveStatus] = await Promise.all([fetchLeagueOngoingGame(), fetchLeagueLabStatus()]);
                const players = game?.players || [];
                const options = getLeaguePresetOptions(liveStatus?.settings, kind);
                // Shortcut target is authoritative for the trigger; the
                // saved selected-player list is still honoured only when the
                // user explicitly chose it in the page.
                const targetOptions = { ...options, targetMode: target };
                const selected = selectLeaguePresetPlayers(players, targetOptions, liveStatus?.current_summoner?.puuid || "");
                const lines = buildLeaguePresetLines(kind, selected, options, {});
                if (lines.length) await sendLeagueInGameLines(lines.slice(0, 10), "", "shortcut", kind, target);
              } catch {
                // Missing live data or a backend safety rejection leaves the game untouched.
              }
            });
            if (!disposed) registered.current.set(shortcut, action);
          } catch {
            // Invalid or occupied accelerators remain unregistered and are retried.
          }
        }
      } catch {
        // Backend startup and shutdown races are expected.
      } finally {
        syncing.current = false;
      }
    };

    const unsubscribe = subscribeLeagueLabStatus((status) => { void sync(status); });
    return () => {
      disposed = true;
      unsubscribe();
      for (const shortcut of registered.current.keys()) void unregister(shortcut).catch(() => {});
      registered.current.clear();
    };
  }, []);

  return null;
}
