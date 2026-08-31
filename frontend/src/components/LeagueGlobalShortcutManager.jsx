import { useEffect, useRef } from "react";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { terminateLeagueGameClient } from "../api/leagueLabApi";
import { subscribeLeagueLabStatus } from "../utils/leagueLabStatusSubscription";

const TRIGGER_DEBOUNCE_MS = 1500;

export default function LeagueGlobalShortcutManager() {
  const registeredShortcut = useRef("");
  const syncing = useRef(false);
  const lastTriggeredAt = useRef(0);

  useEffect(() => {
    if (!window.__TAURI_INTERNALS__) return undefined;
    let disposed = false;

    const clearRegisteredShortcut = async () => {
      const current = registeredShortcut.current;
      registeredShortcut.current = "";
      if (current) await unregister(current).catch(() => {});
    };

    const sync = async (status) => {
      if (disposed || syncing.current) return;
      syncing.current = true;
      try {
        if (disposed || !status) return;
        const settings = status?.settings || {};
        const desired = settings.terminate_game_shortcut_enabled
          ? String(settings.terminate_game_shortcut || "").trim()
          : "";
        if (desired === registeredShortcut.current) return;
        await clearRegisteredShortcut();
        if (!desired || disposed) return;
        await register(desired, async (event) => {
          if (event?.state !== "Pressed") return;
          const now = Date.now();
          if (now - lastTriggeredAt.current < TRIGGER_DEBOUNCE_MS) return;
          lastTriggeredAt.current = now;
          await terminateLeagueGameClient().catch(() => {
            // The backend deliberately rejects the action unless League is foreground.
          });
        });
        if (disposed) {
          await unregister(desired).catch(() => {});
          return;
        }
        registeredShortcut.current = desired;
      } catch {
        // Backend startup races and accelerators owned by another app are retried.
      } finally {
        syncing.current = false;
      }
    };

    const unsubscribe = subscribeLeagueLabStatus((status) => { void sync(status); });
    return () => {
      disposed = true;
      unsubscribe();
      void clearRegisteredShortcut();
    };
  }, []);

  return null;
}
