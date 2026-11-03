import {lazy, Suspense, useEffect, useRef, useState} from 'react';
import {createPortal} from 'react-dom';
import {useRecordingQueue} from '../../stores/recordingQueueStore';
import '../analysis/cosmetics/cosmetic-loadout.css';

const Loadout = lazy(() => import('../analysis/cosmetics/CosmeticLoadoutView.jsx'));

export default function QueueAppearancePanel({item, disabled = false}) {
  const [open, setOpen] = useState(false);
  const [share, setShare] = useState(false);
  const edit = useRef(null);
  const dialog = useRef(null);
  const save = useRecordingQueue(s => s.updateItemAppearance);
  const available = Boolean(item?.demoPath && item?.targetSteamId);
  useEffect(() => {setOpen(false); setShare(false);}, [item?.id, disabled]);
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement;
    dialog.current?.focus();
    return () => previous?.isConnected && previous.focus();
  }, [open]);
  function keys(event) {
    if(event.key === 'Escape') {event.preventDefault(); event.stopPropagation(); setOpen(false);}
    if(event.key !== 'Tab') return;
    const nodes = [...dialog.current.querySelectorAll('button:not(:disabled),input:not(:disabled),select:not(:disabled),[tabindex="0"]')]
      .filter(node => node.getClientRects().length > 0);
    if(!nodes.length) {event.preventDefault(); return;}
    if(event.shiftKey && (document.activeElement === nodes[0] || document.activeElement === dialog.current)) {
      event.preventDefault(); nodes.at(-1).focus();
    } else if(!event.shiftKey && document.activeElement === nodes.at(-1)) {event.preventDefault(); nodes[0].focus();}
  }
  return <section className="queue-appearance-panel">
    <h3>录制皮肤</h3>
    <p>{item?.appearance ? `已选择 ${new Set(item.appearance.replacements.map(r=>r.preset_id)).size} 种皮肤` : '保留本场原皮肤'}</p>
    <button ref={edit} type="button" disabled={disabled || !available} onClick={()=>setOpen(true)}>选择武器皮肤</button>
    {item?.appearance && <button type="button" disabled={disabled} onClick={()=>save(item.id, null)}>恢复原皮肤</button>}
    {!available && <p>先从 Demo 分析中选择玩家并添加片段，再设置皮肤。</p>}
    <p>仅影响这个录制片段，不修改原始 Demo 或库存。</p>
    {open && createPortal(<div className="queue-appearance-backdrop">
      <section ref={dialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label="录制武器皮肤" className="queue-appearance-dialog" onKeyDown={keys}>
        <header><div><h2>选择录制皮肤</h2><p>选阵营 → 点武器 → 选皮肤 → 装备 → 保存</p></div><button type="button" aria-label="取消皮肤编辑" onClick={()=>setOpen(false)}>关闭</button></header>
        <label className="queue-share-choice"><input type="checkbox" checked={share} onChange={e=>setShare(e.target.checked)}/>同时用于队列里同一 Demo、同一玩家的其他片段</label>
        <div className="queue-appearance-scroll"><Suspense fallback={<p role="status">正在打开装备菜单…</p>}>
          <Loadout demoPath={item.demoPath} selectedXuid={String(item.targetSteamId)} queueProfile={item.appearance}
            onQueueSave={profile=>{save(item.id,profile,share);setOpen(false);}}/>
        </Suspense></div>
      </section>
    </div>, document.body)}
  </section>;
}
