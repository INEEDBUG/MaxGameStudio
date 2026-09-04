import { AlertTriangle, Check, CircleHelp, Cpu, Loader2, RefreshCw, Rocket, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { desktopBridge, isDesktopApp } from "../desktop/desktopBridge.js";
import { useT } from "../i18n/useT.js";
import { getLeagueLabStatusSnapshot } from "../utils/leagueLabStatusSubscription.js";
import {
  clearHandledLeagueSession,
  launchLeagueRuntimeCoordinated,
  leagueClientSessionId,
} from "../utils/leagueRuntimeLaunchCoordinator.js";
import {
  clearLeagueStartupPreference,
  LEAGUE_STARTUP_MODES,
  readLeagueStartupPreference,
  writeLeagueStartupPreference,
} from "../utils/leagueStartupPreference.js";

const MODES = LEAGUE_STARTUP_MODES.map(({ id }) => id);

function formatWorkingSet(bytes) {
  if (bytes === null || bytes === undefined || bytes === "") return "--";
  const value = Number(bytes);
  return Number.isFinite(value) && value >= 0 ? `${(value / 1024 / 1024).toFixed(0)} MB` : "--";
}

function statusValue(status, key) {
  if (!status || typeof status !== "object") return undefined;
  if (status[key] !== undefined) return status[key];
  return status.embedded_runtime?.[key];
}

export default function LeagueRuntimePage() {
  const t = useT();
  const initialPreference = useRef(readLeagueStartupPreference());
  const [mode, setMode] = useState(() => initialPreference.current?.mode || "ask");
  const [remember, setRemember] = useState(() => initialPreference.current?.remembered ?? true);
  const [administrator, setAdministrator] = useState(() => initialPreference.current?.administrator === true);
  const [status, setStatus] = useState(null);
  const [loading, setLoading] = useState(isDesktopApp);
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState("");
  const refresh = useCallback(async () => {
    if (!isDesktopApp || !desktopBridge?.getLeagueRuntimeStatus) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setError("");
    try {
      const nextStatus = await desktopBridge.getLeagueRuntimeStatus();
      setStatus(nextStatus);
      if (nextStatus?.last_error) setError(String(nextStatus.last_error));
    }
    catch (cause) { setError(cause?.message || String(cause)); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const launch = useCallback(async (selectedMode) => {
    if (!isDesktopApp || !desktopBridge?.launchLeagueRuntime || !MODES.includes(selectedMode)) return;
    setLaunching(true);
    setError("");
    try {
      const sessionId = leagueClientSessionId(getLeagueLabStatusSnapshot()) || "*";
      const result = await launchLeagueRuntimeCoordinated(selectedMode, { force: true, sessionId, administrator });
      if (result?.reason === "in-flight" && (result.mode !== selectedMode || result.administrator !== administrator)) {
        throw new Error(t("leagueRuntime.launchInFlight"));
      }
      if (result?.launched === true) {
        writeLeagueStartupPreference(selectedMode, remember, globalThis.localStorage, { administrator });
      }
      await refresh();
    } catch (cause) {
      const message = cause?.message || String(cause);
      setError(message.includes("UAC_CANCELLED") ? t("leagueRuntime.adminCancelled") : message);
    }
    finally { setLaunching(false); }
  }, [administrator, refresh, remember, t]);

  const clearPreference = () => {
    clearLeagueStartupPreference();
    clearHandledLeagueSession();
    setMode("ask");
    setRemember(true);
    setAdministrator(false);
  };

  const available = statusValue(status, "available");
  const runtimeAvailable = available === true;
  const administratorAvailable = statusValue(status, "administrator_available") === true;
  const active = statusValue(status, "active");
  const expected = statusValue(status, "expected_runtime_memory_mb");
  const expectedMemoryMb = expected !== null && expected !== undefined && expected !== "" && Number.isFinite(Number(expected))
    ? Math.round(Number(expected))
    : 800;
  const modeLabels = useMemo(() => ({
    ask: [t("leagueRuntime.modeAsk"), t("leagueRuntime.modeAskDesc")],
    memory: [t("leagueRuntime.modeMemory"), t("leagueRuntime.modeMemoryDesc")],
    parallel: [t("leagueRuntime.modeParallel"), t("leagueRuntime.modeParallelDesc")],
  }), [t]);

  return <main className="mx-auto w-full max-w-5xl px-5 py-8 sm:px-8" aria-labelledby="league-runtime-title">
    <header className="mb-6 flex flex-wrap items-start justify-between gap-4">
      <div><p className="mb-2 text-[11px] font-bold uppercase tracking-[.18em] text-cs2-accent">MaxGameStudio</p><h1 id="league-runtime-title" className="text-2xl font-bold text-cs2-text-primary">{t("leagueRuntime.title")}</h1><p className="mt-2 text-sm text-cs2-text-secondary">{t("leagueRuntime.subtitle")}</p></div>
      <button type="button" onClick={refresh} disabled={loading || launching || !isDesktopApp} aria-label={t("leagueRuntime.refresh")} className="inline-flex items-center gap-2 rounded-xl border border-cs2-border px-3 py-2 text-xs text-cs2-text-secondary disabled:cursor-not-allowed disabled:opacity-50"><RefreshCw className="h-4 w-4" aria-hidden="true" />{t("leagueRuntime.refresh")}</button>
    </header>
    {!isDesktopApp ? <div role="status" className="mb-5 rounded-2xl border border-amber-500/40 bg-amber-500/10 p-4 text-sm text-cs2-text-primary"><AlertTriangle className="mr-2 inline h-4 w-4 text-amber-500" aria-hidden="true" />{t("leagueRuntime.desktopOnly")}</div> : null}
    {loading ? <div role="status" aria-busy="true" className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-8 text-center text-sm text-cs2-text-secondary"><Loader2 className="mx-auto mb-3 h-6 w-6 animate-spin" aria-hidden="true" />{t("leagueRuntime.loading")}</div> : null}
    {!loading && error ? <div role="alert" className="mb-5 rounded-2xl border border-red-500/40 bg-red-500/10 p-4 text-sm text-cs2-text-primary">{t("leagueRuntime.error")}: {error}</div> : null}
    {!loading && isDesktopApp && status && !runtimeAvailable ? <div role="alert" className="mb-5 rounded-2xl border border-amber-500/40 bg-amber-500/10 p-4 text-sm text-cs2-text-primary"><AlertTriangle className="mr-2 inline h-4 w-4 text-amber-500" aria-hidden="true" />{t("leagueRuntime.unavailable")}</div> : null}
    {!loading ? <>
      <section className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-5" aria-labelledby="league-runtime-mode-title">
        <div className="flex items-start gap-3"><span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl bg-cs2-accent/15 text-cs2-accent"><Rocket className="h-5 w-5" aria-hidden="true" /></span><div><h2 id="league-runtime-mode-title" className="font-semibold text-cs2-text-primary">{t("leagueRuntime.chooseMode")}</h2><p className="mt-1 text-xs leading-5 text-cs2-text-muted">{t("leagueRuntime.explicitLaunch")}</p></div></div>
        <div className="mt-5 grid gap-3 md:grid-cols-3" role="radiogroup" aria-label={t("leagueRuntime.chooseMode")}>{MODES.map((item) => <button key={item} type="button" role="radio" aria-checked={mode === item} onClick={() => setMode(item)} disabled={launching || !isDesktopApp || !runtimeAvailable} className={`rounded-xl border p-4 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${mode === item ? "border-cs2-accent bg-cs2-accent/10" : "border-cs2-border bg-cs2-bg-input/40 hover:border-cs2-accent/50"}`}><span className="flex items-center justify-between text-sm font-semibold text-cs2-text-primary">{modeLabels[item][0]}{mode === item ? <Check className="h-4 w-4 text-cs2-accent" aria-hidden="true" /> : null}</span><span className="mt-2 block text-xs leading-5 text-cs2-text-secondary">{modeLabels[item][1]}</span></button>)}</div>
        {mode === "parallel" ? <div role="note" className="mt-4 rounded-xl border border-amber-500/40 bg-amber-500/10 px-4 py-3 text-xs leading-5 text-cs2-text-primary"><AlertTriangle className="mr-2 inline h-4 w-4 text-amber-500" aria-hidden="true" />{t("leagueRuntime.parallelMemoryWarning", { memory: expectedMemoryMb })}</div> : null}
        <div className="mt-4 flex flex-wrap items-end justify-between gap-3"><div className="flex flex-col gap-3"><label className="inline-flex items-center gap-2 text-xs text-cs2-text-secondary"><input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} disabled={launching || !isDesktopApp} className="accent-cs2-accent" />{t("leagueRuntime.remember")}</label><div><label className="inline-flex items-center gap-2 text-xs text-cs2-text-secondary"><input type="checkbox" checked={administrator} onChange={(event) => setAdministrator(event.target.checked)} disabled={launching || !isDesktopApp || mode === "ask" || (!administratorAvailable && !administrator)} className="accent-cs2-accent" />{t("leagueRuntime.administrator")}</label><p className="mt-1 max-w-xl pl-5 text-[11px] leading-4 text-cs2-text-muted">{administratorAvailable ? t("leagueRuntime.administratorHint") : t("leagueRuntime.administratorUnavailable")}</p></div></div><div className="flex gap-2"><button type="button" onClick={clearPreference} disabled={launching} className="inline-flex items-center gap-1 rounded-xl border border-cs2-border px-3 py-2 text-xs text-cs2-text-secondary disabled:opacity-50"><RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />{t("leagueRuntime.clear")}</button><button type="button" onClick={() => void launch(mode)} disabled={launching || mode === "ask" || !isDesktopApp || !runtimeAvailable || (administrator && !administratorAvailable)} className="inline-flex items-center gap-2 rounded-xl bg-cs2-accent px-4 py-2 text-xs font-bold text-cs2-text-on-accent disabled:cursor-not-allowed disabled:opacity-50">{launching ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" /> : <Rocket className="h-4 w-4" aria-hidden="true" />}{launching ? t("leagueRuntime.launching") : t("leagueRuntime.launch")}</button></div></div>
      </section>
      <section className="mt-5 grid gap-4 md:grid-cols-2" aria-label={t("leagueRuntime.statusTitle")}>
        <div className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-5"><div className="flex items-center gap-2 text-sm font-semibold text-cs2-text-primary"><Cpu className="h-4 w-4 text-cs2-accent" aria-hidden="true" />{t("leagueRuntime.statusTitle")}</div><dl className="mt-4 grid grid-cols-2 gap-3 text-xs"><div><dt className="text-cs2-text-muted">{t("leagueRuntime.runtimeStatus")}</dt><dd className="mt-1 text-cs2-text-primary">{active === true ? t("leagueRuntime.active") : active === false ? t("leagueRuntime.stopped") : "--"}</dd></div><div><dt className="text-cs2-text-muted">{t("leagueRuntime.activePrivilege")}</dt><dd className="mt-1 text-cs2-text-primary">{active === true ? (statusValue(status, "administrator") === true ? t("leagueRuntime.privilegeAdministrator") : t("leagueRuntime.privilegeStandard")) : "--"}</dd></div><div><dt className="text-cs2-text-muted">{t("leagueRuntime.hostMemory")}</dt><dd className="mt-1 text-cs2-text-primary">{formatWorkingSet(statusValue(status, "host_working_set_bytes"))}</dd></div><div><dt className="text-cs2-text-muted">{t("leagueRuntime.expectedMemory")}</dt><dd className="mt-1 text-cs2-text-primary">{expected !== null && expected !== undefined && expected !== "" && Number.isFinite(Number(expected)) ? `${expectedMemoryMb} MB` : "--"}</dd></div></dl></div>
        <div className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-5 text-xs leading-5 text-cs2-text-secondary"><CircleHelp className="mr-2 inline h-4 w-4 text-cs2-accent" aria-hidden="true" />{t("leagueRuntime.safety")}</div>
      </section>
    </> : null}
  </main>;
}
