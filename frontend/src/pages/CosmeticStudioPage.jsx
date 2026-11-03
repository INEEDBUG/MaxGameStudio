import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { AlertTriangle, CheckCircle2, CircleDot, Eye, Film, LoaderCircle, LockKeyhole, Play, RotateCcw, ShieldCheck, Square, Wrench, XCircle } from "lucide-react";
import {
  cancelCosmeticJob,
  confirmCosmeticJob,
  createCosmeticJob,
  getCosmeticCapabilities,
  getActiveCosmeticJob,
  getCosmeticJob,
  getCosmeticPreviewBlob,
  inspectCosmeticDemo,
  importCosmeticDirectorPlan,
  validateCosmeticInterval,
  getCosmeticProfiles, saveCosmeticProfile, deleteCosmeticProfile,
} from "../api/cosmeticStudio.js";

const TERMINAL_STATES = new Set(["complete", "failed", "cancelled"]);
const TOOL_KEYS = ["cs2_exe", "hlae_exe", "nskinz_dll", "core_exe", "obs_exe", "ffmpeg_exe", "ffprobe_exe", "output_dir"];
const TOOL_LABELS = { cs2_exe: "CS2 程序", hlae_exe: "HLAE 程序", nskinz_dll: "nSkinz DLL（nSkinz 提供方）", core_exe: "SkinCore 程序（文件提供方）", obs_exe: "OBS 程序", ffmpeg_exe: "FFmpeg 程序", ffprobe_exe: "ffprobe 程序", output_dir: "输出目录" };
const EMPTY_TOOLS = Object.fromEntries(TOOL_KEYS.map((key) => [key, ""]));
const candidateKey = (candidate) => `${String(candidate?.xuid || "")}:${Number(candidate?.start_tick || 0)}:${Number(candidate?.end_tick || 0)}`;

function responseData(response) {
  return response?.data ?? response ?? {};
}

function apiError(error, fallback) {
  const detail = error?.response?.data?.message || error?.response?.data?.detail;
  if (typeof detail === "string" && detail) return detail;
  const issues = Array.isArray(detail) ? detail : detail ? [detail] : [];
  const messages = issues.slice(0, 5).flatMap((issue) => {
    if (typeof issue === "string") return [issue];
    if (typeof issue?.msg !== "string") return [];
    const field = Array.isArray(issue.loc) ? issue.loc.at(-1) : null;
    const label = TOOL_LABELS[field] || (field === "demo_path" ? "Demo 路径" : "输入参数");
    // Do not render the validation payload/input: it may contain private paths.
    return [`${label}：${issue.msg}`];
  });
  return messages.join("；") || (typeof error?.message === "string" ? error.message : fallback);
}

function StatusLine({ state, message }) {
  const Icon = state === "complete" ? CheckCircle2 : state === "failed" ? AlertTriangle : state === "cancelled" ? XCircle : state === "preview" ? Eye : CircleDot;
  return <div className="flex items-start gap-3 rounded-xl border border-cs2-border-subtle bg-cs2-bg-elevated/70 p-3 text-sm" role="status" aria-live="polite">
    <Icon className="mt-0.5 h-4 w-4 shrink-0 text-cs2-text-secondary" aria-hidden="true" />
    <span className="min-w-0 leading-6 text-cs2-text-secondary">{message || state || "等待操作"}</span>
  </div>;
}

function Step({ number, title, active, complete, children }) {
  return <section className={`cs-studio-step rounded-2xl border p-4 transition-colors ${active ? "border-cs2-accent/40 bg-cs2-bg-elevated" : "border-cs2-border-subtle bg-cs2-bg-elevated/60"}`}>
    <div className="mb-4 flex items-center gap-3">
      <span className={`flex h-7 w-7 items-center justify-center rounded-full border text-xs font-bold ${complete ? "border-cs2-accent/60 text-cs2-accent" : active ? "border-cs2-text-secondary text-cs2-text-primary" : "border-cs2-border text-cs2-text-muted"}`} aria-hidden="true">{complete ? "✓" : number}</span>
      <h2 className="text-sm font-bold text-cs2-text-primary">{title}</h2>
    </div>
    {children}
  </section>;
}

export default function CosmeticStudioPage() {
  const [capabilities, setCapabilities] = useState(null);
  const [capabilityError, setCapabilityError] = useState("");
  const [demoPath, setDemoPath] = useState("");
  const [inspection, setInspection] = useState(null);
  const [selectedCandidate, setSelectedCandidate] = useState("");
  const [manualXuid, setManualXuid] = useState('');
  const [manualStart, setManualStart] = useState('');
  const [manualEnd, setManualEnd] = useState('');
  const [providerId, setProviderId] = useState("nskinz");
  const [selectedPresetId, setSelectedPresetId] = useState("awp-dragon-lore");
  const [patternSeed, setPatternSeed] = useState(0);
  const [wear, setWear] = useState(0.01);
  const [glovePresetId, setGlovePresetId] = useState('');
  const [profiles, setProfiles] = useState([]);
  const [profileName, setProfileName] = useState('');
  const [profileId, setProfileId] = useState('');
  const [profileError, setProfileError] = useState('');
  const [profileBusy, setProfileBusy] = useState(false);
  const [inspectBusy, setInspectBusy] = useState(false);
  const [startBusy, setStartBusy] = useState(false);
  const [job, setJob] = useState(null);
  const [actionError, setActionError] = useState("");
  const [pollError, setPollError] = useState("");
  const [activeError, setActiveError] = useState("");
  const [activeLoading, setActiveLoading] = useState(true);
  const setJobError = (value) => { setActionError(value); setPollError(""); };
  const [previewState, setPreviewState] = useState("empty");
  const [previewUrl, setPreviewUrl] = useState("");
  const [previewError, setPreviewError] = useState("");
  const [appearanceConfirmed, setAppearanceConfirmed] = useState(false);
  const [tools, setTools] = useState(EMPTY_TOOLS);
  const [pollGeneration, setPollGeneration] = useState(0);
  const previewObjectUrlRef = useRef("");
  const pollInFlightRef = useRef(false);
  const previewGenerationRef = useRef(0);
  const startInFlightRef = useRef(false);
  const confirmInFlightRef = useRef(false);

  const enabled = capabilities?.enabled === true;
  const candidates = inspection?.candidates || [];
  const selected = candidates.find((candidate) => candidateKey(candidate) === selectedCandidate) || null;
  const providers = capabilities?.providers?.length ? capabilities.providers : [{ id: "nskinz", label: "nSkinz", notice: "本地运行时提供方" }];
  const provider = providers.find((item) => item.id === providerId) || providers[0];
  const presets = useMemo(() => {
    const all = capabilities?.presets?.length ? capabilities.presets : [{ id: "awp-dragon-lore", label: "AWP · 巨龙传说", weapon_id: 9, paint_kit: 344 }];
    return all.filter((item) => (!item.provider_ids?.length || item.provider_ids.includes(provider?.id))
      && (!selected || (item.source_weapon_id ?? item.weapon_id) === (selected.weapon_id || 9)));
  }, [capabilities, provider?.id, selected]);
  const preset = presets.find((item) => item.id === selectedPresetId) || presets[0];
  const maximumWear = [507, 515].includes(preset?.weapon_id) ? 0.08 : 1;
  const gloveOptions = (capabilities?.glove_presets || []).filter(item=>
    !item.weapon_ids || item.weapon_ids.includes(preset?.weapon_id));
  const activeGlovePresetId = gloveOptions.some(item=>item.id===glovePresetId) ? glovePresetId : '';
  const visibleToolKeys = TOOL_KEYS.filter((key) => provider?.id === "skin_core" ? key !== "nskinz_dll" : key !== "core_exe");
  const customValuesValid = provider?.id !== "skin_core"
    || (String(patternSeed).trim() !== "" && Number.isInteger(Number(patternSeed)) && Number(patternSeed) >= 0 && Number(patternSeed) <= 1000
      && String(wear).trim() !== "" && Number.isFinite(Number(wear)) && Number(wear) >= 0 && Number(wear) <= maximumWear);
  const manualValuesValid = manualXuid && manualStart.trim() && manualEnd.trim()
    && Number.isInteger(Number(manualStart)) && Number.isInteger(Number(manualEnd))
    && Number(manualStart) >= 256 && Number(manualEnd) <= 100000000
    && Number(manualEnd) - Number(manualStart) >= 128 && Number(manualEnd) - Number(manualStart) <= 1920;
  const running = Boolean(job && !TERMINAL_STATES.has(job.state));
  const inputsLocked = running || inspectBusy || startBusy;
  const canConfirm = job?.state === "preview" && previewState === "ready" && appearanceConfirmed && Boolean(job.preview_id) && !actionError && !pollError && !confirmInFlightRef.current;

  useEffect(() => {
    let mounted = true;
    getCosmeticCapabilities().then((response) => {
      if (!mounted) return;
      const data = responseData(response);
      setCapabilities(data);
      setTools({ ...EMPTY_TOOLS, ...(data.defaults || {}) });
      const preferredProvider = data.default_provider || "nskinz";
      setProviderId((data.providers || []).some((item) => item.id === preferredProvider) ? preferredProvider : (data.providers?.[0]?.id || "nskinz"));
      if (data.presets?.[0]?.id) setSelectedPresetId(data.presets[0].id);
    }).catch((error) => {
      if (mounted) setCapabilityError(apiError(error, "无法读取本地实验能力状态"));
    });
    getActiveCosmeticJob().then((response) => {
      if (mounted) {
        const active = response && Object.prototype.hasOwnProperty.call(response, "data") ? response.data : response;
        setJob((current) => current || active);
      }
    }).catch((error) => { if (mounted) setActiveError(apiError(error, "无法读取本地会话状态；请重试")); }).finally(() => { if (mounted) setActiveLoading(false); });
    getCosmeticProfiles().then(response=>{if(mounted) setProfiles(responseData(response));})
      .catch(error=>{if(mounted) setProfileError(apiError(error,'无法读取玩家预设，原有文件不会被覆盖'));});
    return () => { mounted = false; };
  }, []);

  useEffect(() => () => {
    previewGenerationRef.current += 1;
    if (previewObjectUrlRef.current) URL.revokeObjectURL(previewObjectUrlRef.current);
  }, []);

  const clearPreview = useCallback(() => {
    previewGenerationRef.current += 1;
    if (previewObjectUrlRef.current) URL.revokeObjectURL(previewObjectUrlRef.current);
    previewObjectUrlRef.current = "";
    setPreviewUrl("");
    setPreviewState("empty");
    setPreviewError("");
    setAppearanceConfirmed(false);
  }, []);

  const loadPreview = useCallback(async (url) => {
    clearPreview();
    const generation = previewGenerationRef.current;
    if (!url) return;
    setPreviewState("loading");
    try {
      const response = await getCosmeticPreviewBlob(url);
      if (generation !== previewGenerationRef.current) return;
      const objectUrl = URL.createObjectURL(responseData(response));
      previewObjectUrlRef.current = objectUrl;
      setPreviewUrl(objectUrl);
      setPreviewState("decoding");
    } catch (error) {
      if (generation !== previewGenerationRef.current) return;
      setPreviewState("error");
      setPreviewError(apiError(error, "预览图片加载失败，不能确认或录制"));
    }
  }, [clearPreview]);

  useEffect(() => {
    if (job?.state === "preview" && job.preview_url) loadPreview(job.preview_url);
    else if (!job || job.state !== "preview") clearPreview();
  }, [job?.state, job?.preview_url, loadPreview, clearPreview]);

  useEffect(() => {
    if (!job?.id || TERMINAL_STATES.has(job.state)) return undefined;
    let disposed = false;
    const timer = window.setInterval(async () => {
      if (disposed || pollInFlightRef.current) return;
      pollInFlightRef.current = true;
      try {
        const response = await getCosmeticJob(job.id);
        if (!disposed) { setJob(responseData(response)); setPollError(""); }
      } catch (error) {
        if (!disposed) setPollError(apiError(error, "状态读取失败；不会把任务误报为成功"));
      } finally {
        pollInFlightRef.current = false;
      }
    }, 900);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [job?.id, job?.state, pollGeneration]);

  const inspect = async (event) => {
    event.preventDefault();
    setInspectBusy(true); setActionError("");
    try {
      const data = responseData(await inspectCosmeticDemo(demoPath.trim()));
      setInspection(data);
      setManualXuid(data.players?.[0]?.xuid || '');
      setSelectedCandidate(data.candidates?.[0] ? candidateKey(data.candidates[0]) : "");
    } catch (error) { setActionError(apiError(error, "Demo 检查失败，请保留输入后重试")); }
    finally { setInspectBusy(false); }
  };

  const startJob = async () => {
    if (!enabled || activeLoading || activeError || !selected || !provider?.id || !preset?.id || !customValuesValid || inputsLocked || job || startInFlightRef.current || visibleToolKeys.some((key) => !String(tools[key] || "").trim())) return;
    startInFlightRef.current = true;
    setStartBusy(true);
    setActionError("");
    try {
      const data = responseData(await createCosmeticJob({
        demo_path: demoPath.trim(), expected_demo_sha256: inspection.demo_sha256, xuid: String(selected.xuid), start_tick: Number(selected.start_tick), end_tick: Number(selected.end_tick),
        provider_id: provider.id, preset_id: preset.id, pattern_seed: provider.id === "nskinz" ? 0 : Number(patternSeed), wear: provider.id === "nskinz" ? 0.01 : Number(wear), tools: Object.fromEntries(visibleToolKeys.map((key) => [key, tools[key]])), acknowledge_offline: true,
        glove_preset_id: provider.id === 'skin_core' ? activeGlovePresetId || null : null,
      }));
      setJob(data); setPollGeneration((value) => value + 1);
    } catch (error) { setActionError(apiError(error, "无法启动专用离线会话")); }
    finally { startInFlightRef.current = false; setStartBusy(false); }
  };

  const importDirectorPlan = async (event) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file || inputsLocked || !inspection?.demo_sha256) return;
    setInspectBusy(true); setActionError('');
    try {
      if (file.size > 512 * 1024) throw new Error('导演计划不得超过 512 KB');
      const plan = JSON.parse(await file.text());
      const data = responseData(await importCosmeticDirectorPlan({demo_path:demoPath.trim(),
        expected_demo_sha256:inspection.demo_sha256,plan}));
      setInspection(data); setSelectedCandidate(data.candidates?.[0] ? candidateKey(data.candidates[0]) : '');
      clearPreview();
    } catch(error) {setActionError(apiError(error,'导演计划导入失败；未启动或录制'));}
    finally {setInspectBusy(false);}
  };

  const validateManualKnife = async () => {
    if (!manualValuesValid || inputsLocked || provider?.id !== 'skin_core' || !inspection?.demo_sha256) return;
    setInspectBusy(true); setActionError('');
    try {
      const candidate = responseData(await validateCosmeticInterval({demo_path:demoPath.trim(),
        expected_demo_sha256:inspection.demo_sha256, xuid:manualXuid,
        start_tick:Number(manualStart), end_tick:Number(manualEnd), weapon_id:507}));
      setInspection(current => ({...current, candidates:[...(current.candidates || []).filter(
        item => candidateKey(item) !== candidateKey(candidate)), candidate]}));
      setSelectedCandidate(candidateKey(candidate));
      setSelectedPresetId('karambit-fade'); clearPreview();
    } catch (error) { setActionError(apiError(error, '刀展示片段未通过连续持有与归属检查')); }
    finally { setInspectBusy(false); }
  };

  const saveProfile = async () => {
    if(inputsLocked || profileBusy || !selected || !preset || !customValuesValid || !profileName.trim()) return;
    setProfileBusy(true); setProfileError('');
    try {
      const saved = responseData(await saveCosmeticProfile({name:profileName.trim(),xuid:String(selected.xuid),
        demo_sha256:inspection.demo_sha256,start_tick:selected.start_tick,end_tick:selected.end_tick,
        preset_id:preset.id,pattern_seed:Number(patternSeed),wear:Number(wear),
        glove_preset_id:provider?.id==='skin_core' ? activeGlovePresetId || null : null}));
      setProfiles(items=>[...items,saved]);setProfileId(saved.id);setProfileName('');
    } catch(error) {setProfileError(apiError(error,'预设保存失败'));}
    finally {setProfileBusy(false);}
  };
  const applyProfile = () => {
    if(inputsLocked || profileBusy) return;
    const saved=profiles.find(item=>item.id===profileId);
    if(!saved || !selected || (saved.demo_sha256 && saved.demo_sha256!==inspection?.demo_sha256)
       || saved.xuid!==String(selected.xuid) || (saved.start_tick!=null &&
          (saved.start_tick!==selected.start_tick || saved.end_tick!==selected.end_tick))
       || !presets.some(item=>item.id===saved.preset_id)) {
      setProfileError('预设的 Demo、玩家或片段与当前选择不匹配；请重新检查后选择。');return;
    }
    if(saved.glove_preset_id && (provider?.id !== 'skin_core' ||
      !gloveOptions.some(item=>item.id===saved.glove_preset_id))) {
      setProfileError('当前提供方不支持此手套预设，请先选择文件提供方。');return;
    }
    setSelectedPresetId(saved.preset_id);setPatternSeed(saved.pattern_seed);setWear(saved.wear);setProfileError('');
    setGlovePresetId(saved.glove_preset_id || '');
    clearPreview(); // Applying configuration is never approval to record.
  };
  const removeProfile = async () => {
    if(inputsLocked || profileBusy || !profileId) return;
    setProfileBusy(true);setProfileError('');
    try {await deleteCosmeticProfile(profileId);setProfiles(items=>items.filter(item=>item.id!==profileId));setProfileId('');}
    catch(error){setProfileError(apiError(error,'预设删除失败'));}
    finally{setProfileBusy(false);}
  };

  const confirmPreview = async () => {
    if (!canConfirm || confirmInFlightRef.current) return;
    confirmInFlightRef.current = true;
    setActionError("");
    try { setJob(responseData(await confirmCosmeticJob(job.id, job.preview_id))); }
    catch (error) { setActionError(apiError(error, "确认失败；未开始录制")); }
    finally { confirmInFlightRef.current = false; }
  };

  const cancelJob = async () => {
    if (!job?.id || !running) return;
    setActionError("");
    try { setJob(responseData(await cancelCosmeticJob(job.id))); }
    catch (error) { setActionError(apiError(error, "取消请求失败，请保留当前状态并重试")); }
  };

  return <section className="cs-workspace-cosmetic flex min-h-0 flex-1 flex-col overflow-y-auto">
    <div className="mx-auto w-full max-w-6xl space-y-5 p-4 md:p-6" data-testid="cosmetic-studio">
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div><div className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.18em] text-cs2-accent"><Film className="h-4 w-4" aria-hidden="true" />Cosmetic Studio · 本地实验</div><h1 className="text-2xl font-bold text-cs2-text-primary">Demo 外观预览</h1><p className="mt-2 max-w-2xl text-sm leading-6 text-cs2-text-muted">仅限离线 Demo。先检查目标片段，再查看真实预览；每次录制都必须由你确认当前画面。</p></div>
      <div className="flex items-center gap-2 rounded-full border border-cs2-border-subtle bg-cs2-bg-elevated/70 px-3 py-2 text-xs text-cs2-text-secondary"><LockKeyhole className="h-4 w-4" aria-hidden="true" />不触碰在线库存或普通录制流程</div>
    </header>

    {capabilityError && <StatusLine state="failed" message={capabilityError} />}
    {activeError && <div className="flex items-center justify-between gap-3 rounded-xl border border-amber-400/30 bg-amber-400/5 p-3 text-sm text-cs2-text-primary" role="alert"><span>{activeError}</span><button type="button" onClick={() => { setActiveError(""); setActiveLoading(true); getActiveCosmeticJob().then((response) => setJob(response && Object.prototype.hasOwnProperty.call(response, "data") ? response.data : response)).catch((error) => setActiveError(apiError(error, "无法读取本地会话状态；请重试"))).finally(() => setActiveLoading(false)); }} className="rounded-lg border border-amber-300/40 px-3 py-1.5 text-xs font-semibold text-cs2-text-primary">重试读取状态</button></div>}
    {capabilities && !enabled && <div className="rounded-2xl border border-amber-400/30 bg-amber-400/5 p-4 text-sm leading-6 text-cs2-text-primary"><div className="flex items-center gap-2 font-semibold"><ShieldCheck className="h-4 w-4" aria-hidden="true" />本地实验暂不可用</div><p className="mt-1 text-cs2-text-secondary">{capabilities.reason || "后端未开启此功能。"}</p></div>}

    <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(320px,0.72fr)]">
      <div className="space-y-4">
        <Step number="1" title="选择 Demo 与目标片段" active={!inspection} complete={Boolean(inspection)}>
          <form onSubmit={inspect} className="space-y-4">
            <label className="block"><span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">本地 Demo 路径</span><input aria-label="本地 Demo 路径" disabled={inputsLocked} value={demoPath} onChange={(event) => { const value = event.target.value; setDemoPath(value); if (value !== demoPath) { setInspection(null); setSelectedCandidate(""); clearPreview(); } }} placeholder="D:\\...\\match.dem" className="w-full rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary outline-none focus:border-cs2-accent/60 disabled:cursor-not-allowed disabled:opacity-60" /></label>
            <div className="flex flex-wrap items-center gap-3"><button type="submit" disabled={!enabled || !demoPath.trim() || inputsLocked} className="inline-flex min-h-11 items-center gap-2 rounded-xl bg-cs2-accent px-4 py-2 text-sm font-bold text-cs2-text-on-accent disabled:cursor-not-allowed disabled:opacity-40"><Wrench className="h-4 w-4" aria-hidden="true" />{inspectBusy ? "检查中…" : "检查 Demo"}</button>{inspection?.demo_sha256 && <span className="text-xs text-cs2-text-muted">SHA256：{inspection.demo_sha256}</span>}</div>
          </form>
          {candidates.length > 0 && <label className="mt-5 block"><span className="mb-1.5 block text-xs font-semibold text-cs2-text-secondary">目标玩家与片段</span><select aria-label="目标玩家与片段" disabled={inputsLocked} value={selectedCandidate} onChange={(event) => setSelectedCandidate(event.target.value)} className="w-full rounded-xl border border-cs2-border bg-cs2-bg-input px-3 py-2.5 text-sm text-cs2-text-primary disabled:cursor-not-allowed disabled:opacity-60">{candidates.map((candidate) => <option key={candidateKey(candidate)} value={candidateKey(candidate)}>{candidate.label || `${candidate.name || candidate.xuid} · tick ${candidate.start_tick}–${candidate.end_tick}`}</option>)}</select><span className="mt-1 block text-xs leading-5 text-cs2-text-muted">仅保留连续至少 2 秒、目标玩家存活且明确持有所选武器的片段；武器切换或数据间隙会被跳过。</span></label>}
          {inspection?.candidates_limited && <p className="mt-3 text-xs text-cs2-text-muted">当前列表为有界抽样结果，不代表全部精彩片段。</p>}
          {inspection && <label className="mt-3 block text-xs text-cs2-text-secondary">导入现有 AI 导演计划（仅筛选片段，不自动录制）<input type="file" accept=".json,application/json" aria-label="导入导演计划 JSON" disabled={inputsLocked} onChange={importDirectorPlan} className="mt-2 block w-full rounded-lg border border-cs2-border p-2 text-cs2-text-primary disabled:opacity-50" /></label>}
          {inspection?.players?.length > 0 && provider?.id === 'skin_core' && capabilities?.presets?.some(item=>item.weapon_id===507) && <details className="mt-4 rounded-xl border border-cs2-border-subtle p-3 text-xs text-cs2-text-secondary">
            <summary className="cursor-pointer font-semibold">可选：指定爪子刀展示片段</summary>
            <p className="mt-2 leading-5 text-cs2-text-muted">原始 Demo 中需持有自己的爪子刀；校验后可选择保留刀型，或使用已验证的替换刀型预设。填写 64 tick/s 的 2–30 秒片段；校验包含前一秒预滚。此操作不会自动启动或录制。</p>
            <div className="mt-3 grid gap-3 sm:grid-cols-3">
              <label>目标玩家<select aria-label="刀展示目标玩家" value={manualXuid} disabled={inputsLocked} onChange={event=>setManualXuid(event.target.value)} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input p-2 text-cs2-text-primary">{inspection.players.map(player=><option key={player.xuid} value={player.xuid}>{player.name}</option>)}</select></label>
              <label>开始 tick<input aria-label="刀展示开始 tick" type="number" min="256" step="1" value={manualStart} disabled={inputsLocked} onChange={event=>setManualStart(event.target.value)} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input p-2 text-cs2-text-primary" /></label>
              <label>结束 tick<input aria-label="刀展示结束 tick" type="number" step="1" value={manualEnd} disabled={inputsLocked} onChange={event=>setManualEnd(event.target.value)} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input p-2 text-cs2-text-primary" /></label>
            </div>
            <button type="button" onClick={validateManualKnife} disabled={inputsLocked || !manualValuesValid} className="mt-3 min-h-11 rounded-lg border border-cs2-border px-3 text-cs2-text-primary disabled:opacity-40">校验刀展示片段</button>
          </details>}
          {inspection && candidates.length === 0 && <p className="mt-4 text-sm text-cs2-text-muted">未发现可用目标片段。请更换 Demo 或保留路径后重试。</p>}
        </Step>

        <Step number="2" title="启动专用离线会话并预览" active={Boolean(inspection) && !job?.state} complete={job?.state === "preview" || job?.state === "recording" || job?.state === "exporting" || job?.state === "complete"}>
          <div className="rounded-xl border border-cs2-border-subtle bg-cs2-bg-input/40 p-3">
            <div className="grid gap-3 sm:grid-cols-2">
              <label className="block"><span className="mb-1 block text-[11px] font-semibold text-cs2-text-secondary">外观提供方</span><select aria-label="外观提供方" disabled={inputsLocked} value={provider?.id || ""} onChange={(event) => { const next = event.target.value; setProviderId(next); setSelectedPresetId(""); if (next === "nskinz") { setPatternSeed(0); setWear(0.01); } }} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-2 text-xs text-cs2-text-primary disabled:cursor-not-allowed disabled:opacity-60">{providers.map((item) => <option key={item.id} value={item.id}>{item.label || item.id}</option>)}</select></label>
              <label className="block"><span className="mb-1 block text-[11px] font-semibold text-cs2-text-secondary">皮肤预设</span><select aria-label="皮肤预设" disabled={inputsLocked || !presets.length} value={preset?.id || ""} onChange={(event) => setSelectedPresetId(event.target.value)} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-2 text-xs text-cs2-text-primary disabled:cursor-not-allowed disabled:opacity-60">{presets.map((item) => <option key={item.id} value={item.id}>{item.label || item.id}</option>)}</select></label>
            </div>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              {provider?.id === "skin_core" ? <>
                <label className="block"><span className="mb-1 block text-[11px] font-semibold text-cs2-text-secondary">图案种子（0–1000）</span><input aria-label="图案种子" type="number" min="0" max="1000" step="1" disabled={inputsLocked} value={patternSeed} onChange={(event) => setPatternSeed(event.target.value)} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-2 text-xs text-cs2-text-primary disabled:cursor-not-allowed disabled:opacity-60" />{String(patternSeed).trim() === "" || !Number.isInteger(Number(patternSeed)) || Number(patternSeed) < 0 || Number(patternSeed) > 1000 ? <span className="mt-1 block text-xs text-cs2-text-secondary">请输入 0–1000 的整数。</span> : null}</label>
                <label className="block"><span className="mb-1 block text-[11px] font-semibold text-cs2-text-secondary">磨损（0–{maximumWear}）</span><input aria-label="磨损" type="number" min="0" max={maximumWear} step="0.001" disabled={inputsLocked} value={wear} onChange={(event) => setWear(event.target.value)} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-2 text-xs text-cs2-text-primary disabled:cursor-not-allowed disabled:opacity-60" />{String(wear).trim() === "" || !Number.isFinite(Number(wear)) || Number(wear) < 0 || Number(wear) > maximumWear ? <span className="mt-1 block text-xs text-cs2-text-secondary">请输入当前外观允许范围内的有效磨损。</span> : null}</label>
              </> : <p className="sm:col-span-2 text-xs leading-5 text-cs2-text-muted">nSkinz 实验使用已验证的固定种子与近零磨损；切换到文件提供方后可编辑。</p>}
              {provider?.id === 'skin_core' && gloveOptions.length > 0 && <label className="sm:col-span-3 block">
                <span className="mb-1 block text-[11px] font-semibold text-cs2-text-secondary">手套外观（可选）</span>
                <select aria-label="手套外观" disabled={inputsLocked} value={activeGlovePresetId} onChange={event=>setGlovePresetId(event.target.value)} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-2 text-xs text-cs2-text-primary">
                  <option value="">保持原手套</option>{gloveOptions.map(item=><option key={item.id} value={item.id}>{item.label}</option>)}
                </select><span className="mt-1 block text-xs leading-5 text-cs2-text-muted">仅支持已验证的原手套型号 5030；固定种子 0、磨损 0.01。身份或型号不符会停止，不会尝试猜测替换。预览时需同时核对武器和手套。</span>
              </label>}
            </div>
            <div className="mt-3 flex items-center gap-2 text-sm font-semibold text-cs2-text-primary"><Play className="h-4 w-4" aria-hidden="true" />{preset?.label || "没有可用预设"}</div><p className="mt-1 text-xs leading-5 text-cs2-text-muted">{provider?.notice || "仅用于离线 Demo 外观预览；不触碰在线库存。"}</p>
          </div>
          <section className="mt-3 space-y-3 rounded-xl border border-cs2-border-subtle p-3" aria-label="玩家预设管理">
            <p className="text-xs text-cs2-text-muted">保存到应用的数据目录；绑定当前 Demo、玩家与片段。应用预设不会启动游戏或代替画面确认。</p>
            <label className="block"><span className="text-xs text-cs2-text-secondary">预设名称</span><input aria-label="预设名称" maxLength={80} disabled={inputsLocked || profileBusy} value={profileName} onChange={event=>setProfileName(event.target.value)} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-sm text-cs2-text-primary" /></label>
            <button type="button" disabled={inputsLocked || profileBusy || !selected || !preset || !customValuesValid || !profileName.trim()} onClick={saveProfile} className="min-h-11 rounded-lg border border-cs2-border px-3 text-xs text-cs2-text-primary disabled:opacity-40">保存玩家预设</button>
            <label className="block"><span className="text-xs text-cs2-text-secondary">已保存玩家预设</span><select aria-label="已保存玩家预设" disabled={inputsLocked || profileBusy} value={profileId} onChange={event=>setProfileId(event.target.value)} className="mt-1 w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-sm text-cs2-text-primary"><option value="">选择已保存预设</option>{profiles.map(item=><option value={item.id} key={item.id}>{item.name} · {item.xuid}</option>)}</select></label>
            <div className="flex gap-2"><button type="button" disabled={inputsLocked || profileBusy || !profileId || !selected} onClick={applyProfile} className="min-h-11 rounded-lg border border-cs2-border px-3 text-xs text-cs2-text-primary disabled:opacity-40">应用预设</button><button type="button" disabled={inputsLocked || profileBusy || !profileId} onClick={removeProfile} className="min-h-11 rounded-lg border border-cs2-border px-3 text-xs text-cs2-text-primary disabled:opacity-40">删除预设</button></div>
            {profileError && <p role="alert" className="text-xs text-cs2-text-primary">{profileError}</p>}
          </section>
          <p className="text-xs leading-5 text-cs2-text-muted">本实验请选择较短的输出目录，例如 <code className="text-cs2-text-secondary">D:\MGS</code>；过长目录会在启动前提示，不会自动搬迁数据。</p>
          <div className="mt-4 grid gap-2 sm:grid-cols-2">{visibleToolKeys.map((key) => <label key={key} className="block"><span className="mb-1 block text-[11px] font-semibold text-cs2-text-muted">{TOOL_LABELS[key]}</span><input aria-label={key} disabled={inputsLocked} value={tools[key] || ""} onChange={(event) => setTools((current) => ({ ...current, [key]: event.target.value }))} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input px-2.5 py-2 text-xs text-cs2-text-primary disabled:cursor-not-allowed disabled:opacity-60" /></label>)}</div>
          <button type="button" onClick={startJob} disabled={!enabled || activeLoading || Boolean(activeError) || !selected || !preset || !customValuesValid || inputsLocked || Boolean(job) || visibleToolKeys.some((key) => !String(tools[key] || "").trim())} className="mt-4 inline-flex min-h-11 items-center gap-2 rounded-xl border border-cs2-accent/50 px-4 py-2 text-sm font-bold text-cs2-accent disabled:cursor-not-allowed disabled:opacity-40"><Play className="h-4 w-4" aria-hidden="true" />{activeLoading ? "读取状态中…" : startBusy ? "启动中…" : "启动预览"}</button>
          {job && <div className="mt-4"><StatusLine state={job.state} message={job.message || job.state} /></div>}
          {actionError && <p className="mt-3 text-sm leading-6 text-cs2-text-primary" role="alert">{actionError}</p>}
          {pollError && <p className="mt-3 text-sm leading-6 text-cs2-text-primary" role="alert">{pollError}</p>}
        </Step>

        <Step number="3" title="确认当前画面后录制" active={job?.state === "preview"} complete={job?.state === "recording" || job?.state === "exporting" || job?.state === "complete"}>
          <p className="text-sm leading-6 text-cs2-text-muted">只有图片成功加载且与你看到的当前画面一致时，确认按钮才可用。未加载、加载失败或网络异常都不会开始录制。</p>
          <label className="mt-4 flex items-start gap-2 text-sm leading-6 text-cs2-text-secondary"><input type="checkbox" aria-label="确认当前预览画面" checked={appearanceConfirmed} onChange={(event) => setAppearanceConfirmed(event.target.checked)} disabled={previewState !== "ready" || job?.state !== "preview"} className="mt-1 h-4 w-4 accent-cs2-accent" />我确认当前预览画面与目标外观一致</label>
          <button type="button" onClick={confirmPreview} disabled={!canConfirm} className="mt-3 inline-flex min-h-11 items-center gap-2 rounded-xl bg-cs2-accent px-4 py-2 text-sm font-bold text-cs2-text-on-accent disabled:cursor-not-allowed disabled:opacity-40"><CheckCircle2 className="h-4 w-4" aria-hidden="true" />确认当前画面并录制</button>
          {job?.output_path && <p className="mt-3 break-all text-xs text-cs2-text-muted">录制输出：{job.output_path}</p>}
          {job?.litecut_project_id && <p className="mt-1 text-xs text-cs2-accent">已创建 LiteCut 工程 #{job.litecut_project_id} · <Link className="underline underline-offset-2" to="/cs2/lite-cut">打开 LiteCut</Link></p>}
          {job?.output_path && <p className="mt-1 text-xs text-cs2-text-muted"><Link className="underline underline-offset-2" to="/cs2/recorded-videos">在录制素材中查看</Link></p>}
        </Step>
      </div>

      <aside className="space-y-4">
        <section className="overflow-hidden rounded-2xl border border-cs2-border bg-cs2-bg-elevated">
          <div className="flex items-center justify-between border-b border-cs2-border-subtle px-4 py-3"><h2 className="text-sm font-bold text-cs2-text-primary">原生预览</h2><span className="text-xs text-cs2-text-muted">{previewState === "ready" ? "已加载" : previewState === "loading" ? "加载中" : "等待预览"}</span></div>
          <div className="aspect-video bg-slate-950/70 p-3">{previewUrl ? <img src={previewUrl} alt="当前 Demo 外观预览" className="h-full w-full rounded-lg object-contain" onLoad={() => setPreviewState("ready")} onError={() => { setPreviewState("error"); setPreviewError("预览图片无法显示，不能确认或录制"); }} /> : <div className="flex h-full items-center justify-center text-center text-xs leading-5 text-cs2-text-muted">{previewState === "loading" || previewState === "decoding" ? <LoaderCircle className="h-6 w-6 animate-spin" aria-label="预览加载中" /> : "启动后将在这里显示真实暂停画面"}</div>}</div>
          {previewError && <p className="border-t border-amber-400/20 px-4 py-3 text-xs leading-5 text-cs2-text-primary" role="alert">{previewError}</p>}
        </section>
        <div className="flex flex-wrap gap-2"><button type="button" disabled={running || startInFlightRef.current} onClick={() => { if (running || startInFlightRef.current) return; setInspection(null); setSelectedCandidate(""); setJob(null); setJobError(""); clearPreview(); }} className="inline-flex min-h-11 items-center gap-2 rounded-xl border border-cs2-border px-3 py-2 text-xs font-semibold text-cs2-text-secondary"><RotateCcw className="h-4 w-4" aria-hidden="true" />重新选择</button><button type="button" onClick={cancelJob} disabled={!running} className="inline-flex min-h-11 items-center gap-2 rounded-xl border border-red-400/40 px-3 py-2 text-xs font-semibold text-red-200 disabled:cursor-not-allowed disabled:opacity-40"><Square className="h-4 w-4" aria-hidden="true" />取消专用会话</button></div>
      </aside>
    </div>
    </div>
  </section>;
}
