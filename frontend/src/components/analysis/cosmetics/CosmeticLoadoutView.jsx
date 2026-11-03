import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Check, ChevronRight, Loader2, Palette, RotateCcw, ShieldCheck, SlidersHorizontal, X } from 'lucide-react';
import Button from '../../ui/Button.jsx';
import { applyAppearancePlan, cancelAppearanceJob, getAppearanceInventory, getAppearanceJob, playAppearanceDemo, saveAppearanceSettings } from '../../../api/cosmeticStudio.js';
import './cosmetic-loadout.css';
import SkyControls from './SkyControls.jsx';
import QueueWeaponMenu from './QueueWeaponMenu.jsx';
import baseWeapons from '../../../../../backend/app/cosmetic_studio/base_weapon_metadata.json';

const TERMINAL = new Set(['file_verified', 'failed', 'cancelled']);
const MODEL_NAMES = { 507: '爪子刀', 515: '蝴蝶刀', 5030: '运动手套', 5034: '专家手套' };
const EMPTY = [];
const READONLY_REASONS = {
  not_observed: '此阵营未读到这把武器的持有记录。不能对不存在的装备生成替换结果。',
  foreign_pickup: '这是拾取其他玩家的武器，不是目标玩家的原始装备。录制时保留它的原外观。',
  unsupported_source_definition: '此装备暂未完成外观适配',
  no_bounded_holding_evidence: '未找到可确认的持有记录，暂不可替换',
  qualified_combination_requires_karambit: '手套当前仅支持已验证的爪子刀组合',
  owned_glove_identity_unverified: '无法确认此玩家实际佩戴这副手套',
  missing_item_identity: '已读到持有记录，但缺少有效的饰品编号，当前后端暂不能替换',
  unverified_original_ownership: '已读到持有记录，但无法确认原始归属，暂不可替换',
  match_ownership_unknown: '整局中存在无法确认归属的记录，不会生成部分替换结果',
};
function errorText(error) {
  const detail = error?.response?.data?.detail;
  return typeof detail === 'string' ? detail : error?.message || '请求失败，请重试。';
}
function mapPlan(plan = EMPTY) { return Object.fromEntries(plan.map((entry) => [entry.slot_id, entry])); }
function sortedPlan(draft) { return Object.values(draft).sort((a, b) => a.slot_id.localeCompare(b.slot_id)); }
function appearanceResultLabel(result, data) {
  const name = data.presets.find((preset) => preset.id === result.preset_id)?.label
    || data.items.find((item) => item.slot_id === result.slot_id)?.name || '装备外观';
  return `${name}：${result.error || (result.status === 'applied' ? '已应用到制片副本' : result.status === 'failed' ? '未应用成功' : '等待校验')}`;
}
function safeImageUrl(value) {
  if (!value) return '';
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && ['community.cloudflare.steamstatic.com', 'community.akamai.steamstatic.com', 'cdn.cloudflare.steamstatic.com', 'cdn.akamai.steamstatic.com'].includes(url.hostname) ? url.href : '';
  } catch { return ''; }
}

function ItemArtwork({ definition, imageUrl, paintIndex }) {
  const finishUrl=safeImageUrl(imageUrl);
  const baseUrl=baseWeapons.items[String(definition)]?.image_url||'';
  // Five stock models use the pinned image-tracker source rather than Steam.
  // This is not a general-purpose arbitrary GitHub URL permission.
  const pinnedBasePrefix=`https://raw.githubusercontent.com/ByMykel/counter-strike-image-tracker/${baseWeapons.image_tracker_commit}/static/panorama/images/econ/weapons/base_weapons/`;
  const stockUrl=safeImageUrl(baseUrl)||(baseUrl.startsWith(pinnedBasePrefix)&&/^\w+_png\.png$/.test(baseUrl.slice(pinnedBasePrefix.length))?baseUrl:'');
  const url=finishUrl||stockUrl;
  const kind=finishUrl?'finish':paintIndex===0?'stock':'model-reference';
  const [imageState, setImageState] = useState({url:'',loaded:false,failed:false});
  const loaded=imageState.url===url&&imageState.loaded;
  const failed=imageState.url===url&&imageState.failed;
  return <div className="cosmetic-art-shell" data-artwork-kind={kind}>{!loaded&&<div className="cosmetic-art-loading">{url&&!failed?'图片加载中':failed?'图片加载失败':'图片暂不可用'}</div>}
  {url&&!failed&&<img className="cosmetic-art cosmetic-art-image" src={url} style={{opacity:loaded?1:0}} loading="lazy" decoding="async" alt="" width="240" height="100"
    onLoad={()=>setImageState({url,loaded:true,failed:false})} onError={()=>setImageState({url,loaded:false,failed:true})}/>}
  </div>;
}

function AppearancePicker({ item, presets, replacement, onClose, onConfirm, inline=false }) {
  const available = useMemo(() => {
    const compatible = new Set(item.preset_ids || EMPTY);
    return presets.filter((preset) => compatible.has(preset.id));
  }, [item.preset_ids, presets]);
  const [targetId, setTargetId] = useState(replacement?.preset_id || available[0]?.id || '');
  const target = available.find((preset) => preset.id === targetId);
  const [wear, setWear] = useState(String(replacement?.wear ?? Math.max(target?.wear_min ?? 0, Math.min(.01, target?.wear_max ?? 1))));
  const [seed, setSeed] = useState(String(replacement?.seed ?? 0));
  const [search, setSearch] = useState('');
  const [model, setModel] = useState('');
  const dialogRef = useRef(null);
  useEffect(() => {
    if(inline)return undefined;
    const previous = document.activeElement;
    dialogRef.current?.querySelector('input[type="search"]')?.focus();
    return () => { if (previous?.isConnected) previous.focus(); };
  }, [inline]);
  const min = target?.wear_min ?? 0;
  const max = target?.wear_max ?? 1;
  const valid = target && wear.trim() && seed.trim() && Number.isFinite(Number(wear)) && Number(wear) >= min && Number(wear) <= max && Number.isInteger(Number(seed)) && Number(seed) >= 0 && Number(seed) <= 1000;
  const models = [...new Set(available.map((entry) => entry.target_definition))];
  const visible = available.filter((entry) => (!model || String(entry.target_definition) === model) && entry.label.toLowerCase().includes(search.trim().toLowerCase()));
  function choose(entry) {
    setTargetId(entry.id);
    setWear(String(Math.max(entry.wear_min, Math.min(Number(wear) || 0, entry.wear_max))));
  }
  function handleKeys(event) {
    if(inline)return;
    if (event.key === 'Escape') { event.preventDefault(); onClose(); }
    if (event.key !== 'Tab') return;
    const elements = [...dialogRef.current.querySelectorAll('button:not(:disabled),input:not(:disabled),select:not(:disabled),summary')].filter((entry) => !entry.hidden && !entry.closest('details:not([open]) .cosmetic-parameters'));
    const first = elements[0]; const last = elements.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }
  return <div className={inline?'queue-buy-preview':'cosmetic-dialog-scrim'} onMouseDown={(event) => { if (!inline&&event.target === event.currentTarget) onClose(); }}>
    <section ref={dialogRef} role={inline?'region':'dialog'} aria-modal={inline?undefined:true} aria-labelledby="cosmetic-picker-title" className="cosmetic-dialog mgs-glass-panel" onKeyDown={handleKeys}>
      <header><div><h2 id="cosmetic-picker-title">选择饰品外观</h2><p>{inline?'选皮肤 → 装备 → 保存录制外观。':'选中皮肤，点击“装备”。保存 Demo 副本后，才会用于回放。'}</p></div><Button variant="ghost" aria-label="关闭外观选择" onClick={onClose}><X size={18} aria-hidden="true" /></Button></header>
      <div className="cosmetic-picker-body">
        <div className="cosmetic-comparison">
          <div className="cosmetic-comparison-card"><span className="cosmetic-eyebrow">Demo 原始饰品</span><ItemArtwork definition={item.definition} imageUrl={item.image_url} paintIndex={item.paint_index} /><strong>{item.name}</strong><p>{item.paint_index==null?'原外观未确认，图片仅供型号参考':'原始 Demo 保持不变'}</p></div>
          <ChevronRight size={20} className="cosmetic-comparison-arrow" aria-hidden="true" />
          <div className="cosmetic-comparison-card cosmetic-selected"><span className="cosmetic-eyebrow">替换外观</span><ItemArtwork key={targetId} definition={target?.target_definition ?? item.definition} imageUrl={target?.image_url} /><strong>{target?.label || '请选择外观'}</strong>
            <p>已自动设置磨损与图案，可直接装备。</p>
            <details className="cosmetic-picker-advanced"><summary>高级选项</summary><div className="cosmetic-parameters"><label>磨损<input aria-label="磨损数值" type="number" min={min} max={max} step="0.001" value={wear} onChange={(event) => setWear(event.target.value)} /></label><input aria-label="磨损滑块" type="range" min={min} max={max} step="0.001" value={Number(wear) || 0} onChange={(event) => setWear(event.target.value)} /><small>允许范围 {min}–{max}</small>
              <label>图案模板<input aria-label="图案模板数值" type="number" min="0" max="1000" step="1" value={seed} onChange={(event) => setSeed(event.target.value)} /></label><input aria-label="图案模板滑块" type="range" min="0" max="1000" step="1" value={Number(seed) || 0} onChange={(event) => setSeed(event.target.value)} />
            </div></details>
          </div>
        </div>
        <label className="cosmetic-search">搜索外观<input type="search" value={search} placeholder="输入饰品名称" onChange={(event) => setSearch(event.target.value)} /></label>
        {models.length > 1 && <div className="cosmetic-model-filter" aria-label="饰品型号"><Button size="sm" variant={!model ? 'primary' : 'secondary'} aria-pressed={!model} onClick={() => setModel('')}>全部型号</Button>{models.map((definition) => <Button key={definition} size="sm" variant={model === String(definition) ? 'primary' : 'secondary'} aria-pressed={model === String(definition)} onClick={() => setModel(String(definition))}>{MODEL_NAMES[definition] || `型号 ${definition}`}</Button>)}</div>}
        <div className="cosmetic-choice-grid">{visible.map((entry) => <button key={entry.id} type="button" aria-label={`选择 ${entry.label}`} aria-pressed={entry.id === targetId} className={`cosmetic-choice ${entry.id === targetId ? 'cosmetic-selected' : ''}`} onClick={() => choose(entry)}><ItemArtwork definition={entry.target_definition} imageUrl={entry.image_url} /><strong>{entry.label}</strong>{entry.id === targetId && <Check size={16} aria-hidden="true" />}</button>)}</div>
        {!visible.length && <p className="cosmetic-empty">没有符合搜索条件的皮肤。</p>}
        {!valid && <p className="cosmetic-error" role="alert">请填写外观允许范围内的磨损，以及 0–1000 的整数图案模板。</p>}
      </div>
      <footer><Button variant="ghost" onClick={() => onConfirm(null)}><RotateCcw size={15} aria-hidden="true" />恢复本场原皮肤</Button><div><Button variant="secondary" onClick={onClose}>取消</Button><Button disabled={!valid} onClick={() => onConfirm({ slot_id: item.slot_id, preset_id: target.id, wear: Number(wear), seed: Number(seed) })}>装备</Button></div></footer>
    </section>
  </div>;
}

function LoadoutEditor({ data, demoPath, selectedXuid, onArtifact, queueProfile, onQueueSave }) {
  const queueMode=Boolean(onQueueSave);
  const preferred = data.players.find((player) => player.xuid === selectedXuid)?.xuid || data.players[0]?.xuid || '';
  const [xuid, setXuid] = useState(preferred);
  const [plans, setPlans] = useState(data.plans || {});
  const [draft, setDraft] = useState(() => mapPlan(queueMode?(queueProfile?.expected_demo_sha256===data.demo_sha256?queueProfile.replacements:EMPTY):data.plans?.[preferred]));
  const [picker, setPicker] = useState(null);
  const [scope, setScope] = useState('match');
  const [settings, setSettings] = useState(data.settings);
  const [settingsDraft, setSettingsDraft] = useState(data.settings);
  const [settingsBusy, setSettingsBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [job, setJob] = useState(() => {
    const id = data.artifact?.job_id || data.artifact_job_id;
    return data.artifact && id ? { id, state: 'file_verified', artifact: data.artifact } : null;
  });
  const [offlineAck, setOfflineAck] = useState(false);
  const [playBusy, setPlayBusy] = useState(false);
  const [pollError, setPollError] = useState('');
  const alive = useRef(false);
  const activeJob = useRef(null);
  const submitted = useRef(null);
  const submitInFlight = useRef(false);
  const playInFlight = useRef(false);
  const inheritedPlayer = useRef(selectedXuid);
  const fileBusy = saving || Boolean(job && !TERMINAL.has(job.state));
  const busy = fileBusy || playBusy;
  const items = data.items.filter((item) => item.xuid === xuid);
  const displayItems = useMemo(() => {
    const playerItems=data.items.filter(item=>item.xuid===xuid);
    if(scope==='item')return playerItems;
    const groups=new Map();
    for(const item of playerItems.filter(row=>row.editable&&row.preset_ids?.length)){
      const group=groups.get(item.definition);
      if(!group)groups.set(item.definition,{...item,covered_slots:[item.slot_id]});
      else{
        group.covered_slots.push(item.slot_id);
        if(group.team!==item.team)group.team='both';
        const next=new Set(item.preset_ids);group.preset_ids=group.preset_ids.filter(id=>next.has(id));
      }
    }
    return [...groups.values()];
  },[data.items,xuid,scope]);
  const editableCount = items.filter(item => item.editable && item.preset_ids?.length).length;
  const presets = useMemo(() => new Map(data.presets.map((preset) => [preset.id, preset])), [data.presets]);
  const dirty = JSON.stringify(sortedPlan(draft)) !== JSON.stringify(sortedPlan(mapPlan(plans[xuid])));
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (activeJob.current) void cancelAppearanceJob(activeJob.current).catch(() => {});
    };
  }, []);
  function acceptStatus(next) {
    setJob(next);
    if (TERMINAL.has(next.state)) activeJob.current = null;
    if (next.state === 'file_verified') {
      const resolved=next.resolved_plan || submitted.current.replacements;
      setPlans((current) => ({ ...current, [submitted.current.xuid]: resolved }));
      setDraft(mapPlan(resolved));
      setMessage('');
      onArtifact?.(next.artifact);
    }
    if (next.state === 'failed') setError(next.error || '外观应用失败；已保存的方案和原始 Demo 保持不变。');
    if (next.state === 'cancelled') setMessage('已取消，本次草稿未应用。');
  }
  const jobId = job?.id;
  const terminal = Boolean(job && TERMINAL.has(job.state));
  useEffect(() => {
    if (!jobId || terminal) return undefined;
    let stopped = false; let timer; let failures = 0;
    const poll = async () => {
      try {
        const { data: next } = await getAppearanceJob(jobId);
        if (stopped || !alive.current) return;
        failures = 0; setPollError(''); acceptStatus(next);
        if (!TERMINAL.has(next.state)) timer = setTimeout(poll, 750);
      } catch (problem) {
        if (stopped || !alive.current) return;
        setPollError(`无法确认任务状态：${errorText(problem)}。不会标记成功。`);
        // Keep a bounded, low-frequency status probe; never create a second job.
        failures += 1; timer = setTimeout(poll, Math.min(10000, 2000 * failures));
      }
    };
    timer = setTimeout(poll, 350);
    return () => { stopped = true; clearTimeout(timer); };
    // Status handler uses only stable state setters and refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobId, terminal]);
  const selectPlayer = useCallback((next) => {
    setXuid(next); setDraft(mapPlan(plans[next])); setPicker(null);
    setJob(null); setError(''); setMessage(''); setPollError(''); setOfflineAck(false);
  }, [plans]);
  useEffect(() => {
    // A parent analysis-player change replaces the local draft. Defer it while
    // saving/launching so the in-flight action remains tied to its exact XUID.
    if (busy || inheritedPlayer.current === selectedXuid) return;
    inheritedPlayer.current = selectedXuid;
    if (selectedXuid !== xuid && data.players.some(player => player.xuid === selectedXuid)) selectPlayer(selectedXuid);
  }, [busy, data.players, selectedXuid, selectPlayer, xuid]);
  async function save(forceCopy=false) {
    if (busy || (!dirty && forceCopy!==true) || submitInFlight.current) return;
    submitInFlight.current = true;
    setSaving(true); setError(''); setMessage(''); setJob(null); setPollError(''); setOfflineAck(false);
    const payload = { demo_path: demoPath, expected_demo_sha256: data.demo_sha256, xuid, replacements: sortedPlan(draft), scope };
    submitted.current = payload;
    try {
      const { data: next } = await applyAppearancePlan(payload);
      if (!alive.current) { if (!TERMINAL.has(next.state)) void cancelAppearanceJob(next.id).catch(() => {}); return; }
      activeJob.current = TERMINAL.has(next.state) ? null : next.id;
      acceptStatus(next);
    } catch (problem) { if (alive.current) setError(errorText(problem)); }
    finally { submitInFlight.current = false; if (alive.current) setSaving(false); }
  }
  async function saveSettings(event) {
    event.preventDefault(); if (settingsBusy || busy) return;
    setSettingsBusy(true); setError('');
    try {
      const { data: next } = await saveAppearanceSettings(settingsDraft);
      if (alive.current) { setSettings(next); setSettingsDraft(next); setMessage('高级设置已保存，以后无需重复填写。'); }
    } catch (problem) { if (alive.current) setError(errorText(problem)); }
    finally { if (alive.current) setSettingsBusy(false); }
  }
  async function cancel() {
    if (!jobId) return;
    try { const { data: next } = await cancelAppearanceJob(jobId); if (alive.current) acceptStatus(next); }
    catch (problem) { if (alive.current) setPollError(errorText(problem)); }
  }
  async function play() {
    if (!offlineAck || job?.state !== 'file_verified' || busy || dirty || playInFlight.current) return;
    playInFlight.current = true; setPlayBusy(true); setError('');
    try {
      await playAppearanceDemo(job.id);
      if (alive.current) setMessage('已启动专用离线回放。请在原生画面中核对外观；启动成功不等于外观确认。');
    } catch (problem) { if (alive.current) setError(errorText(problem)); }
    finally { playInFlight.current = false; if (alive.current) setPlayBusy(false); }
  }
  function restore(slotId) { setDraft((current) => {
    const next={...current};const item=items.find(row=>row.slot_id===slotId);
    for(const row of items){if(row.slot_id===slotId||(scope==='match'&&item&&row.definition===item.definition))delete next[row.slot_id];}
    if(scope==='match'&&item)for(const [slot,selection] of Object.entries(next))if(presets.get(selection.preset_id)?.source_definition===item.definition)delete next[slot];
    return next;
  }); }
  function itemReason(item) {
    return !item.editable ? READONLY_REASONS[item.reason] || item.reason || '此装备暂未完成适配' : !item.preset_ids?.length ? '暂无兼容皮肤' : '';
  }
  function renderSlot(item) {
    const replacement = draft[item.slot_id];
    const preset = replacement ? presets.get(replacement.preset_id) : null;
    const reason = itemReason(item);
    return <article key={item.slot_id} className={`cosmetic-slot ${preset ? 'cosmetic-selected' : ''}`}><button type="button" aria-label={`更换 ${item.name}`} className="cosmetic-slot-main" disabled={busy || Boolean(reason)} onClick={(event) => { event.currentTarget.focus(); setPicker(item); }}><ItemArtwork key={preset?.id || 'original'} definition={preset?.target_definition ?? item.definition} imageUrl={preset?.image_url || item.image_url} /><strong>{preset?.label || item.name}</strong><span>{preset ? '已选择 · 用于 Demo 副本' : '本场原皮肤'}</span>{scope==='match'&&!reason&&<span>整局同型号 · 自动覆盖重新购买</span>}{!reason && <span className="cosmetic-edit-link">换皮肤 <ChevronRight size={13} aria-hidden="true" /></span>}</button>{preset && !reason && <button className="cosmetic-slot-reset" aria-label={`恢复原始 ${item.name}`} title="恢复本场原皮肤" disabled={busy} onClick={() => restore(item.slot_id)}><RotateCcw size={15} aria-hidden="true" /></button>}{reason && <p className="cosmetic-readonly">{reason}</p>}</article>;
  }
  const readonlyItems = items.filter(item => itemReason(item));
  if(queueMode){
    const menuItems=[...displayItems,...readonlyItems];
    return <section className="cosmetic-loadout queue-loadout">
      <header><h2>{data.players.find(player=>player.xuid===xuid)?.name} · 录制装备</h2><p className="cosmetic-hint">仅影响所选录制片段；原始 Demo、分析数据和 Steam 库存不变。拾取他人的枪保留原外观。</p></header>
      {queueProfile&&queueProfile.expected_demo_sha256!==data.demo_sha256&&<p role="alert">Demo 已变化，旧外观未载入，请重新选择。</p>}
      <div className="queue-loadout-layout"><QueueWeaponMenu items={menuItems} selected={picker} onSelect={setPicker} reason={itemReason} draft={draft} presets={presets} renderArtwork={(item,preset)=><ItemArtwork definition={item.definition} imageUrl={preset?.image_url||item.image_url} paintIndex={preset?.paint_index??item.paint_index}/>}/>
      {picker&&itemReason(picker)?<aside className="queue-buy-placeholder queue-buy-unavailable" role="status"><ItemArtwork definition={picker.definition} imageUrl={picker.image_url}/><h3>{picker.name}</h3><p>{itemReason(picker)}</p><p>当前保留原外观，不会把未验证的替换带入录制。</p></aside>:picker?<AppearancePicker key={picker.slot_id} inline item={picker} presets={data.presets} replacement={draft[picker.slot_id]} onClose={()=>setPicker(null)} onConfirm={replacement=>{
        if(replacement)setDraft(current=>({...current,...Object.fromEntries((picker.covered_slots||[picker.slot_id]).map(slot=>[slot,{...replacement,slot_id:slot}]))}));else restore(picker.slot_id);
      }}/>:<aside className="queue-buy-placeholder"><Palette size={40}/><h3>从左侧选择武器</h3><p>这里会展示原皮肤、替换皮肤和可选外观。</p></aside>}</div>
      <div className="cosmetic-save-bar"><div><strong>已选择 {Object.keys(draft).length} 件装备</strong><p className="cosmetic-hint">点击开始录制后自动生成并校验副本；失败会停止，不会录成原皮肤。</p></div><div className="cosmetic-save-actions"><Button variant="secondary" onClick={()=>onQueueSave(null)}>恢复原皮肤</Button><Button onClick={()=>onQueueSave(Object.keys(draft).length?{expected_demo_sha256:data.demo_sha256,replacements:sortedPlan(draft),scope:'match'}:null)}>保存录制外观</Button></div></div>
    </section>;
  }
  return <section className="cosmetic-loadout">
    <header className="cosmetic-toolbar mgs-glass-panel"><div><div className="cosmetic-title"><Palette size={19} aria-hidden="true" /><h2>玩家装备</h2></div><p>选玩家 → 点武器 → 选皮肤 → 保存，整局使用</p></div><label>目标玩家<select value={xuid} disabled={busy} onChange={(event) => selectPlayer(event.target.value)}>{data.players.map((player) => <option key={player.xuid} value={player.xuid}>{player.name || '未命名玩家'}</option>)}</select></label></header>
    <div className="cosmetic-scope"><strong>{scope==='match'?'整局应用':'逐件编辑'}</strong><span>{scope==='match'?'自动查找整份 Demo 中该玩家拥有的同型号装备，不限回合；拾取他人的枪不变。':'仅替换所选装备编号，不代表整局同型号枪械。'}</span></div>
    <p className="cosmetic-hint" role="status">{editableCount ? `可更换 ${editableCount} 件装备，直接点击下面的武器。` : '当前玩家暂无可编辑装备；展开“暂不支持替换”查看本场装备与原因。'}</p>
    {items.some((item) => item.item_id === '0' && item.definition === 61 && item.editable) && <p className="cosmetic-hint">这份 Demo 的 USP-S 现在可生成替换副本；其他未适配装备仍不可编辑。</p>}
    {[["ct", "CT 装备"], ["t", "T 装备"], ["both", "CT/T 通用装备"], ["unknown", "未确认阵营的装备"]].map(([team, label]) => {
      const group = displayItems.filter((item) => !itemReason(item) && (['ct', 't', 'both'].includes(item.team) ? item.team : 'unknown') === team);
      if (['both', 'unknown'].includes(team) && !group.length) return null;
      return <section className="cosmetic-team" key={team}><h3>{label}<span>{group.length} 件可更换</span></h3><div className="cosmetic-slot-grid">{group.map(renderSlot)}{!group.length && <div className="cosmetic-empty-slot">此阵营暂无可更换装备</div>}</div></section>;
    })}
    {readonlyItems.length > 0 && <details className="cosmetic-readonly-group cosmetic-settings"><summary>暂不支持替换 · {readonlyItems.length} 件本场装备</summary><p>保留本场读取到的原始外观；暂时只展示，具体原因见装备卡片。</p><div className="cosmetic-slot-grid">{readonlyItems.map(renderSlot)}</div></details>}
    <div className="cosmetic-save-bar"><div><strong>{dirty ? <span className="cosmetic-draft">待保存</span> : '点武器选择皮肤'}</strong><p className="cosmetic-hint">只生成独立 Demo 副本，不改原文件或 Steam 库存，不自动启动游戏。</p></div><div className="cosmetic-save-actions">{dirty && <Button variant="secondary" disabled={busy} onClick={() => { setDraft(mapPlan(plans[xuid])); setPicker(null); }}>撤销本次选择</Button>}<Button disabled={!dirty || busy || settingsBusy || !settings?.core_exe || !settings?.output_dir} onClick={save}>{fileBusy ? <><Loader2 size={16} className="animate-spin" aria-hidden="true" />正在保存…</> : '保存 Demo 副本'}</Button>{fileBusy && <Button variant="secondary" onClick={cancel} disabled={!jobId}>取消任务</Button>}</div></div>
    {error && <p className="cosmetic-error" role="alert">{error}</p>}
    {pollError && <p className="cosmetic-error" role="alert">{pollError}</p>}
    {message && <p role="status" className="cosmetic-hint">{message}</p>}
    {job?.skipped_foreign_items>0&&<p className="cosmetic-hint">已保留 {job.skipped_foreign_items} 件拾取他人的装备，未修改其他玩家外观。</p>}
    <SkyControls jobId={job?.state==='file_verified'?job.id:null} disabled={busy||dirty} onCreateCopy={()=>save(true)} />
    {job && <div className="cosmetic-result" role="status"><ShieldCheck size={21} aria-hidden="true" /><div>{job.state === 'file_verified' ? <><strong>制片副本已校验，原始 Demo 未修改。</strong><p>尚未确认原生回放画面；录制需要单独进行预览确认。</p><details><summary>副本信息</summary><p className="cosmetic-path">{job.artifact?.playback || job.artifact?.playback_path || demoPath}</p></details><div className="cosmetic-playback"><label><input type="checkbox" checked={offlineAck} disabled={playBusy || dirty || busy} onChange={(event) => setOfflineAck(event.target.checked)} />只启动专用离线 Demo 回放（-insecure）</label><Button variant="secondary" disabled={!offlineAck || playBusy || dirty || busy} onClick={play}>{playBusy ? '正在启动离线回放…' : '播放制片副本'}</Button></div></> : <><strong>{job.state === 'failed' ? '未应用成功' : job.state === 'cancelled' ? '任务已取消' : '正在生成并校验制片副本…'}</strong><p>不会自动启动游戏或录制。</p></>}{job.items?.map((item, index) => <p key={item.slot_id || index}>{appearanceResultLabel(item, data)}</p>)}</div></div>}
    <details className="cosmetic-settings mgs-glass-panel"><summary><SlidersHorizontal size={15} aria-hidden="true" />高级设置</summary><label>应用范围<select value={scope} disabled={busy} onChange={event=>{setScope(event.target.value);setPicker(null);}}><option value="match">整局应用（推荐）</option><option value="item">逐件编辑（仅当前装备编号）</option></select></label><p>外观工具与制片副本的保存位置。只需配置一次；回放、HLAE 与 OBS 设置不影响饰品编辑。</p><form onSubmit={saveSettings}><label>外观工具路径<input value={settingsDraft?.core_exe || ''} disabled={busy || settingsBusy} onChange={(event) => setSettingsDraft((current) => ({ ...current, core_exe: event.target.value }))} /></label><label>制片副本目录<input value={settingsDraft?.output_dir || ''} disabled={busy || settingsBusy} onChange={(event) => setSettingsDraft((current) => ({ ...current, output_dir: event.target.value }))} /></label><Button type="submit" variant="secondary" disabled={busy || settingsBusy || !settingsDraft?.core_exe || !settingsDraft?.output_dir}>{settingsBusy ? '正在保存…' : '保存设置'}</Button></form></details>
    <p className="cosmetic-footnote">仅用于专用离线 Demo 制片，不改变 Steam 库存、在线对局或分析统计。枪皮按武器与合法磨损范围筛选；新增皮肤需在原生回放中确认实际效果。图片是饰品示意，不代表本场的具体磨损或模板。</p>
    {picker && <AppearancePicker item={picker} presets={data.presets} replacement={draft[picker.slot_id]} onClose={() => setPicker(null)} onConfirm={(replacement) => {
      if(replacement)setDraft(current=>{const next={...current};const slots=new Set(picker.covered_slots||[picker.slot_id]);
        if(scope==='match')for(const [slot,selection] of Object.entries(current))if(presets.get(selection.preset_id)?.source_definition===picker.definition)slots.add(slot);
        for(const slot of slots)next[slot]={...replacement,slot_id:slot};return next;});
      else restore(picker.slot_id);setPicker(null);
    }} />}
  </section>;
}

export default function CosmeticLoadoutView({ demoPath, selectedXuid, onArtifact, queueProfile, onQueueSave }) {
  const [snapshot, setSnapshot] = useState(null);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let stale = false;
    setSnapshot(null); setError('');
    if (demoPath) (selectedXuid ? getAppearanceInventory(demoPath,selectedXuid) : getAppearanceInventory(demoPath)).then(({ data }) => { if (!stale) setSnapshot({ path: demoPath, data }); }).catch((problem) => { if (!stale) setError(errorText(problem)); });
    return () => { stale = true; };
  }, [demoPath, selectedXuid, retry]);
  if (!demoPath) return <div className="cosmetic-empty"><Palette size={28} aria-hidden="true" /><h2>先选择一个 Demo</h2><p>饰品直接使用当前 Demo 和玩家，不需要再次上传或填写玩家 ID。</p></div>;
  if (error) return <div className="cosmetic-empty"><p role="alert">{error}</p><Button variant="secondary" onClick={() => setRetry((value) => value + 1)}>重新读取装备</Button></div>;
  if (!snapshot || snapshot.path !== demoPath) return <div className="cosmetic-empty" role="status"><Loader2 size={23} className="animate-spin" aria-hidden="true" /><p>正在读取当前 Demo 的玩家装备…</p></div>;
  if (onQueueSave && !snapshot.data.players.some(player=>player.xuid===selectedXuid)) return <p role="alert">未在 Demo 中确认录制玩家，未使用其他玩家代替。</p>;
  return <LoadoutEditor key={`${demoPath}:${snapshot.data.demo_sha256}:${selectedXuid||''}`} data={snapshot.data} demoPath={demoPath} selectedXuid={selectedXuid} onArtifact={onArtifact} queueProfile={queueProfile} onQueueSave={onQueueSave}/>;
}
