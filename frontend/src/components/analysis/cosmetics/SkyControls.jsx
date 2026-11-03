import {useEffect,useRef,useState} from 'react';
import {Cloud,Loader2} from 'lucide-react';
import Button from '../../ui/Button.jsx';
import {getSkyOptions,getSkyStatus,startSkyPreview,stopSkyPreview} from '../../../api/cosmeticStudio.js';
const ACTIVE=new Set(['starting','preview','stopping','cleanup_unknown']);
const COLORS={original:'linear-gradient(150deg,#5b6e80,#a4b6c2)',italy:'linear-gradient(155deg,#368dce,#c6e1f2)',dust2:'linear-gradient(155deg,#569ec0,#e9ce98)',night:'linear-gradient(155deg,#141a32,#536478)',overcast:'linear-gradient(155deg,#617080,#adb5bb)',sunset:'linear-gradient(155deg,#816384,#d99367)'};
function message(error){const detail=error?.response?.data?.detail;return typeof detail==='string'?detail:error?.message||'无法确认预览状态。';}
export default function SkyControls({jobId,disabled,onCreateCopy}){
 const [options,setOptions]=useState(null),[sky,setSky]=useState('original'),[tool,setTool]=useState(''),[ack,setAck]=useState(false),[status,setStatus]=useState(null),[error,setError]=useState(''),[sending,setSending]=useState(false);
 const alive=useRef(false),owned=useRef(false),inflight=useRef(false);
 const active=ACTIVE.has(status?.state);
 useEffect(()=>{
  alive.current=true;
  Promise.all([getSkyOptions(),getSkyStatus()]).then(([{data:next},{data:current}])=>{
   if(alive.current){setOptions(next);setTool(next.hlae_exe||'');setStatus(current);}
  }).catch(problem=>{if(alive.current)setError(message(problem));});
  return()=>{alive.current=false;if(owned.current)void stopSkyPreview().catch(()=>{});};
 },[]);
 useEffect(()=>{
  if(!active)return undefined;
  let stopped=false,timer;
  const poll=async()=>{try{const {data:next}=await getSkyStatus();if(!stopped&&alive.current){setStatus(next);setError('');if(!ACTIVE.has(next.state))owned.current=false;}}
   catch(problem){if(!stopped&&alive.current)setError(message(problem));}
   finally{if(!stopped)timer=setTimeout(poll,1500);}};
  timer=setTimeout(poll,1000);return()=>{stopped=true;clearTimeout(timer);};
 },[active]);
 async function preview(){
  if(!jobId||disabled||!ack||!tool||active||inflight.current)return;
  inflight.current=true;setSending(true);setError('');
  try{const {data:next}=await startSkyPreview({job_id:jobId,sky_id:sky,offline_acknowledged:true,hlae_exe:tool});
   owned.current=true;if(alive.current)setStatus(next);else await stopSkyPreview();}
  catch(problem){if(alive.current)setError(message(problem));}
  finally{inflight.current=false;if(alive.current)setSending(false);}
 }
 async function stop(){try{const {data:next}=await stopSkyPreview();if(alive.current)setStatus(next);}catch(problem){if(alive.current)setError(message(problem));}}
 return <section className="cosmetic-sky mgs-glass-panel" aria-labelledby="cosmetic-sky-title">
  <div className="cosmetic-title"><Cloud size={19} aria-hidden="true"/><h2 id="cosmetic-sky-title">天空</h2></div>
  <p>选天空 → 保存上方 Demo 副本 → 预览。只影响专用 HLAE 回放；普通回放仍是原始天空。</p>
  <div className="cosmetic-sky-grid">{options?.presets.map(preset=><button type="button" key={preset.id} aria-pressed={sky===preset.id} disabled={!preset.available||active||sending||disabled} onClick={()=>setSky(preset.id)} className={`cosmetic-sky-choice ${sky===preset.id?'cosmetic-selected':''}`}>
   <span className="cosmetic-sky-swatch" aria-hidden="true" style={{background:COLORS[preset.id]}}/><strong>{preset.label}</strong><small>{preset.available?(sky===preset.id?'已选择':'选择'):'本机未安装资源'}</small>
  </button>)}</div>
  <small>色块为选项示意，不是原生天空截图。天空不会写回 Demo 文件。</small>
  <details className="cosmetic-picker-advanced"><summary>HLAE 设置（只需填写工具路径）</summary><label>HLAE 路径<input value={tool} placeholder="选择已安装的 HLAE.exe" disabled={active||sending} onChange={event=>setTool(event.target.value)}/></label></details>
  {!tool&&options&&<p className="cosmetic-hint">未找到 HLAE，请在上方设置填写已安装的 HLAE.exe；不会自动下载或启动。</p>}
  {!jobId&&<div><p className="cosmetic-hint">请先保存上方皮肤选择；只换天空时，直接生成原始副本。</p><Button variant="secondary" disabled={disabled} onClick={onCreateCopy}>只换天空 · 生成 Demo 副本</Button></div>}
  <div className="cosmetic-playback"><label><input type="checkbox" checked={ack} disabled={active||sending} onChange={event=>setAck(event.target.checked)}/>仅启动专用离线 Demo（-insecure），不接管已有游戏</label>
   {active?<Button variant="secondary" disabled={status?.state==='stopping'||status?.state==='cleanup_unknown'} onClick={stop}>关闭天空预览</Button>:<Button variant="secondary" disabled={!ack||!jobId||!tool||disabled||sending||!options?.presets.some(p=>p.id===sky&&p.available)} onClick={preview}>{sending?<><Loader2 size={15} className="animate-spin" aria-hidden="true"/>正在启动…</>:'预览所选天空'}</Button>}
  </div>
  {status&&status.state!=='idle'&&<p role="status">{status.message}</p>}{error&&<p role="alert" className="cosmetic-error">{error}</p>}
 </section>;
}
