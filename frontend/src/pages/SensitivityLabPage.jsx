import { useCallback, useEffect, useRef, useState } from "react";
import { Activity, AlertTriangle, Check, Clipboard, Crosshair, Gauge, History, Lightbulb, ListChecks, Monitor, Mouse, RefreshCw, UserRound } from "lucide-react";
import SensitivityAimArena from "../components/training/SensitivityAimArena";
import { createSensitivityRecommendation, fetchLocalCs2Settings, fetchSensitivityHistory } from "../api/trainingApi";
import { SENSITIVITY_TRIAL_SCHEDULE } from "../utils/sensitivityLab";
import { useT } from "../i18n/useT.js";

const DEFAULT_SETUP = {
  dpi: 800,
  current_sensitivity: 1,
  m_yaw: 0.022,
  game_width: 1024,
  game_height: 1080,
  display_aspect: "16:9",
  scaling_mode: "stretched",
};

const DURATION_OPTIONS = [15_000, 30_000, 60_000, 0];

function NumberField({ label, value, onChange, min, max, step = 1, suffix }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{label}</span>
      <div className="flex items-center rounded-lg border border-cs2-border bg-cs2-bg-input focus-within:border-cs2-accent">
        <input
          type="number"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(event) => onChange(Number(event.target.value))}
          className="min-w-0 flex-1 bg-transparent px-3 py-2.5 font-mono text-sm text-cs2-text-primary outline-none"
        />
        {suffix && <span className="pr-3 text-xs text-cs2-text-muted">{suffix}</span>}
      </div>
    </label>
  );
}

export default function SensitivityLabPage() {
  const t = useT();
  const [setup, setSetup] = useState(DEFAULT_SETUP);
  const [roundDurationMs, setRoundDurationMs] = useState(15_000);
  const [testActive, setTestActive] = useState(false);
  const [trialIndex, setTrialIndex] = useState(0);
  const [trials, setTrials] = useState([]);
  const [result, setResult] = useState(null);
  const [history, setHistory] = useState([]);
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [copied, setCopied] = useState(false);
  const [localAccounts, setLocalAccounts] = useState([]);
  const [selectedAccountId, setSelectedAccountId] = useState("");
  const [localSettingsLoading, setLocalSettingsLoading] = useState(true);
  const [localSettingsError, setLocalSettingsError] = useState("");
  const [importedAccountId, setImportedAccountId] = useState("");
  const setupRef = useRef(null);
  const resultRef = useRef(null);

  const loadHistory = useCallback(() => {
    fetchSensitivityHistory(12)
      .then((data) => setHistory(data.items || []))
      .catch(() => setHistory([]));
  }, []);

  useEffect(loadHistory, [loadHistory]);

  const applyLocalSettings = useCallback((account) => {
    const detected = account?.settings;
    if (!detected) return;
    setSetup((current) => ({
      ...current,
      current_sensitivity: detected.current_sensitivity ?? current.current_sensitivity,
      m_yaw: detected.m_yaw ?? current.m_yaw,
      game_width: detected.game_width ?? current.game_width,
      game_height: detected.game_height ?? current.game_height,
      display_aspect: detected.display_aspect ?? current.display_aspect,
    }));
    setImportedAccountId(account.account_id);
  }, []);

  const loadLocalSettings = useCallback(async () => {
    setLocalSettingsLoading(true);
    setLocalSettingsError("");
    try {
      const data = await fetchLocalCs2Settings();
      const accounts = data.accounts || [];
      setLocalAccounts(accounts);
      const active = accounts.find((item) => item.account_id === data.active_account_id) || accounts[0];
      setSelectedAccountId(active?.account_id || "");
      if (active) applyLocalSettings(active);
    } catch {
      setLocalAccounts([]);
      setLocalSettingsError(t("training.localCfgReadFailed"));
    } finally {
      setLocalSettingsLoading(false);
    }
  }, [applyLocalSettings, t]);

  useEffect(() => {
    void loadLocalSettings();
  }, [loadLocalSettings]);

  useEffect(() => {
    if (!result || !resultRef.current) return;
    const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
    window.requestAnimationFrame(() => {
      resultRef.current?.scrollIntoView?.({ behavior: reduceMotion ? "auto" : "smooth", block: "start" });
    });
  }, [result]);

  const selectedLocalAccount = localAccounts.find((account) => account.account_id === selectedAccountId);

  function updateSetup(key, value) {
    setSetup((current) => ({ ...current, [key]: value }));
  }

  function beginTest() {
    if (setup.dpi < 100 || setup.current_sensitivity <= 0 || setup.m_yaw <= 0 || setup.game_width < 320 || setup.game_height < 240) {
      setError(t("training.invalidSetup"));
      return;
    }
    setError("");
    setResult(null);
    setTrials([]);
    setTrialIndex(0);
    setTestActive(true);
  }

  async function submitCompletedTrials(completedTrials) {
    setSubmitting(true);
    setError("");
    try {
      const recommendation = await createSensitivityRecommendation({ ...setup, trials: completedTrials });
      setResult(recommendation);
      setTestActive(false);
      loadHistory();
    } catch (requestError) {
      setError(requestError?.response?.data?.detail || t("training.recommendFail"));
      setTestActive(false);
    } finally {
      setSubmitting(false);
    }
  }

  function handleTrialComplete(trialResult) {
    const completed = [...trials, trialResult];
    setTrials(completed);
    if (trialIndex >= SENSITIVITY_TRIAL_SCHEDULE.length - 1) {
      void submitCompletedTrials(completed);
      return;
    }
    setTrialIndex((value) => value + 1);
  }

  async function copyCommand() {
    if (!result?.console_command) return;
    await navigator.clipboard.writeText(result.console_command);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  if (testActive) {
    const trial = SENSITIVITY_TRIAL_SCHEDULE[trialIndex];
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-5">
        <SensitivityAimArena
          key={`${trialIndex}-${trial.kind}-${trial.multiplier}`}
          trial={trial}
          setup={setup}
          index={trialIndex}
          total={SENSITIVITY_TRIAL_SCHEDULE.length}
          durationMs={roundDurationMs}
          onComplete={handleTrialComplete}
          onCancel={() => setTestActive(false)}
        />
      </div>
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-7">
      <div className="mx-auto flex max-w-6xl flex-col gap-5">
        <header className="flex flex-wrap items-end justify-between gap-4">
          <div>
            <div className="mb-2 inline-flex items-center gap-2 rounded-full border border-cs2-accent/25 bg-cs2-accent/10 px-2.5 py-1 text-[11px] font-bold uppercase tracking-[0.16em] text-cs2-accent">
              <Crosshair className="h-3.5 w-3.5" /> {t("training.badge")}
            </div>
            <h1 className="text-2xl font-bold tracking-tight text-cs2-text-primary">{t("training.pageTitle")}</h1>
            <p className="mt-1 max-w-3xl text-sm leading-6 text-cs2-text-secondary">{t("training.pageSubtitle")}</p>
          </div>
          <div className="rounded-xl border border-cs2-border bg-cs2-bg-card px-4 py-3 text-right">
            <div className="text-[10px] font-bold uppercase tracking-widest text-cs2-text-muted">{t("training.protocol")}</div>
            <div className="mt-1 font-mono text-sm font-bold text-cs2-text-primary">
              6 × {roundDurationMs === 0 ? t("training.unlimited") : `${roundDurationMs / 1000}s`} · Flick + Track
            </div>
          </div>
        </header>

        <div className="grid gap-5 lg:grid-cols-[1.05fr_0.95fr]">
          <section ref={setupRef} className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-5">
            <div className="flex items-center gap-2">
              <Mouse className="h-5 w-5 text-cs2-orange" />
              <h2 className="text-base font-bold text-cs2-text-primary">{t("training.setupTitle")}</h2>
            </div>
            <div className="mt-4 rounded-xl border border-cs2-accent/20 bg-cs2-accent/[0.06] p-3.5">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="flex min-w-0 items-start gap-2.5">
                  <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-cs2-accent/15 text-cs2-accent">
                    <UserRound className="h-4 w-4" />
                  </span>
                  <div className="min-w-0">
                    <div className="text-xs font-bold text-cs2-text-primary">{t("training.localCfgTitle")}</div>
                    <div className="mt-0.5 text-[11px] leading-4 text-cs2-text-muted">
                      {localSettingsLoading
                        ? t("training.localCfgReading")
                        : localAccounts.length
                          ? t("training.localCfgFound", { count: localAccounts.length })
                          : t("training.localCfgNotFound")}
                    </div>
                  </div>
                </div>
                <button type="button" onClick={() => void loadLocalSettings()} disabled={localSettingsLoading} className="inline-flex items-center gap-1.5 rounded-lg border border-cs2-border-subtle bg-cs2-bg-input px-2.5 py-1.5 text-[11px] font-semibold text-cs2-text-secondary transition-[background-color,transform] duration-150 hover:bg-cs2-bg-hover active:scale-[0.97] disabled:opacity-50">
                  <RefreshCw className={`h-3.5 w-3.5 ${localSettingsLoading ? "animate-spin" : ""}`} />
                  {t("training.localCfgRefresh")}
                </button>
              </div>
              {localAccounts.length > 0 && (
                <div className="mt-3 flex flex-col gap-2 sm:flex-row">
                  <select value={selectedAccountId} onChange={(event) => setSelectedAccountId(event.target.value)} className="min-w-0 flex-1 rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs text-cs2-text-primary outline-none focus:border-cs2-accent">
                    {localAccounts.map((account) => (
                      <option key={account.account_id} value={account.account_id}>
                        {account.persona_name || account.account_name || `Steam ${account.account_id}`}
                        {account.is_current
                          ? ` · ${t("training.localCfgCurrent")}`
                          : account.most_recent
                            ? ` · ${t("training.localCfgRecent")}`
                            : ""}
                      </option>
                    ))}
                  </select>
                  <button type="button" onClick={() => applyLocalSettings(localAccounts.find((account) => account.account_id === selectedAccountId))} className="rounded-lg bg-cs2-accent px-3.5 py-2 text-xs font-bold text-white transition-transform duration-150 active:scale-[0.97]">
                    {t("training.localCfgApply")}
                  </button>
                </div>
              )}
              {selectedLocalAccount && (
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-cs2-text-muted">
                  <span>SteamID64 ·•••• {String(selectedLocalAccount.steam_id64).slice(-4)}</span>
                  <span>{selectedLocalAccount.remember_password ? t("training.localCfgRemembered") : t("training.localCfgLoginRequired")}</span>
                  {selectedLocalAccount.settings?.m_yaw != null && <span>m_yaw {selectedLocalAccount.settings.m_yaw}</span>}
                  {selectedLocalAccount.settings?.zoom_sensitivity_ratio != null && <span>zoom {selectedLocalAccount.settings.zoom_sensitivity_ratio}</span>}
                </div>
              )}
              {importedAccountId && <div className="mt-2 text-[10px] leading-4 text-cs2-text-muted">{t("training.localCfgApplied")} · {t("training.localCfgDpiNote")}</div>}
              {localSettingsError && <div className="mt-2 text-[11px] text-cs2-fail">{localSettingsError}</div>}
            </div>
            <div className="mt-4 grid gap-4 sm:grid-cols-2">
              <NumberField label={t("training.dpi")} value={setup.dpi} min={100} max={32000} suffix="DPI" onChange={(value) => updateSetup("dpi", value)} />
              <NumberField label={t("training.currentSens")} value={setup.current_sensitivity} min={0.01} max={25} step={0.001} onChange={(value) => updateSetup("current_sensitivity", value)} />
              <NumberField label={t("training.mYaw")} value={setup.m_yaw} min={0.001} max={1} step={0.001} onChange={(value) => updateSetup("m_yaw", value)} />
              <NumberField label={t("training.gameWidth")} value={setup.game_width} min={320} max={16384} suffix="px" onChange={(value) => updateSetup("game_width", value)} />
              <NumberField label={t("training.gameHeight")} value={setup.game_height} min={240} max={16384} suffix="px" onChange={(value) => updateSetup("game_height", value)} />
              <label className="block">
                <span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{t("training.displayAspect")}</span>
                <select value={setup.display_aspect} onChange={(event) => updateSetup("display_aspect", event.target.value)} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary outline-none focus:border-cs2-accent">
                  {["16:9", "16:10", "4:3", "5:4", "other"].map((value) => <option key={value} value={value}>{value}</option>)}
                </select>
              </label>
              <label className="block">
                <span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{t("training.roundDuration")}</span>
                <select value={roundDurationMs} onChange={(event) => setRoundDurationMs(Number(event.target.value))} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary outline-none focus:border-cs2-accent">
                  {DURATION_OPTIONS.map((value) => <option key={value} value={value}>{value === 0 ? t("training.unlimitedManual") : `${value / 1000} s`}</option>)}
                </select>
              </label>
              <label className="block">
                <span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{t("training.scalingMode")}</span>
                <select value={setup.scaling_mode} onChange={(event) => updateSetup("scaling_mode", event.target.value)} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary outline-none focus:border-cs2-accent">
                  <option value="stretched">{t("training.scalingStretched")}</option>
                  <option value="black_bars">{t("training.scalingBars")}</option>
                  <option value="native">{t("training.scalingNative")}</option>
                </select>
              </label>
            </div>
            <div className="mt-4 rounded-xl border border-sky-400/20 bg-sky-400/[0.07] px-3.5 py-3 text-xs leading-5 text-sky-100">
              <Monitor className="mr-2 inline h-4 w-4 text-sky-300" />
              {t("training.resolutionNote")}
            </div>
            {error && <div className="mt-3 rounded-lg border border-cs2-fail/30 bg-cs2-fail/10 px-3 py-2 text-xs text-cs2-fail">{error}</div>}
            <button type="button" onClick={beginTest} disabled={submitting} className="mt-5 flex w-full items-center justify-center gap-2 rounded-xl bg-cs2-orange px-4 py-3 text-sm font-bold text-black transition-transform duration-150 active:scale-[0.98] disabled:opacity-50">
              <Activity className="h-4 w-4" /> {submitting ? t("training.calculating") : t("training.beginFullTest")}
            </button>
          </section>

          <section className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-5">
            <div className="flex items-center gap-2">
              <Gauge className="h-5 w-5 text-cs2-accent" />
              <h2 className="text-base font-bold text-cs2-text-primary">{t("training.howTitle")}</h2>
            </div>
            <div className="mt-4 space-y-3">
              {["training.step1", "training.step2", "training.step3"].map((key, index) => (
                <div key={key} className="flex gap-3 rounded-xl border border-cs2-border-subtle bg-black/10 p-3.5">
                  <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-cs2-accent/15 font-mono text-xs font-bold text-cs2-accent">{index + 1}</span>
                  <p className="text-xs leading-5 text-cs2-text-secondary">{t(key)}</p>
                </div>
              ))}
            </div>
            <div className="mt-4 rounded-xl border border-cs2-accent/20 bg-cs2-accent/[0.06] p-3.5">
              <div className="flex items-center gap-3">
                <span className="relative grid h-12 w-12 shrink-0 place-items-center rounded-full border border-cs2-orange/40 bg-cs2-orange/15">
                  <span className="h-6 w-6 rounded-full bg-cs2-orange shadow-[0_0_18px_rgba(255,154,61,0.35)]" />
                  <Crosshair className="absolute -bottom-1 -right-1 h-5 w-5 text-white" />
                </span>
                <div>
                  <div className="text-xs font-bold text-cs2-text-primary">{t("training.targetPreviewTitle")}</div>
                  <p className="mt-1 text-[11px] leading-5 text-cs2-text-muted">{t("training.targetPreviewBody")}</p>
                </div>
              </div>
            </div>
            <p className="mt-4 text-xs leading-5 text-cs2-text-muted">{t("training.scienceNote")}</p>
          </section>
        </div>

        {result && (
          <section ref={resultRef} className="scroll-mt-5 rounded-2xl border border-emerald-400/25 bg-gradient-to-br from-emerald-400/[0.10] to-cs2-bg-card p-5">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div>
                <div className="text-xs font-bold uppercase tracking-[0.18em] text-emerald-300">{t("training.resultTitle")}</div>
                <div className="mt-2 font-mono text-5xl font-bold tracking-tight text-white">{result.recommended_sensitivity}</div>
                <div className="mt-1 text-xs text-cs2-text-muted">CS2 sensitivity</div>
              </div>
              <div className="flex flex-wrap gap-2">
                <button type="button" onClick={() => { setResult(null); setupRef.current?.scrollIntoView?.({ block: "start" }); }} className="flex items-center gap-2 rounded-lg border border-cs2-border bg-black/10 px-3 py-2 text-xs font-bold text-cs2-text-secondary transition-[background-color,transform] duration-150 hover:bg-white/5 active:scale-[0.97]">
                  <RefreshCw className="h-4 w-4" />{t("training.adjustAndRetest")}
                </button>
                <button type="button" onClick={copyCommand} className="flex items-center gap-2 rounded-lg border border-emerald-300/30 bg-emerald-300/10 px-3 py-2 text-xs font-bold text-emerald-200 transition-transform duration-150 active:scale-[0.97]">
                  {copied ? <Check className="h-4 w-4" /> : <Clipboard className="h-4 w-4" />}
                  {copied ? t("training.copied") : result.console_command}
                </button>
              </div>
            </div>
            <div className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
              {[
                [t("training.edpi"), result.edpi],
                [t("training.cm360"), `${result.cm_per_360} cm`],
                [t("training.multiplier"), `×${result.multiplier}`],
                [t("training.confidence"), `${Math.round(result.confidence * 100)}%`],
                ["m_yaw", result.m_yaw],
              ].map(([label, value]) => (
                <div key={label} className="rounded-xl border border-white/10 bg-black/15 px-3.5 py-3">
                  <div className="text-[10px] font-bold uppercase tracking-wider text-cs2-text-muted">{label}</div>
                  <div className="mt-1 font-mono text-lg font-bold text-cs2-text-primary">{value}</div>
                </div>
              ))}
            </div>
            <div className="mt-4 grid gap-3 lg:grid-cols-[0.85fr_1.15fr]">
              <div className="rounded-xl border border-cs2-accent/25 bg-cs2-accent/[0.07] p-4">
                <div className="flex items-center gap-2 text-xs font-bold text-cs2-accent">
                  <AlertTriangle className="h-4 w-4" />{t("training.measuredDiagnosis")}
                </div>
                <div className="mt-2 text-base font-bold text-cs2-text-primary">{result.diagnosis_label || t("training.personalizedReady")}</div>
                {result.click_tendency_label && (
                  <div className="mt-2 rounded-lg border border-white/10 bg-black/15 px-3 py-2">
                    <div className="text-[10px] font-bold uppercase tracking-wider text-cs2-text-muted">{t("training.clickTendency")}</div>
                    <div className="mt-1 text-sm font-semibold text-cs2-text-primary">{result.click_tendency_label}</div>
                  </div>
                )}
                <div className="mt-2 font-mono text-xs text-cs2-text-secondary">
                  {t("training.adjustment")} {Number(result.adjustment_percent || 0) >= 0 ? "+" : ""}{result.adjustment_percent || 0}% · {t("training.retestRange")} {result.suggested_min ?? result.recommended_sensitivity}–{result.suggested_max ?? result.recommended_sensitivity}
                </div>
                <div className="mt-3 space-y-2">
                  {(result.insights || []).map((item) => <p key={item} className="text-xs leading-5 text-cs2-text-secondary">• {item}</p>)}
                </div>
                {(result.click_evidence || []).length > 0 && (
                  <details className="mt-3 rounded-lg border border-white/10 bg-black/10 px-3 py-2">
                    <summary className="cursor-pointer text-[11px] font-semibold text-cs2-text-muted">{t("training.clickEvidence")}</summary>
                    <div className="mt-2 space-y-1.5">
                      {result.click_evidence.map((item) => <p key={item} className="text-[11px] leading-5 text-cs2-text-secondary">• {item}</p>)}
                    </div>
                  </details>
                )}
              </div>
              <div className="rounded-xl border border-emerald-300/20 bg-black/15 p-4">
                <div className="flex items-center gap-2 text-xs font-bold text-emerald-200"><ListChecks className="h-4 w-4" />{t("training.howToAdjust")}</div>
                <div className="mt-3 space-y-2.5">
                  {(result.action_plan || []).map((item, index) => (
                    <div key={item} className="flex gap-2.5 text-xs leading-5 text-cs2-text-secondary"><span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-emerald-300/10 font-mono text-[10px] font-bold text-emerald-200">{index + 1}</span><span>{item}</span></div>
                  ))}
                </div>
              </div>
            </div>
            {result.methodology_note && <p className="mt-3 flex items-start gap-2 text-[11px] leading-5 text-cs2-text-muted"><Lightbulb className="mt-0.5 h-3.5 w-3.5 shrink-0" />{result.methodology_note}</p>}
            <p className="mt-4 text-xs leading-5 text-cs2-text-secondary">{result.resolution_context}</p>
          </section>
        )}

        {history.length > 0 && (
          <section className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-5">
            <div className="flex items-center gap-2"><History className="h-4 w-4 text-cs2-text-muted" /><h2 className="text-sm font-bold text-cs2-text-primary">{t("training.historyTitle")}</h2></div>
            <div className="mt-3 overflow-x-auto">
              <table className="w-full text-left text-xs">
                <thead className="text-cs2-text-muted"><tr><th className="pb-2">{t("training.date")}</th><th>{t("training.recommendation")}</th><th>eDPI</th><th>cm/360</th><th>{t("training.resolution")}</th></tr></thead>
                <tbody className="text-cs2-text-secondary">
                  {history.map((item) => <tr key={item.id} className="border-t border-cs2-border-subtle"><td className="py-2.5">{new Date(item.created_at).toLocaleString()}</td><td className="font-mono font-bold text-cs2-text-primary">{item.recommended_sensitivity}</td><td>{item.edpi}</td><td>{item.cm_per_360}</td><td>{item.game_width}×{item.game_height}</td></tr>)}
                </tbody>
              </table>
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
