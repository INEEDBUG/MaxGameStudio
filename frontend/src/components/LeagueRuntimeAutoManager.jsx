import { useEffect, useRef } from "react";
import { useLocation } from "react-router-dom";
import { isDesktopApp } from "../desktop/desktopBridge.js";
import { readLeagueStartupPreference } from "../utils/leagueStartupPreference.js";
import {
  clearHandledLeagueSession,
  launchLeagueRuntimeCoordinated,
  leagueClientSessionId,
} from "../utils/leagueRuntimeLaunchCoordinator.js";
import { subscribeLeagueLabStatus } from "../utils/leagueLabStatusSubscription.js";

const CONFIRMED_DISCONNECT_MS = 5_000;

export default function LeagueRuntimeAutoManager() {
  const location = useLocation();
  const pathnameRef = useRef(location.pathname);
  pathnameRef.current = location.pathname;

  useEffect(() => {
    if (!isDesktopApp) return undefined;
    let disposed = false;
    let disconnectTimer = null;
    let failedSessionId = "";

    const cancelDisconnectReset = () => {
      if (disconnectTimer !== null) window.clearTimeout(disconnectTimer);
      disconnectTimer = null;
    };

    const unsubscribe = subscribeLeagueLabStatus((status) => {
      if (disposed || !status) return;
      const sessionId = leagueClientSessionId(status);
      if (!sessionId) {
        if (status.connected === false && status.client_window_detected !== true && disconnectTimer === null) {
          disconnectTimer = window.setTimeout(() => {
            disconnectTimer = null;
            failedSessionId = "";
            clearHandledLeagueSession();
          }, CONFIRMED_DISCONNECT_MS);
        }
        return;
      }
      cancelDisconnectReset();
      if (pathnameRef.current === "/league" || pathnameRef.current.startsWith("/league/")) return;
      const preference = readLeagueStartupPreference();
      if (!preference?.remembered || !["memory", "parallel"].includes(preference.mode) || failedSessionId === sessionId) return;
      const launchOptions = { sessionId };
      if (preference.administrator === true) launchOptions.administrator = true;
      void launchLeagueRuntimeCoordinated(preference.mode, launchOptions)
        .catch(() => { failedSessionId = sessionId; });
    });

    return () => {
      disposed = true;
      cancelDisconnectReset();
      unsubscribe();
    };
  }, []);

  return null;
}
