import { useCallback, useEffect, useRef, useState } from "react";
import { Activity, AlertTriangle, History, Keyboard, ListChecks, Radio, RotateCcw, ShieldAlert, ShieldCheck, Square, Wrench } from "lucide-react";
import { createInputAnalysis, fetchInputHistory } from "../api/trainingApi";
import { useT } from "../i18n/useT.js";

const TRACKED_CODES = ["KeyW", "KeyA", "KeyS", "KeyD", "ShiftLeft", "ControlLeft", "Space"];
const KEY_LABELS = { KeyW: "W", KeyA: "A", KeyS: "S", KeyD: "D", ShiftLeft: "Shift", ControlLeft: "Ctrl", Space: "Space" };
const COUNTDOWN_MS = 3_000;
const DURATION_OPTIONS = [15_000, 30_000, 60_000, 0];

function SettingField({ label, value, onChange, min = 0.05, max = 4, step = 0.05 }) {
  return (
    <label>
      <span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{label}</span>
      <div className="flex items-center rounded-lg border border-cs2-border bg-cs2-bg-input focus-within:border-cs2-accent">
        <input type="number" value={value} min={min} max={max} step={step} onChange={(event) => onChange(Number(event.target.value))} className="min-w-0 flex-1 bg-transparent px-3 py-2.5 font-mono text-sm text-cs2-text-primary outline-none" />
        <span className="pr-3 text-xs text-cs2-text-muted">mm</span>
      </div>
    </label>
  );
}

export default function MagneticInputLabPage() {
  const t = useT();
  const [setup, setSetup] = useState({ keyboard_name: "Magnetic keyboard", mode: "counter_strafe", actuation_mm: 1.0, rapid_trigger_press_mm: 0.2, rapid_trigger_release_mm: 0.2, duration_ms: 15_000 });
  const [phase, setPhase] = useState("setup");
  const [displayMs, setDisplayMs] = useState(15_000);
  const [pressed, setPressed] = useState(new Set());
  const [eventCount, setEventCount] = useState(0);
  const [result, setResult] = useState(null);
  const [history, setHistory] = useState([]);
  const [error, setError] = useState("");
  const runtimeRef = useRef({ events: [], frame: 0, startAt: 0, endAt: 0, countdownEnd: 0, finished: false });

  const loadHistory = useCallback(() => {
    fetchInputHistory(12).then((data) => setHistory(data.items || [])).catch(() => setHistory([]));
  }, []);

  useEffect(loadHistory, [loadHistory]);

  const finish = useCallback(async () => {
    const runtime = runtimeRef.current;
    if (runtime.finished) return;
    runtime.finished = true;
    cancelAnimationFrame(runtime.frame);
    setPressed(new Set());
    if (runtime.events.length < 2) {
      setError(t("inputLab.notEnoughEvents"));
      setPhase("setup");
      return;
    }
    setPhase("analyzing");
    try {
      const durationMs = Math.max(3_000, Math.round(performance.now() - runtime.startAt));
      const analysis = await createInputAnalysis({
        ...setup,
        duration_ms: durationMs,
        events: runtime.events,
      });
      setResult(analysis);
      setPhase("result");
      loadHistory();
    } catch (requestError) {
      setError(requestError?.response?.data?.detail || t("inputLab.analyzeFail"));
      setPhase("setup");
    }
  }, [loadHistory, setup, t]);

  useEffect(() => {
    if (phase !== "countdown" && phase !== "running") return undefined;
    function tick(now) {
      const runtime = runtimeRef.current;
      if (phase === "countdown") {
        const remaining = runtime.countdownEnd - now;
        setDisplayMs(Math.max(0, remaining));
        if (remaining <= 0) {
          runtime.startAt = now;
          runtime.endAt = setup.duration_ms > 0 ? now + setup.duration_ms : null;
          setDisplayMs(setup.duration_ms > 0 ? setup.duration_ms : 0);
          setPhase("running");
          return;
        }
      } else {
        const display = runtime.endAt == null ? now - runtime.startAt : Math.max(0, runtime.endAt - now);
        setDisplayMs(display);
        if (runtime.endAt != null && display <= 0) {
          void finish();
          return;
        }
      }
      runtime.frame = requestAnimationFrame(tick);
    }
    runtimeRef.current.frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(runtimeRef.current.frame);
  }, [finish, phase, setup.duration_ms]);

  useEffect(() => {
    if (phase !== "running") return undefined;
    function record(event, eventType) {
      if (!TRACKED_CODES.includes(event.code)) return;
      event.preventDefault();
      if (eventType === "down" && event.repeat) return;
      const timestamp = Math.max(0, performance.now() - runtimeRef.current.startAt);
      runtimeRef.current.events.push({ code: event.code, event_type: eventType, timestamp_ms: timestamp });
      setEventCount(runtimeRef.current.events.length);
      if (runtimeRef.current.events.length >= 20_000) {
        void finish();
        return;
      }
      setPressed((current) => {
        const next = new Set(current);
        if (eventType === "down") next.add(event.code); else next.delete(event.code);
        return next;
      });
    }
    const onDown = (event) => record(event, "down");
    const onUp = (event) => record(event, "up");
    window.addEventListener("keydown", onDown, { passive: false });
    window.addEventListener("keyup", onUp, { passive: false });
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
    };
  }, [finish, phase]);

  function begin() {
    if (!setup.keyboard_name.trim()) return;
    setError("");
    setResult(null);
    setEventCount(0);
    setPressed(new Set());
    const now = performance.now();
    runtimeRef.current = { events: [], frame: 0, startAt: 0, endAt: 0, countdownEnd: now + COUNTDOWN_MS, finished: false };
    setDisplayMs(COUNTDOWN_MS);
    setPhase("countdown");
  }

  const activeTest = phase === "countdown" || phase === "running" || phase === "analyzing";
  const modeHelp = setup.mode === "counter_strafe" ? t("inputLab.counterHelp") : setup.mode === "rapid_tap" ? t("inputLab.rapidHelp") : t("inputLab.gameplayHelp");

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-7">
      <div className="mx-auto flex max-w-6xl flex-col gap-5">
        <header>
          <div className="mb-2 inline-flex items-center gap-2 rounded-full border border-violet-400/25 bg-violet-400/10 px-2.5 py-1 text-[11px] font-bold uppercase tracking-[0.16em] text-violet-300"><Radio className="h-3.5 w-3.5" /> Magnetic Input Lab</div>
          <h1 className="text-2xl font-bold text-cs2-text-primary">{t("inputLab.pageTitle")}</h1>
          <p className="mt-1 max-w-3xl text-sm leading-6 text-cs2-text-secondary">{t("inputLab.pageSubtitle")}</p>
        </header>

        {!activeTest && phase !== "result" && (
          <div className="grid gap-5 lg:grid-cols-[1fr_0.9fr]">
            <section className="mgs-readable-content mgs-glass-card rounded-2xl border border-cs2-border bg-cs2-bg-card p-5">
              <div className="flex items-center gap-2"><Keyboard className="h-5 w-5 text-violet-300" /><h2 className="font-bold text-cs2-text-primary">{t("inputLab.setupTitle")}</h2></div>
              <label className="mt-4 block"><span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{t("inputLab.keyboardName")}</span><input value={setup.keyboard_name} onChange={(event) => setSetup((value) => ({ ...value, keyboard_name: event.target.value }))} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary outline-none focus:border-cs2-accent" /></label>
              <div className="mt-4 grid gap-4 sm:grid-cols-3">
                <SettingField label={t("inputLab.actuation")} value={setup.actuation_mm} onChange={(value) => setSetup((current) => ({ ...current, actuation_mm: value }))} />
                <SettingField label={t("inputLab.rtPress")} value={setup.rapid_trigger_press_mm} onChange={(value) => setSetup((current) => ({ ...current, rapid_trigger_press_mm: value }))} />
                <SettingField label={t("inputLab.rtRelease")} value={setup.rapid_trigger_release_mm} onChange={(value) => setSetup((current) => ({ ...current, rapid_trigger_release_mm: value }))} />
              </div>
              <label className="mt-4 block"><span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{t("inputLab.mode")}</span><select value={setup.mode} onChange={(event) => setSetup((current) => ({ ...current, mode: event.target.value }))} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary outline-none"><option value="counter_strafe">{t("inputLab.counterMode")}</option><option value="rapid_tap">{t("inputLab.rapidMode")}</option><option value="gameplay">{t("inputLab.gameplayMode")}</option></select></label>
              <label className="mt-4 block"><span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">{t("inputLab.duration")}</span><select value={setup.duration_ms} onChange={(event) => setSetup((current) => ({ ...current, duration_ms: Number(event.target.value) }))} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary outline-none">{DURATION_OPTIONS.map((value) => <option key={value} value={value}>{value === 0 ? t("inputLab.unlimitedManual") : `${value / 1000} s`}</option>)}</select></label>
              <div className="mt-4 rounded-xl border border-violet-400/20 bg-violet-400/[0.07] px-3.5 py-3 text-xs leading-5 text-violet-100">{modeHelp}</div>
              {error && <div className="mt-3 rounded-lg border border-cs2-fail/30 bg-cs2-fail/10 px-3 py-2 text-xs text-cs2-fail">{error}</div>}
              <button type="button" onClick={begin} className="mt-5 flex w-full items-center justify-center gap-2 rounded-xl bg-violet-400 px-4 py-3 text-sm font-bold text-black transition-transform duration-150 active:scale-[0.98]"><Activity className="h-4 w-4" />{t("inputLab.start")}</button>
            </section>
            <section className="mgs-readable-content rounded-2xl border border-amber-400/20 bg-amber-400/[0.05] p-5">
              <div className="flex items-center gap-2 text-amber-200"><AlertTriangle className="h-5 w-5" /><h2 className="font-bold">{t("inputLab.limitTitle")}</h2></div>
              <p className="mt-3 text-xs leading-6 text-amber-100/80">{t("inputLab.limitBody")}</p>
              <div className="mt-4 space-y-2 text-xs text-cs2-text-secondary"><p>• {t("inputLab.measure1")}</p><p>• {t("inputLab.measure2")}</p><p>• {t("inputLab.measure3")}</p></div>
            </section>
          </div>
        )}

        {activeTest && (
          <section className="mgs-readable-content rounded-2xl border border-violet-400/30 bg-cs2-bg-card p-6 text-center">
            <div className="text-xs font-bold uppercase tracking-[0.2em] text-violet-300">{phase === "countdown" ? t("inputLab.getReady") : phase === "analyzing" ? t("inputLab.analyzing") : t("inputLab.recording")}</div>
            <div className="mt-2 font-mono text-6xl font-bold tabular-nums text-cs2-text-primary">{phase === "analyzing" ? "…" : phase === "countdown" ? Math.max(1, Math.ceil(displayMs / 1000)) : setup.duration_ms === 0 ? `∞ · ${(displayMs / 1000).toFixed(1)}s` : `${(displayMs / 1000).toFixed(1)}s`}</div>
            <div className="mx-auto mt-7 grid max-w-2xl grid-cols-4 gap-3 sm:grid-cols-7">
              {TRACKED_CODES.map((code) => <div key={code} className={`flex h-16 items-center justify-center rounded-xl border font-mono text-sm font-bold transition-[transform,background-color,border-color] duration-100 ${pressed.has(code) ? "scale-[0.96] border-violet-300 bg-violet-400 text-black" : "border-cs2-border bg-black/20 text-cs2-text-secondary"}`}>{KEY_LABELS[code]}</div>)}
            </div>
            <p className="mt-5 text-sm text-cs2-text-secondary">{modeHelp}</p>
            <div className="mt-3 font-mono text-xs text-cs2-text-muted">{eventCount} events</div>
            {phase === "running" && setup.duration_ms === 0 && (
              <button type="button" disabled={displayMs < 3_000} onClick={() => void finish()} className="mx-auto mt-5 flex items-center gap-2 rounded-xl bg-violet-400 px-4 py-2.5 text-sm font-bold text-black transition-transform duration-150 active:scale-[0.98] disabled:opacity-40"><Square className="h-4 w-4 fill-current" />{t("inputLab.finish")}</button>
            )}
          </section>
        )}

        {phase === "result" && result && (
          <section className="mgs-readable-content rounded-2xl border border-emerald-400/25 bg-gradient-to-br from-emerald-400/[0.10] to-cs2-bg-card p-5">
            <div className="flex flex-wrap items-start justify-between gap-4"><div><div className="flex items-center gap-2 text-xs font-bold uppercase tracking-[0.18em] text-emerald-300"><ShieldCheck className="h-4 w-4" />{t("inputLab.resultTitle")}</div><div className="mt-2 font-mono text-5xl font-bold text-cs2-text-primary">{result.stability_score}<span className="text-xl text-cs2-text-muted">/100</span></div></div><button type="button" onClick={() => setPhase("setup")} className="flex items-center gap-2 rounded-lg border border-cs2-border px-3 py-2 text-xs font-bold text-cs2-text-secondary active:scale-[0.97]"><RotateCcw className="h-4 w-4" />{t("inputLab.testAgain")}</button></div>
            <div className="mt-5 grid gap-3 sm:grid-cols-5">{[[t("inputLab.totalPresses"), result.total_presses],[t("inputLab.pps"), result.presses_per_second],[t("inputLab.hold"), `${result.mean_hold_ms} ms`],[t("inputLab.transition"), result.mean_transition_ms == null ? "—" : `${result.mean_transition_ms} ms`],[t("inputLab.chatter"), result.chatter_count]].map(([label,value]) => <div key={label} className="rounded-xl border border-white/10 bg-black/15 px-3 py-3"><div className="text-[10px] font-bold uppercase tracking-wider text-cs2-text-muted">{label}</div><div className="mt-1 font-mono text-lg font-bold text-cs2-text-primary">{value}</div></div>)}</div>
            <div className="mt-4 grid gap-3 lg:grid-cols-[0.9fr_1.1fr]">
              <div className="rounded-xl border border-violet-300/20 bg-violet-300/[0.06] p-4">
                <div className="flex items-center gap-2 text-xs font-bold text-violet-200"><Wrench className="h-4 w-4" />{t("inputLab.optimization")}</div>
                <div className="mt-2 text-base font-bold text-cs2-text-primary">{result.diagnosis_label || result.recommendation}</div>
                <div className="mt-3 grid grid-cols-3 gap-2">
                  {[[t("inputLab.actuationShort"), result.recommended_actuation_mm],[t("inputLab.rtPressShort"), result.recommended_rt_press_mm],[t("inputLab.rtReleaseShort"), result.recommended_rt_release_mm]].map(([label,value]) => <div key={label} className="rounded-lg border border-white/10 bg-black/15 px-2.5 py-2"><div className="text-[9px] font-bold text-cs2-text-muted">{label}</div><div className="mt-1 font-mono text-sm font-bold text-violet-100">{value ?? "—"} mm</div></div>)}
                </div>
                <div className="mt-3 space-y-2">{(result.issues || []).map((item) => <p key={item} className="text-xs leading-5 text-cs2-text-secondary">• {item}</p>)}</div>
              </div>
              <div className="rounded-xl border border-emerald-300/20 bg-black/15 p-4">
                <div className="flex items-center gap-2 text-xs font-bold text-emerald-200"><ListChecks className="h-4 w-4" />{t("inputLab.adjustAndRetest")}</div>
                <div className="mt-3 space-y-2.5">{(result.action_plan || [result.recommendation]).map((item,index) => <div key={item} className="flex gap-2.5 text-xs leading-5 text-cs2-text-secondary"><span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-emerald-300/10 font-mono text-[10px] font-bold text-emerald-200">{index + 1}</span><span>{item}</span></div>)}</div>
              </div>
            </div>
            <div className="mt-3 rounded-xl border border-amber-300/20 bg-amber-300/[0.05] px-3.5 py-3">
              <div className="flex items-center gap-2 text-xs font-bold text-amber-200"><ShieldAlert className="h-4 w-4" />{t("inputLab.officialSafety")}</div>
              <div className="mt-2 space-y-1">{(result.safety_notes || []).map((item) => <p key={item} className="text-[11px] leading-5 text-amber-100/80">• {item}</p>)}</div>
            </div>
          </section>
        )}

        {history.length > 0 && <section className="mgs-glass-card rounded-2xl border border-cs2-border bg-cs2-bg-card p-5"><div className="flex items-center gap-2"><History className="h-4 w-4 text-cs2-text-muted" /><h2 className="text-sm font-bold text-cs2-text-primary">{t("inputLab.history")}</h2></div><div className="mt-3 grid gap-2 sm:grid-cols-2 lg:grid-cols-3">{history.map((item) => <div key={item.id} className="rounded-xl border border-cs2-border-subtle bg-black/10 p-3"><div className="flex items-center justify-between"><span className="truncate text-xs font-semibold text-cs2-text-primary">{item.keyboard_name}</span><span className="font-mono text-sm font-bold text-emerald-300">{item.stability_score}</span></div><div className="mt-1 text-[11px] text-cs2-text-muted">{item.actuation_mm} mm · RT {item.rapid_trigger_press_mm}/{item.rapid_trigger_release_mm} mm</div></div>)}</div></section>}
      </div>
    </div>
  );
}
