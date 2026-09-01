import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { subscribeLeagueLabStatus } from "../utils/leagueLabStatusSubscription";

const MINI_AUTO_PHASES = new Set(["Lobby", "Matchmaking", "ReadyCheck", "ChampSelect"]);
const ONGOING_AUTO_PHASES = new Set(["GameStart", "InProgress", "Reconnect"]);
const COOLDOWN_AUTO_PHASES = new Set(["InProgress"]);

export default function LeagueMiniAutoManager() {
  const lastSync = useRef("");
  const lastOngoingSync = useRef("");
  const lastCooldownSync = useRef("");
  const syncing = useRef(false);
  const pendingStatus = useRef(undefined);

  useEffect(() => {
    if (!window.__TAURI_INTERNALS__) return undefined;
    let disposed = false;
    const sync = async (status) => {
      if (disposed) return;
      // Keep only the newest snapshot while native window work is running.
      // Dropping an InProgress snapshot here can leave Mini visible over the
      // game until another phase change arrives.
      pendingStatus.current = status;
      if (syncing.current) return;
      syncing.current = true;
      try {
        while (!disposed && pendingStatus.current !== undefined) {
          const nextStatus = pendingStatus.current;
          pendingStatus.current = undefined;
          const current = nextStatus || { connected: false, phase: "None", settings: {}, mini_should_show: false, cooldown_timer_should_show: false };
          const settings = current.settings || {};
          // Keep the phase gate local to the desktop manager as well as the
          // backend. A stale status response during a game must never create an
          // auxiliary WebView merely because the page was opened/refreshed.
          const shouldShow = Boolean(
            settings.mini_enabled
            && settings.mini_auto_show
            && MINI_AUTO_PHASES.has(current.phase)
            && current.mini_should_show,
          );
          const context = `${current.connected ? "connected" : "offline"}:${current.phase || "None"}:${current.champ_select?.is_spectating ? "spectating" : "playing"}`;
          const contentProtected = Boolean(settings.streamer_content_protection_enabled);
          const signature = `${shouldShow}:${context}:${contentProtected}`;
          if (signature !== lastSync.current) {
            await invoke("set_league_content_protection", { enabled: contentProtected });
            await invoke("sync_league_mini", { shouldShow, context });
            lastSync.current = signature;
          }
          const ongoingShouldShow = Boolean(
            current.connected
            && ONGOING_AUTO_PHASES.has(current.phase),
          );
          const ongoingContext = `${current.connected ? "connected" : "offline"}:${current.phase || "None"}:${current.game_mode || "unknown"}`;
          const ongoingSignature = `${ongoingShouldShow}:${ongoingContext}:${contentProtected}`;
          if (ongoingSignature !== lastOngoingSync.current) {
            await invoke("sync_league_ongoing", {
              shouldShow: ongoingShouldShow,
              context: ongoingContext,
            });
            lastOngoingSync.current = ongoingSignature;
          }
          const cooldownShouldShow = Boolean(
            settings.cooldown_timer_enabled
            && COOLDOWN_AUTO_PHASES.has(current.phase)
            && current.cooldown_timer_should_show,
          );
          const cooldownContext = `${current.connected ? "connected" : "offline"}:${current.phase || "None"}:${current.game_mode || "unknown"}`;
          const cooldownSignature = `${cooldownShouldShow}:${cooldownContext}:${contentProtected}`;
          if (cooldownSignature !== lastCooldownSync.current) {
            await invoke("sync_league_cd_timer", { shouldShow: cooldownShouldShow, context: cooldownContext });
            lastCooldownSync.current = cooldownSignature;
          }
        }
      } catch {
        // Backend startup and shutdown races are expected; the next poll retries.
      } finally {
        syncing.current = false;
      }
    };
    const unsubscribe = subscribeLeagueLabStatus((status) => { void sync(status); });
    return () => { disposed = true; pendingStatus.current = undefined; unsubscribe(); };
  }, []);

  return null;
}
