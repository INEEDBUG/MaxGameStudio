import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  Clipboard,
  Code2,
  Download,
  ExternalLink,
  Gamepad2,
  Gauge,
  HardDriveDownload,
  Monitor,
  RefreshCw,
  Save,
  ScanSearch,
  ShieldAlert,
  SlidersHorizontal,
  Upload,
} from "lucide-react";
import {
  applyValorantStretch,
  confirmValorantStretch,
  decodeValorantCrosshair,
  encodeValorantCrosshair,
  fetchValorantCrosshair,
  fetchValorantDisplayStatus,
  isValorantLabApiUnavailable,
  openValorantDeviceManager,
  prepareValorantStretch,
  restoreValorantStretch,
  saveValorantCrosshair,
} from "../api/valorantLabApi";
import { useT } from "../i18n/useT.js";
import {
  CROSSHAIR_COLORS,
  DEFAULT_CROSSHAIR_PROFILES,
  DEFAULT_DISPLAY_STATUS,
  VALORANT_RESOLUTION_PRESETS,
  getResolutionLabel,
  isDisplayStatusReady,
  normalizeCrosshairProfiles,
  normalizeDisplayStatus,
  parseCrosshairCode,
  resolutionFromSelection,
  serializeCrosshairCode,
} from "../utils/valorantLab";

const LOCAL_CROSSHAIR_KEY = "maxgamestudio.valorant.crosshair.v1";
const PROFILE_KEYS = ["P", "A", "S"];
const STATUS_CLASS = {
  ready: "border-emerald-400/25 bg-emerald-400/10 text-emerald-200",
  warning: "border-amber-400/25 bg-amber-400/10 text-amber-200",
  error: "border-rose-400/25 bg-rose-400/10 text-rose-200",
  unknown: "border-cs2-border-subtle bg-cs2-bg-input text-cs2-text-muted",
};

function cn(...values) {
  return values.filter(Boolean).join(" ");
}

function readLocalCrosshair() {
  try {
    const value = window.localStorage.getItem(LOCAL_CROSSHAIR_KEY);
    if (!value) return null;
    const parsed = JSON.parse(value);
    const source = parsed?.profiles && typeof parsed.profiles === "object" ? parsed.profiles : parsed;
    return {
      profiles: normalizeCrosshairProfiles(source),
      code: typeof parsed?.code === "string" ? parsed.code.trim() : "",
      lossWarning: parsed?.lossWarning === true,
    };
  } catch {
    return null;
  }
}

function writeLocalCrosshair(profiles, code, lossWarning = false) {
  try {
    window.localStorage.setItem(
      LOCAL_CROSSHAIR_KEY,
      JSON.stringify({
        version: 2,
        profiles: normalizeCrosshairProfiles(profiles),
        code: String(code || "").trim(),
        lossWarning: lossWarning === true,
      }),
    );
    return true;
  } catch {
    return false;
  }
}

function parseLocalNativeCode(text) {
  const sourceCode = String(text || "").trim();
  const profiles = parseCrosshairCode(sourceCode);
  const normalizedProfiles = normalizeCrosshairProfiles(profiles);
  // The browser fallback only exposes the small editor schema. Keep the
  // imported text intact for copy/export, and warn if the local serializer
  // cannot reproduce it after an edit (for example, future native fields).
  const localRoundTrip = serializeCrosshairCode(normalizedProfiles);
  return {
    profiles: normalizedProfiles,
    code: sourceCode,
    lossWarning: localRoundTrip !== sourceCode,
  };
}

function crosshairResponseProfiles(value) {
  const raw = value && typeof value === "object" ? value : {};
  const source = raw.profiles && typeof raw.profiles === "object" ? raw.profiles : raw;
  return normalizeCrosshairProfiles(source);
}

function crosshairResponseCode(value) {
  return value && typeof value.code === "string" ? value.code.trim() : "";
}

function StatusBadge({ status = "unknown", children }) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 rounded-full border px-2 py-1 text-[10px] font-semibold", STATUS_CLASS[status] || STATUS_CLASS.unknown)}>
      <span className={cn("h-1.5 w-1.5 rounded-full", status === "ready" ? "bg-emerald-300" : status === "warning" ? "bg-amber-300" : status === "error" ? "bg-rose-300" : "bg-cs2-text-muted")} />
      {children}
    </span>
  );
}

function NumberInput({ label, value, min, max, step = 1, suffix, onChange }) {
  return (
    <label className="block min-w-0">
      <span className="mb-1.5 block text-[11px] font-semibold text-cs2-text-secondary">{label}</span>
      <div className="flex items-center rounded-lg border border-cs2-border bg-cs2-bg-input transition-colors duration-150 focus-within:border-cs2-accent">
        <input aria-label={label} type="number" min={min} max={max} step={step} value={value} onChange={(event) => onChange(event.target.value)} className="min-w-0 flex-1 bg-transparent px-2.5 py-2 font-mono text-xs text-cs2-text-primary outline-none" />
        {suffix ? <span className="pr-2.5 text-[10px] text-cs2-text-muted">{suffix}</span> : null}
      </div>
    </label>
  );
}

function Toggle({ label, checked, onChange, hint }) {
  return (
    <label className="flex cursor-pointer items-start gap-2.5 rounded-lg border border-cs2-border-subtle bg-black/10 px-2.5 py-2 transition-colors duration-150 hover:bg-cs2-bg-hover">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} className="mt-0.5 accent-[var(--cs2-accent)]" />
      <span className="min-w-0">
        <span className="block text-[11px] font-semibold text-cs2-text-primary">{label}</span>
        {hint ? <span className="mt-0.5 block text-[10px] leading-4 text-cs2-text-muted">{hint}</span> : null}
      </span>
    </label>
  );
}

function RangeInput({ label, value, min, max, step = 1, onChange }) {
  return (
    <label className="block">
      <span className="mb-1 flex items-center justify-between gap-2 text-[10px] font-semibold text-cs2-text-secondary">
        <span>{label}</span>
        <span className="font-mono tabular-nums text-cs2-text-muted">{Number(value).toFixed(step < 1 ? 2 : 0)}</span>
      </span>
      <input aria-label={label} type="range" min={min} max={max} step={step} value={value} onChange={(event) => onChange(Number(event.target.value))} className="h-1.5 w-full cursor-pointer accent-[var(--cs2-accent)]" />
    </label>
  );
}

function CrosshairArm({ className, style }) {
  return <span aria-hidden="true" className={cn("absolute rounded-[1px]", className)} style={style} />;
}

function CrosshairPreview({ config, t }) {
  const color = CROSSHAIR_COLORS[config.color] || CROSSHAIR_COLORS.green;
  const outline = config.outlines ? String(Math.max(1, Number(config.outlineThickness) + 1)) + "px solid rgba(0,0,0," + config.outlineOpacity + ")" : "none";
  const innerLength = Number(config.innerLinesLength);
  const innerThickness = Number(config.innerLinesThickness);
  const innerOffset = Number(config.innerLinesOffset);
  const outerLength = Number(config.outerLinesLength);
  const outerThickness = Number(config.outerLinesThickness);
  const outerOffset = Number(config.outerLinesOffset);
  const arm = (length, thickness, offset, opacity, side) => ({
    backgroundColor: color,
    opacity,
    boxShadow: outline,
    width: side === "horizontal" ? length : thickness,
    height: side === "horizontal" ? thickness : length,
    ...(side === "left" ? { marginRight: offset } : {}),
    ...(side === "right" ? { marginLeft: offset } : {}),
    ...(side === "top" ? { marginBottom: offset } : {}),
    ...(side === "bottom" ? { marginTop: offset } : {}),
  });
  const armDirections = [
    { className: "right-1/2 top-1/2 -translate-y-1/2", side: "left" },
    { className: "left-1/2 top-1/2 -translate-y-1/2", side: "right" },
    { className: "left-1/2 bottom-1/2 -translate-x-1/2", side: "top" },
    { className: "left-1/2 top-1/2 -translate-x-1/2", side: "bottom" },
  ];
  return (
    <div className="relative flex aspect-[16/9] min-h-[190px] items-center justify-center overflow-hidden rounded-xl border border-cs2-border-subtle bg-[#15191f]" aria-label={t("valorant.crosshair.previewAria")} style={{ backgroundImage: "linear-gradient(rgba(255,255,255,.035) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,.035) 1px, transparent 1px), radial-gradient(circle at center, rgba(10,132,255,.12), transparent 52%)", backgroundSize: "28px 28px, 28px 28px, auto" }}>
      <div className="absolute inset-x-0 top-3 flex justify-center text-[9px] font-semibold uppercase tracking-[0.18em] text-white/30">{t("valorant.crosshair.previewLabel")}</div>
      <div className="relative h-24 w-24" aria-hidden="true">
        {config.innerLines ? armDirections.map((item, index) => <CrosshairArm key={"inner-" + index} className={item.className} style={arm(innerLength, innerThickness, innerOffset, config.innerLinesOpacity, item.side === "left" || item.side === "right" ? "horizontal" : item.side)} />) : null}
        {config.outerLines ? armDirections.map((item, index) => <CrosshairArm key={"outer-" + index} className={item.className} style={arm(outerLength, outerThickness, outerOffset + innerOffset + innerLength, config.outerLinesOpacity, item.side === "left" || item.side === "right" ? "horizontal" : item.side)} />) : null}
        {config.centerDot ? <span aria-hidden="true" className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-full" style={{ width: config.centerDotThickness * 2, height: config.centerDotThickness * 2, backgroundColor: color, opacity: config.centerDotOpacity, boxShadow: outline }} /> : null}
        {config.firingError ? <span className="absolute inset-[-10px] rounded-full border border-dashed border-amber-300/70" /> : null}
        {config.movementError ? <span className="absolute inset-[-17px] rounded-full border border-dashed border-cyan-300/50" /> : null}
      </div>
      <div className="absolute inset-x-0 bottom-3 flex justify-center text-[10px] text-white/45">{t("valorant.crosshair.previewHint")}</div>
    </div>
  );
}

function DisplayStatusCard({ icon: Icon, label, detail, status, statusLabel }) {
  return (
    <div className="flex min-w-0 items-center gap-2.5 rounded-xl border border-cs2-border-subtle bg-black/10 px-3 py-2.5">
      <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-cs2-bg-input text-cs2-text-secondary"><Icon className="h-4 w-4" /></span>
      <div className="min-w-0 flex-1"><div className="truncate text-[10px] font-bold uppercase tracking-[0.12em] text-cs2-text-muted">{label}</div><div className="mt-0.5 truncate text-xs font-semibold text-cs2-text-primary">{detail || "—"}</div></div>
      <StatusBadge status={status}>{statusLabel}</StatusBadge>
    </div>
  );
}

function StretchWizard({ t }) {
  const [selection, setSelection] = useState("1568x1080");
  const [custom, setCustom] = useState({ width: 1568, height: 1080 });
  const [displayStatus, setDisplayStatus] = useState(DEFAULT_DISPLAY_STATUS);
  const [detecting, setDetecting] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [applying, setApplying] = useState(false);
  const [prepared, setPrepared] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [rollbackDeadline, setRollbackDeadline] = useState(null);
  const [remainingSeconds, setRemainingSeconds] = useState(null);
  const [openingDeviceManager, setOpeningDeviceManager] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const resolution = useMemo(() => resolutionFromSelection(selection, custom), [selection, custom]);
  const statusReady = isDisplayStatusReady(displayStatus);
  const statusLabel = (status) => t("valorant.status." + (status || "unknown"));

  const detect = useCallback(async () => {
    setDetecting(true);
    setNotice("");
    setError("");
    try {
      setDisplayStatus(normalizeDisplayStatus(await fetchValorantDisplayStatus()));
    } catch (requestError) {
      setDisplayStatus(DEFAULT_DISPLAY_STATUS);
      setError(isValorantLabApiUnavailable(requestError) ? t("valorant.apiUnavailable") : t("valorant.detectFailed"));
    } finally {
      setDetecting(false);
    }
  }, [t]);

  useEffect(() => {
    void detect();
  }, [detect]);

  useEffect(() => {
    setPrepared(false);
    setConfirmed(false);
  }, [selection, custom.width, custom.height]);

  useEffect(() => {
    if (!rollbackDeadline) {
      setRemainingSeconds(null);
      return undefined;
    }
    const update = () => {
      const remaining = Math.max(0, Math.ceil(Number(rollbackDeadline) - Date.now() / 1000));
      setRemainingSeconds(remaining);
      if (remaining === 0) setRollbackDeadline(null);
    };
    update();
    const timer = window.setInterval(update, 250);
    return () => window.clearInterval(timer);
  }, [rollbackDeadline]);

  async function prepare() {
    if (!statusReady) {
      setError(t("valorant.unknownCannotExecute"));
      return;
    }
    setPreparing(true);
    setNotice("");
    setError("");
    try {
      await prepareValorantStretch({ width: resolution.width, height: resolution.height, preset: resolution.preset, mode: "real-stretched" });
      setPrepared(true);
      setNotice(t("valorant.prepareSuccess"));
    } catch (requestError) {
      setPrepared(false);
      setError(isValorantLabApiUnavailable(requestError) ? t("valorant.apiUnavailable") : t("valorant.prepareFailed"));
    } finally {
      setPreparing(false);
    }
  }

  async function apply() {
    if (!statusReady || !prepared || !confirmed) {
      setError(t("valorant.applyGuard"));
      return;
    }
    setApplying(true);
    setNotice("");
    setError("");
    try {
      const result = await applyValorantStretch({ width: resolution.width, height: resolution.height, preset: resolution.preset, mode: "real-stretched", confirmed: true, timeout_seconds: 20 });
      setRollbackDeadline(result?.rollback_deadline || null);
      setNotice(t("valorant.applyPending"));
      setConfirmed(false);
    } catch (requestError) {
      setError(isValorantLabApiUnavailable(requestError) ? t("valorant.apiUnavailable") : t("valorant.applyFailed"));
    } finally {
      setApplying(false);
    }
  }

  async function confirmAppliedMode() {
    setApplying(true);
    setError("");
    try {
      await confirmValorantStretch();
      setRollbackDeadline(null);
      setNotice(t("valorant.applySuccess"));
    } catch (requestError) {
      setError(isValorantLabApiUnavailable(requestError) ? t("valorant.apiUnavailable") : t("valorant.applyFailed"));
    } finally {
      setApplying(false);
    }
  }

  async function restoreAppliedMode() {
    setApplying(true);
    setError("");
    try {
      await restoreValorantStretch();
      setRollbackDeadline(null);
      setNotice(t("valorant.restoreSuccess"));
    } catch (requestError) {
      setError(isValorantLabApiUnavailable(requestError) ? t("valorant.apiUnavailable") : t("valorant.applyFailed"));
    } finally {
      setApplying(false);
    }
  }

  async function openDeviceManager() {
    setOpeningDeviceManager(true);
    setError("");
    try {
      await openValorantDeviceManager();
      setNotice(t("valorant.deviceManagerOpened"));
    } catch (requestError) {
      setError(isValorantLabApiUnavailable(requestError) ? t("valorant.apiUnavailable") : t("valorant.deviceManagerFailed"));
    } finally {
      setOpeningDeviceManager(false);
    }
  }

  const monitorStepSkipped = displayStatus.safeToSkipDisable || displayStatus.rawMonitorStatus === "all_present_physical_monitors_disabled";

  return (
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-4 shadow-[var(--cs2-shadow-sm)] sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2.5"><span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-cs2-accent-soft text-cs2-accent"><ScanSearch className="h-4.5 w-4.5" /></span><div><div className="flex flex-wrap items-center gap-2"><h2 className="text-base font-bold text-cs2-text-primary">{t("valorant.stretch.title")}</h2><span className="rounded-full border border-cs2-accent/25 bg-cs2-accent-soft px-2 py-0.5 text-[9px] font-bold uppercase tracking-[0.12em] text-cs2-accent">{t("valorant.stretch.badge")}</span></div><p className="mt-1 max-w-2xl text-xs leading-5 text-cs2-text-secondary">{t("valorant.stretch.subtitle")}</p></div></div>
        <StatusBadge status={statusReady ? "ready" : "unknown"}>{statusReady ? t("valorant.status.ready") : t("valorant.status.unknown")}</StatusBadge>
      </div>
      <div className="mt-4 grid gap-2 sm:grid-cols-3">
        <DisplayStatusCard icon={Gamepad2} label={t("valorant.detect.gpu")} detail={displayStatus.gpu.name || t("valorant.detect.notDetected")} status={displayStatus.gpu.status} statusLabel={statusLabel(displayStatus.gpu.status)} />
        <DisplayStatusCard icon={Monitor} label={t("valorant.detect.monitor")} detail={monitorStepSkipped ? t("valorant.detect.monitorSkipped") : displayStatus.monitor.name || t("valorant.detect.notDetected")} status={monitorStepSkipped ? "ready" : displayStatus.monitor.status} statusLabel={monitorStepSkipped ? t("valorant.detect.monitorSkippedStatus") : statusLabel(displayStatus.monitor.status)} />
        <DisplayStatusCard icon={Gauge} label={t("valorant.detect.refresh")} detail={displayStatus.refreshRate.value ? String(displayStatus.refreshRate.value) + " Hz" : t("valorant.detect.notDetected")} status={displayStatus.refreshRate.status} statusLabel={statusLabel(displayStatus.refreshRate.status)} />
      </div>
      <div className="mt-4 grid gap-4 lg:grid-cols-[1.08fr_0.92fr]">
        <div className="rounded-xl border border-cs2-border-subtle bg-black/10 p-3.5">
          <div className="flex items-center justify-between gap-3"><div><div className="text-xs font-bold text-cs2-text-primary">{t("valorant.stretch.resolutionTitle")}</div><div className="mt-0.5 text-[10px] text-cs2-text-muted">{t("valorant.stretch.resolutionHint")}</div></div><div className="font-mono text-sm font-bold text-cs2-accent">{getResolutionLabel(selection, custom)}</div></div>
          <div className="mt-3 grid gap-2 sm:grid-cols-2">
            {VALORANT_RESOLUTION_PRESETS.map((preset) => <button key={preset.id} type="button" aria-pressed={selection === preset.id} onClick={() => setSelection(preset.id)} className={cn("flex items-center justify-between rounded-lg border px-3 py-2.5 text-left transition-[background-color,border-color,transform] duration-150 active:scale-[0.98]", selection === preset.id ? "border-cs2-accent/55 bg-cs2-accent-soft text-cs2-text-primary" : "border-cs2-border-subtle bg-cs2-bg-input text-cs2-text-secondary hover:bg-cs2-bg-hover")}><span className="font-mono text-xs font-bold">{preset.label}</span>{preset.hintKey ? <span className="text-[9px] text-cs2-accent">{t(preset.hintKey)}</span> : null}</button>)}
            <button type="button" aria-pressed={selection === "custom"} onClick={() => setSelection("custom")} className={cn("flex items-center justify-between rounded-lg border px-3 py-2.5 text-left transition-[background-color,border-color,transform] duration-150 active:scale-[0.98]", selection === "custom" ? "border-cs2-accent/55 bg-cs2-accent-soft text-cs2-text-primary" : "border-cs2-border-subtle bg-cs2-bg-input text-cs2-text-secondary hover:bg-cs2-bg-hover")}><span className="text-xs font-bold">{t("valorant.stretch.custom")}</span><SlidersHorizontal className="h-3.5 w-3.5" /></button>
          </div>
          {selection === "custom" ? <div className="mt-3 grid gap-3 sm:grid-cols-2"><NumberInput label={t("valorant.stretch.width")} value={custom.width} min={320} max={7680} suffix="px" onChange={(value) => setCustom((current) => ({ ...current, width: value }))} /><NumberInput label={t("valorant.stretch.height")} value={custom.height} min={240} max={4320} suffix="px" onChange={(value) => setCustom((current) => ({ ...current, height: value }))} /></div> : null}
        </div>
        <div className="rounded-xl border border-amber-400/20 bg-amber-400/[0.05] p-3.5"><div className="flex items-center gap-2 text-xs font-bold text-amber-200"><ShieldAlert className="h-4 w-4" />{t("valorant.stretch.safetyTitle")}</div><p className="mt-2 text-[11px] leading-5 text-amber-100/75">{t("valorant.stretch.safetyBody")}</p><div className="mt-3 space-y-2 text-[10px] leading-4 text-cs2-text-secondary"><div className="flex gap-2"><span className="font-mono text-emerald-300">01</span><span>{t("valorant.stretch.stepDetect")}</span></div><div className="flex gap-2"><span className="font-mono text-sky-300">02</span><span>{t("valorant.stretch.stepPreview")}</span></div><div className="flex gap-2"><span className="font-mono text-amber-300">03</span><span>{t("valorant.stretch.stepApply")}</span></div></div></div>
      </div>
      <div className="mt-4 rounded-xl border border-cs2-accent/20 bg-cs2-accent-soft/30 p-3.5"><div className="flex flex-wrap items-center justify-between gap-2"><div><div className="text-xs font-bold text-cs2-text-primary">{t("valorant.stretch.workflowTitle")}</div><p className="mt-1 text-[10px] leading-4 text-cs2-text-secondary">{t("valorant.stretch.workflowSubtitle")}</p></div><span className="rounded-full border border-cs2-accent/25 bg-cs2-accent-soft px-2 py-1 text-[9px] font-bold text-cs2-accent">1568×1080</span></div><ol className="mt-3 grid gap-2 text-[10px] leading-4 text-cs2-text-secondary sm:grid-cols-2"><li className="rounded-lg border border-cs2-border-subtle bg-black/10 px-2.5 py-2"><span className="mr-1 font-mono text-cs2-accent">01</span>{t("valorant.stretch.workflowStep1")}</li><li className="rounded-lg border border-cs2-border-subtle bg-black/10 px-2.5 py-2"><span className="mr-1 font-mono text-cs2-accent">02</span>{t("valorant.stretch.workflowStep2")}</li><li className="rounded-lg border border-cs2-border-subtle bg-black/10 px-2.5 py-2"><span className="mr-1 font-mono text-cs2-accent">03</span>{t("valorant.stretch.workflowStep3")}</li><li className="rounded-lg border border-cs2-border-subtle bg-black/10 px-2.5 py-2"><span className="mr-1 font-mono text-cs2-accent">04</span>{t("valorant.stretch.workflowStep4")}</li></ol></div>
      {!monitorStepSkipped ? <div className="mt-3 flex flex-wrap items-center justify-between gap-2 rounded-xl border border-amber-400/20 bg-amber-400/[0.04] px-3.5 py-3"><span className="text-[10px] leading-4 text-amber-100/75">{t("valorant.monitorPrerequisiteHint")}</span><button type="button" onClick={() => void openDeviceManager()} disabled={openingDeviceManager} className="inline-flex items-center gap-1.5 rounded-lg border border-amber-300/30 bg-cs2-bg-input px-3 py-2 text-[11px] font-semibold text-amber-100 transition-[background-color,transform] duration-150 hover:bg-cs2-bg-hover active:scale-[0.97] disabled:cursor-wait disabled:opacity-60"><ExternalLink className="h-3.5 w-3.5" />{openingDeviceManager ? t("valorant.openingDeviceManager") : t("valorant.openDeviceManager")}</button></div> : <div role="status" className="mt-3 rounded-xl border border-emerald-300/20 bg-emerald-300/[0.06] px-3.5 py-3 text-[11px] text-emerald-200">{t("valorant.monitorSkipped")}</div>}
      <div className="mt-4 flex flex-wrap items-center justify-between gap-2 rounded-xl border border-cs2-border-subtle bg-black/10 px-3.5 py-3"><div className="flex min-w-0 items-center gap-2 text-[11px] text-cs2-text-muted"><span className={cn("h-2 w-2 shrink-0 rounded-full", statusReady ? "bg-emerald-300" : "bg-amber-300")} /><span>{statusReady ? t("valorant.readyToPreview") : t("valorant.unknownCannotExecute")}</span></div><button type="button" onClick={() => void detect()} disabled={detecting} className="inline-flex items-center gap-1.5 rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-[11px] font-semibold text-cs2-text-secondary transition-[background-color,transform] duration-150 hover:bg-cs2-bg-hover active:scale-[0.97] disabled:cursor-wait disabled:opacity-60"><RefreshCw className={cn("h-3.5 w-3.5", detecting && "motion-safe:animate-spin")} />{detecting ? t("valorant.detecting") : t("valorant.detectAgain")}</button></div>
      <div className="mt-3 grid gap-2 sm:grid-cols-2"><button type="button" onClick={() => void prepare()} disabled={!statusReady || preparing || applying} className="inline-flex items-center justify-center gap-2 rounded-xl border border-cs2-accent/35 bg-cs2-accent-soft px-3.5 py-3 text-xs font-bold text-cs2-accent transition-[background-color,transform] duration-150 hover:bg-cs2-accent/20 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-40"><HardDriveDownload className="h-4 w-4" />{preparing ? t("valorant.preparing") : t("valorant.preview")}</button><button type="button" onClick={() => void apply()} disabled={!statusReady || !prepared || !confirmed || applying} className="inline-flex items-center justify-center gap-2 rounded-xl border border-rose-400/35 bg-rose-400/[0.10] px-3.5 py-3 text-xs font-bold text-rose-200 transition-[background-color,transform] duration-150 hover:bg-rose-400/20 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-35"><ExternalLink className="h-4 w-4" />{applying ? t("valorant.applying") : t("valorant.apply")}</button></div>
      <label className={cn("mt-3 flex items-start gap-2.5 rounded-xl border px-3.5 py-3 transition-colors duration-150", prepared ? "border-rose-300/25 bg-rose-300/[0.06]" : "border-cs2-border-subtle bg-black/10 opacity-60")}><input type="checkbox" checked={confirmed} disabled={!prepared || !statusReady} onChange={(event) => setConfirmed(event.target.checked)} className="mt-0.5 accent-rose-400" /><span className="text-[11px] leading-5 text-cs2-text-secondary">{t("valorant.applyConfirm")}</span></label>
      {rollbackDeadline ? <div className="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-300/30 bg-amber-300/[0.08] p-3.5"><div><div className="text-xs font-bold text-amber-100">{t("valorant.rollbackTitle")}</div><div className="mt-1 text-[11px] text-amber-100/70">{t("valorant.rollbackBody")} <span className="font-mono font-bold text-amber-200">{remainingSeconds ?? 0}s</span></div></div><div className="flex gap-2"><button type="button" onClick={() => void restoreAppliedMode()} disabled={applying} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-[11px] font-semibold text-cs2-text-secondary active:scale-[0.97] disabled:opacity-50">{t("valorant.restoreNow")}</button><button type="button" onClick={() => void confirmAppliedMode()} disabled={applying} className="rounded-lg bg-emerald-500 px-3 py-2 text-[11px] font-bold text-white active:scale-[0.97] disabled:opacity-50">{t("valorant.keepMode")}</button></div></div> : null}
      {notice ? <div role="status" className="mt-3 rounded-lg border border-emerald-300/20 bg-emerald-300/[0.06] px-3 py-2 text-[11px] text-emerald-200">{notice}</div> : null}
      {error ? <div role="alert" className="mt-3 flex items-start gap-2 rounded-lg border border-amber-300/25 bg-amber-300/[0.06] px-3 py-2 text-[11px] leading-5 text-amber-100"><AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />{error}</div> : null}
    </section>
  );
}

function CrosshairEditor({ t }) {
  const [profiles, setProfiles] = useState(DEFAULT_CROSSHAIR_PROFILES);
  const [activeProfile, setActiveProfile] = useState("P");
  const [loading, setLoading] = useState(true);
  const [mode, setMode] = useState("loading");
  const [code, setCode] = useState("");
  const [sourceCode, setSourceCode] = useState("");
  const [codeDirty, setCodeDirty] = useState(false);
  const [lossWarning, setLossWarning] = useState(false);
  const [encoding, setEncoding] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const importInputRef = useRef(null);
  const config = profiles[activeProfile] || profiles.P;

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const remote = await fetchValorantCrosshair();
        if (cancelled) return;
        const remoteCode = crosshairResponseCode(remote);
        setProfiles(crosshairResponseProfiles(remote));
        setMode("backend");
        setCode(remoteCode);
        setSourceCode(remoteCode);
        setCodeDirty(false);
        setLossWarning(false);
        setError("");
      } catch (requestError) {
        if (cancelled) return;
        if (!isValorantLabApiUnavailable(requestError)) {
          setMode("backend");
          setError(t("valorant.crosshair.loadFailed"));
          return;
        }
        const local = readLocalCrosshair();
        const localProfiles = local?.profiles || DEFAULT_CROSSHAIR_PROFILES;
        const localCode = local?.code || serializeCrosshairCode(localProfiles);
        setProfiles(localProfiles);
        setMode("local");
        setCode(localCode);
        setSourceCode(localCode);
        setCodeDirty(false);
        setLossWarning(local?.lossWarning === true);
        setNotice(t("valorant.crosshair.localModeNotice"));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, [t]);

  // The backend is the only encoder in the online path. If a response did not
  // include a code, or a control changed, refresh the displayed code through
  // the strict endpoint instead of the browser serializer.
  useEffect(() => {
    if (mode !== "backend" || (code && !codeDirty)) return undefined;
    let cancelled = false;
    setEncoding(true);
    encodeValorantCrosshair(profiles)
      .then((response) => {
        const encodedCode = crosshairResponseCode(response);
        if (!encodedCode) throw new Error("missing-crosshair-code");
        if (cancelled) return;
        setCode(encodedCode);
        setSourceCode(encodedCode);
        setCodeDirty(false);
        setLossWarning(Boolean(sourceCode && sourceCode !== encodedCode));
        setError("");
      })
      .catch((requestError) => {
        if (cancelled) return;
        if (isValorantLabApiUnavailable(requestError)) {
          const localCode = serializeCrosshairCode(profiles);
          setMode("local");
          setCode(localCode);
          setSourceCode(localCode);
          setCodeDirty(false);
          setLossWarning(true);
          setNotice(t("valorant.crosshair.localModeNotice"));
        } else {
          setError(t("valorant.crosshair.encodeFailed"));
        }
      })
      .finally(() => {
        if (!cancelled) setEncoding(false);
      });
    return () => {
      cancelled = true;
    };
  }, [code, codeDirty, mode, profiles, t]);

  function updateConfig(key, value) {
    setProfiles((current) => {
      const next = { ...current, [activeProfile]: { ...current[activeProfile], [key]: value } };
      if (mode === "backend") {
        setCodeDirty(true);
      } else {
        const localCode = serializeCrosshairCode(next);
        setCode(localCode);
        setSourceCode(localCode);
        setCodeDirty(false);
      }
      return next;
    });
    if (sourceCode) setLossWarning(true);
    setNotice("");
    setError("");
  }

  async function getCodeForAction({ allowLocalFallback = true } = {}) {
    if (mode === "backend" && !codeDirty && code) return code;
    if (mode !== "backend") {
      const localCode = code || serializeCrosshairCode(profiles);
      setCode(localCode);
      setCodeDirty(false);
      return localCode;
    }
    try {
      const response = await encodeValorantCrosshair(profiles);
      const encodedCode = crosshairResponseCode(response);
      if (!encodedCode) throw new Error("missing-crosshair-code");
      setCode(encodedCode);
      setSourceCode(encodedCode);
      setCodeDirty(false);
      setLossWarning(Boolean(sourceCode && sourceCode !== encodedCode));
      return encodedCode;
    } catch (requestError) {
      if (!allowLocalFallback || !isValorantLabApiUnavailable(requestError)) throw requestError;
      const localCode = serializeCrosshairCode(profiles);
      setMode("local");
      setCode(localCode);
      setSourceCode(localCode);
      setCodeDirty(false);
      setLossWarning(true);
      setNotice(t("valorant.crosshair.localModeNotice"));
      return localCode;
    }
  }

  async function copyCode() {
    try {
      const currentCode = await getCodeForAction();
      await navigator.clipboard.writeText(currentCode);
      setNotice(t("valorant.crosshair.copied"));
    } catch {
      setError(t("valorant.crosshair.copyFailed"));
    }
  }

  async function exportProfile() {
    try {
      const currentCode = await getCodeForAction();
      const blob = new Blob([JSON.stringify({ version: 2, format: "valorant-native-v0", code: currentCode, profiles }, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = "valorant-crosshair-profiles.json";
      anchor.click();
      URL.revokeObjectURL(url);
      setNotice(t("valorant.crosshair.exported"));
    } catch {
      setError(t("valorant.crosshair.exportFailed"));
    }
  }

  function applyLocalPayload(payload) {
    const localProfiles = payload.profiles || DEFAULT_CROSSHAIR_PROFILES;
    const localCode = payload.code || serializeCrosshairCode(localProfiles);
    setProfiles(localProfiles);
    setMode("local");
    setCode(localCode);
    setSourceCode(localCode);
    setCodeDirty(false);
    setLossWarning(payload.lossWarning === true);
  }

  async function importNativeCode(text) {
    // Native share codes, including the code embedded in our JSON export,
    // always use the backend decoder whenever it is reachable.
    try {
      const remote = await decodeValorantCrosshair(text);
      const decodedCode = crosshairResponseCode(remote);
      if (!decodedCode) throw new Error("missing-crosshair-code");
      setProfiles(crosshairResponseProfiles(remote));
      setMode("backend");
      setCode(decodedCode);
      setSourceCode(decodedCode);
      setCodeDirty(false);
      setLossWarning(false);
    } catch (requestError) {
      if (!isValorantLabApiUnavailable(requestError)) throw requestError;
      applyLocalPayload(parseLocalNativeCode(text));
    }
  }

  async function importProfile(event) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    try {
      const text = (await file.text()).trim();
      if (!text) throw new Error("empty-crosshair");
      if (text.startsWith("{")) {
        const payload = JSON.parse(text);
        const exportedCode = typeof payload?.code === "string" ? payload.code.trim() : "";
        if (exportedCode) {
          await importNativeCode(exportedCode);
          setNotice(t("valorant.crosshair.imported"));
          setError("");
          return;
        }
        const source = payload?.profiles && typeof payload.profiles === "object" ? payload.profiles : payload;
        if (!source || typeof source !== "object" || !(source.P || source.p || source.profile)) throw new Error("invalid-crosshair-json");
        const importedProfiles = normalizeCrosshairProfiles(source);
        setProfiles(importedProfiles);
        setSourceCode("");
        setLossWarning(false);
        setNotice(t("valorant.crosshair.imported"));
        setError("");
        if (mode === "backend") {
          setCode("");
          setCodeDirty(true);
        } else {
          const localCode = serializeCrosshairCode(importedProfiles);
          setCode(localCode);
          setSourceCode(localCode);
          setCodeDirty(false);
        }
        return;
      }

      await importNativeCode(text);
      setNotice(t("valorant.crosshair.imported"));
      setError("");
    } catch {
      setError(t("valorant.crosshair.importFailed"));
    }
  }

  async function saveProfile() {
    setError("");
    if (mode !== "backend") {
      const localCode = code || serializeCrosshairCode(profiles);
      const savedLocally = writeLocalCrosshair(profiles, localCode, lossWarning);
      setCode(localCode);
      setSourceCode(localCode);
      setNotice(savedLocally ? t("valorant.crosshair.savedLocal") : t("valorant.crosshair.saveFailed"));
      return;
    }
    try {
      const response = await saveValorantCrosshair(profiles);
      let savedCode = crosshairResponseCode(response);
      if (!savedCode) savedCode = await getCodeForAction({ allowLocalFallback: false });
      setCode(savedCode);
      setSourceCode(savedCode);
      setCodeDirty(false);
      setLossWarning(false);
      writeLocalCrosshair(profiles, savedCode, false);
      setNotice(t("valorant.crosshair.saved"));
    } catch (requestError) {
      if (isValorantLabApiUnavailable(requestError)) {
        const localCode = serializeCrosshairCode(profiles);
        const savedLocally = writeLocalCrosshair(profiles, localCode, true);
        setMode("local");
        setCode(localCode);
        setSourceCode(localCode);
        setCodeDirty(false);
        setLossWarning(true);
        setNotice(savedLocally ? t("valorant.crosshair.savedLocal") : t("valorant.crosshair.saveFailed"));
      } else {
        setError(t("valorant.crosshair.saveFailed"));
      }
    }
  }

  const shownCode = mode === "backend" && (codeDirty || encoding) ? "" : code;
  const modeLabel = mode === "backend" ? t("valorant.crosshair.backendMode") : t("valorant.crosshair.localMode");

  return (
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-card p-4 shadow-[var(--cs2-shadow-sm)] sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3"><div className="flex min-w-0 items-start gap-2.5"><span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-violet-400/10 text-violet-300"><Code2 className="h-4.5 w-4.5" /></span><div><div className="flex flex-wrap items-center gap-2"><h2 className="text-base font-bold text-cs2-text-primary">{t("valorant.crosshair.title")}</h2><span className="rounded-full border border-violet-300/20 bg-violet-300/10 px-2 py-0.5 text-[9px] font-bold uppercase tracking-[0.12em] text-violet-200">{t("valorant.crosshair.badge")}</span></div><p className="mt-1 max-w-2xl text-xs leading-5 text-cs2-text-secondary">{t("valorant.crosshair.subtitle")}</p></div></div>{loading ? <span className="inline-flex items-center gap-1.5 text-[10px] text-cs2-text-muted"><RefreshCw className="h-3.5 w-3.5 motion-safe:animate-spin" />{t("valorant.crosshair.loading")}</span> : <StatusBadge status={mode === "backend" ? "ready" : "warning"}>{modeLabel}</StatusBadge>}</div>
      <div className="mt-4 grid gap-4 xl:grid-cols-[0.92fr_1.08fr]">
        <div className="min-w-0"><CrosshairPreview config={config} t={t} /><div className="mt-3 flex flex-wrap items-center gap-2"><div className="mr-auto flex items-center gap-1.5 rounded-xl border border-cs2-border-subtle bg-black/10 p-1" role="tablist" aria-label={t("valorant.crosshair.profileTabs")}>{PROFILE_KEYS.map((profile) => <button key={profile} type="button" role="tab" aria-selected={activeProfile === profile} onClick={() => setActiveProfile(profile)} className={cn("min-w-10 rounded-lg px-3 py-1.5 text-xs font-bold transition-[background-color,color,transform] duration-150 active:scale-[0.96]", activeProfile === profile ? "bg-cs2-accent text-white" : "text-cs2-text-muted hover:bg-cs2-bg-hover hover:text-cs2-text-primary")}>{profile}</button>)}</div><span className="text-[10px] text-cs2-text-muted">{t("valorant.crosshair.profile." + activeProfile)}</span></div><div className="mt-3 rounded-xl border border-cs2-border-subtle bg-black/10 p-3"><div className="mb-2 flex items-center gap-2 text-[10px] font-bold uppercase tracking-[0.15em] text-cs2-text-muted"><Clipboard className="h-3.5 w-3.5" />{t("valorant.crosshair.codeTitle")}</div><code className="block max-h-12 overflow-hidden break-all rounded-lg bg-cs2-bg-input px-2.5 py-2 text-[9px] leading-4 text-cs2-text-secondary">{shownCode || (encoding ? t("valorant.crosshair.codeEncoding") : t("valorant.crosshair.codeUnavailable"))}</code><div className="mt-2 flex flex-wrap gap-2"><button type="button" onClick={() => void copyCode()} className="inline-flex items-center gap-1.5 rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-1.5 text-[10px] font-semibold text-cs2-text-secondary transition-[background-color,transform] duration-150 hover:bg-cs2-bg-hover active:scale-[0.97]"><Clipboard className="h-3.5 w-3.5" />{t("valorant.crosshair.copy")}</button><button type="button" onClick={() => void exportProfile()} className="inline-flex items-center gap-1.5 rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-1.5 text-[10px] font-semibold text-cs2-text-secondary transition-[background-color,transform] duration-150 hover:bg-cs2-bg-hover active:scale-[0.97]"><Download className="h-3.5 w-3.5" />{t("valorant.crosshair.export")}</button><button type="button" onClick={() => importInputRef.current?.click()} className="inline-flex items-center gap-1.5 rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-1.5 text-[10px] font-semibold text-cs2-text-secondary transition-[background-color,transform] duration-150 hover:bg-cs2-bg-hover active:scale-[0.97]"><Upload className="h-3.5 w-3.5" />{t("valorant.crosshair.import")}</button><input ref={importInputRef} type="file" accept=".json,.txt,application/json,text/plain" className="hidden" onChange={(event) => void importProfile(event)} /></div>{lossWarning ? <div role="alert" className="mt-2 rounded-lg border border-amber-300/25 bg-amber-300/[0.06] px-2.5 py-2 text-[10px] leading-4 text-amber-100">{t("valorant.crosshair.lossWarning")}</div> : null}</div></div>
        <div className="min-w-0 space-y-3">
          <div className="rounded-xl border border-cs2-border-subtle bg-black/10 p-3.5"><div className="flex items-center gap-2 text-xs font-bold text-cs2-text-primary"><SlidersHorizontal className="h-4 w-4 text-violet-300" />{t("valorant.crosshair.basicTitle")}</div><div className="mt-3"><div className="mb-1.5 text-[10px] font-semibold text-cs2-text-secondary">{t("valorant.crosshair.color")}</div><div className="flex flex-wrap gap-2">{Object.entries(CROSSHAIR_COLORS).map(([name, value]) => <button key={name} type="button" aria-label={t("valorant.crosshair.color." + name)} aria-pressed={config.color === name} onClick={() => updateConfig("color", name)} className={cn("grid h-7 w-7 place-items-center rounded-full border-2 transition-[border-color,transform] duration-150 active:scale-[0.92]", config.color === name ? "border-white" : "border-transparent")} style={{ backgroundColor: value, boxShadow: config.color === name ? "0 0 0 2px var(--cs2-accent)" : "none" }}><Check className={cn("h-3.5 w-3.5 text-black/70 transition-opacity duration-150", config.color === name ? "opacity-100" : "opacity-0")} /></button>)}</div></div><div className="mt-3 grid gap-2 sm:grid-cols-2"><Toggle label={t("valorant.crosshair.outlines")} checked={config.outlines} onChange={(value) => updateConfig("outlines", value)} /><Toggle label={t("valorant.crosshair.centerDot")} checked={config.centerDot} onChange={(value) => updateConfig("centerDot", value)} /></div></div>
          <div className="rounded-xl border border-cs2-border-subtle bg-black/10 p-3.5"><div className="text-xs font-bold text-cs2-text-primary">{t("valorant.crosshair.innerTitle")}</div><div className="mt-3 grid gap-3 sm:grid-cols-2"><Toggle label={t("valorant.crosshair.innerLines")} checked={config.innerLines} onChange={(value) => updateConfig("innerLines", value)} /><RangeInput label={t("valorant.crosshair.opacity")} value={config.innerLinesOpacity} min={0} max={1} step={0.05} onChange={(value) => updateConfig("innerLinesOpacity", value)} /><RangeInput label={t("valorant.crosshair.length")} value={config.innerLinesLength} min={1} max={12} onChange={(value) => updateConfig("innerLinesLength", value)} /><RangeInput label={t("valorant.crosshair.thickness")} value={config.innerLinesThickness} min={1} max={6} onChange={(value) => updateConfig("innerLinesThickness", value)} /><RangeInput label={t("valorant.crosshair.offset")} value={config.innerLinesOffset} min={0} max={12} onChange={(value) => updateConfig("innerLinesOffset", value)} /></div></div>
          <div className="rounded-xl border border-cs2-border-subtle bg-black/10 p-3.5"><div className="text-xs font-bold text-cs2-text-primary">{t("valorant.crosshair.outerTitle")}</div><div className="mt-3 grid gap-3 sm:grid-cols-2"><Toggle label={t("valorant.crosshair.outerLines")} checked={config.outerLines} onChange={(value) => updateConfig("outerLines", value)} /><RangeInput label={t("valorant.crosshair.opacity")} value={config.outerLinesOpacity} min={0} max={1} step={0.05} onChange={(value) => updateConfig("outerLinesOpacity", value)} /><RangeInput label={t("valorant.crosshair.length")} value={config.outerLinesLength} min={1} max={12} onChange={(value) => updateConfig("outerLinesLength", value)} /><RangeInput label={t("valorant.crosshair.thickness")} value={config.outerLinesThickness} min={1} max={6} onChange={(value) => updateConfig("outerLinesThickness", value)} /><RangeInput label={t("valorant.crosshair.offset")} value={config.outerLinesOffset} min={0} max={16} onChange={(value) => updateConfig("outerLinesOffset", value)} /></div></div>
          <div className="rounded-xl border border-cs2-border-subtle bg-black/10 p-3.5"><div className="text-xs font-bold text-cs2-text-primary">{t("valorant.crosshair.feedbackTitle")}</div><div className="mt-3 grid gap-2 sm:grid-cols-2"><Toggle label={t("valorant.crosshair.firingError")} checked={config.firingError} onChange={(value) => updateConfig("firingError", value)} /><Toggle label={t("valorant.crosshair.movementError")} checked={config.movementError} onChange={(value) => updateConfig("movementError", value)} /></div></div>
        </div>
      </div>
      <div className="mt-4 flex flex-wrap items-center justify-between gap-2 border-t border-cs2-border-subtle pt-3"><div className="flex min-w-0 items-center gap-2 text-[11px] text-cs2-text-muted"><span className={cn("h-2 w-2 shrink-0 rounded-full", mode === "backend" ? "bg-emerald-300" : "bg-amber-300")} />{mode === "backend" ? t("valorant.crosshair.backendHint") : t("valorant.crosshair.localModeNotice")}</div><button type="button" onClick={() => void saveProfile()} className="inline-flex items-center gap-1.5 rounded-lg bg-cs2-accent px-3.5 py-2 text-[11px] font-bold text-white transition-[background-color,transform] duration-150 hover:bg-cs2-accent-light active:scale-[0.97]"><Save className="h-3.5 w-3.5" />{t("valorant.crosshair.save")}</button></div>
      {notice ? <div role="status" className="mt-3 rounded-lg border border-emerald-300/20 bg-emerald-300/[0.06] px-3 py-2 text-[11px] text-emerald-200">{notice}</div> : null}
      {error ? <div role="alert" className="mt-3 rounded-lg border border-amber-300/25 bg-amber-300/[0.06] px-3 py-2 text-[11px] text-amber-100">{error}</div> : null}
    </section>
  );
}

export default function ValorantLabPage() {
  const t = useT();
  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-5 sm:p-7">
      <div className="mx-auto flex max-w-[1500px] flex-col gap-5">
        <header className="flex flex-wrap items-end justify-between gap-4"><div><div className="mb-2 inline-flex items-center gap-2 rounded-full border border-cs2-accent/25 bg-cs2-accent-soft px-2.5 py-1 text-[11px] font-bold uppercase tracking-[0.16em] text-cs2-accent"><Gamepad2 className="h-3.5 w-3.5" />{t("valorant.badge")}</div><h1 className="text-2xl font-bold tracking-tight text-cs2-text-primary">{t("valorant.pageTitle")}</h1><p className="mt-1 max-w-3xl text-sm leading-6 text-cs2-text-secondary">{t("valorant.pageSubtitle")}</p></div><div className="rounded-xl border border-cs2-border-subtle bg-cs2-bg-card px-3.5 py-2.5 text-right"><div className="text-[9px] font-bold uppercase tracking-[0.16em] text-cs2-text-muted">{t("valorant.protocol")}</div><div className="mt-1 font-mono text-xs font-bold text-cs2-text-primary">{t("valorant.protocolValue")}</div></div></header>
        <div className="grid items-start gap-5 2xl:grid-cols-[1.05fr_0.95fr]"><StretchWizard t={t} /><CrosshairEditor t={t} /></div>
      </div>
    </div>
  );
}
