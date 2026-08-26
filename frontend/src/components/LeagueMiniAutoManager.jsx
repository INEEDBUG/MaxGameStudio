import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { fetchLeagueLabStatus } from "../api/leagueLabApi";

const MINI_AUTO_PHASES = new Set(["Lobby", "Matchmaking", "ReadyCheck", "ChampSelect"]);
const COOLDOWN_AUTO_PHASES = new Set(["InProgress"]);

export default function LeagueMiniAutoManager() {
  const lastSync = useRef("");
  const lastCooldownSync = useRef("");
  // Do not allow overlapping polls. A late ChampSelect response can
  // otherwise re-show Mini after the client has entered InProgress.
  const syncing = useRef(false);

  useEffect(() => {
    if (!window.__TAURI_INTERNALS__) return undefined;
    let disposed = false;
    const sync = async () => {
      if (disposed || syncing.current) return;
      syncing.current = true;
      try {
        const status = await fetchLeagueLabStatus();
        if (disposed) return;
        const settings = status?.settings || {};
        // Keep the phase gate local to the desktop manager as well as the
        // backend. A stale status response during a game must never create an
        // auxiliary WebView merely because the page was opened/refreshed.
        const shouldShow = Boolean(
          settings.mini_enabled
          && settings.mini_auto_show
          && MINI_AUTO_PHASES.has(status?.phase)
          && status?.mini_should_show,
        );
        const context = `${status?.connected ? "connected" : "offline"}:${status?.phase || "None"}:${status?.champ_select?.is_spectating ? "spectating" : "playing"}`;
        const contentProtected = Boolean(settings.streamer_content_protection_enabled);
        const signature = `${shouldShow}:${context}:${contentProtected}`;
        if (signature !== lastSync.current) {
          await invoke("set_league_content_protection", { enabled: contentProtected });
          await invoke("sync_league_mini", { shouldShow, context });
          lastSync.current = signature;
        }
        const cooldownShouldShow = Boolean(
          settings.cooldown_timer_enabled
          && COOLDOWN_AUTO_PHASES.has(status?.phase)
          && status?.cooldown_timer_should_show,
        );
        const cooldownContext = `${status?.connected ? "connected" : "offline"}:${status?.phase || "None"}:${status?.game_mode || "unknown"}`;
        const cooldownSignature = `${cooldownShouldShow}:${cooldownContext}:${contentProtected}`;
        if (cooldownSignature !== lastCooldownSync.current) {
          await invoke("sync_league_cd_timer", { shouldShow: cooldownShouldShow, context: cooldownContext });
          lastCooldownSync.current = cooldownSignature;
        }
      } catch {
        // Backend startup and shutdown races are expected; the next poll retries.
      } finally {
        syncing.current = false;
      }
    };
    sync();
    const timer = window.setInterval(sync, 1500);
    return () => { disposed = true; window.clearInterval(timer); };
  }, []);

  return null;
}
