import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { cancelLeagueInGameSend } from "../api/leagueLabApi";
import { subscribeLeagueLabStatus } from "../utils/leagueLabStatusSubscription";

export default function LeagueAuxShortcutManager() {
  const registered = useRef(new Map());
  const syncing = useRef(false);

  useEffect(() => {
    if (!window.__TAURI_INTERNALS__) return undefined;
    let disposed = false;

    const sync = async (status) => {
      if (disposed || syncing.current) return;
      syncing.current = true;
      try {
        if (disposed || !status) return;
        const settings = status?.settings || {};
        const candidates = [
          [settings.in_game_send_enabled ? settings.in_game_cancel_shortcut : null, "cancel"],
          [settings.ongoing_window_shortcut, "ongoing"],
          [settings.cooldown_window_shortcut, "cooldown"],
        ];
        const desired = new Map();
        for (const [rawShortcut, action] of candidates) {
          const shortcut = String(rawShortcut || "").trim();
          if (shortcut && !desired.has(shortcut)) desired.set(shortcut, action);
        }
        for (const [shortcut, action] of [...registered.current.entries()]) {
          if (desired.get(shortcut) === action) continue;
          await unregister(shortcut).catch(() => {});
          registered.current.delete(shortcut);
        }
        for (const [shortcut, action] of desired.entries()) {
          if (disposed || registered.current.get(shortcut) === action) continue;
          try {
            await register(shortcut, async (event) => {
              if (action === "ongoing") {
                if (event?.state === "Pressed") await invoke("toggle_league_aux_window", { kind: action, visible: true }).catch(() => {});
                if (event?.state === "Released") await invoke("toggle_league_aux_window", { kind: action, visible: false }).catch(() => {});
                return;
              }
              if (event?.state !== "Pressed") return;
              if (action === "cancel") await cancelLeagueInGameSend().catch(() => {});
              else await invoke("toggle_league_aux_window", { kind: action, visible: null }).catch(() => {});
            });
            if (!disposed) registered.current.set(shortcut, action);
          } catch {
            // Occupied or invalid shortcuts remain inactive and are retried.
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
