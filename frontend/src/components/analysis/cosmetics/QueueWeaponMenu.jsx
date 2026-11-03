import {useState} from 'react';

// Stable catalogue positions, not an unbounded list of Demo item instances.
const GROUPS=[['手枪',[[61,'USP-S'],[32,'P2000'],[4,'格洛克18'],[2,'双持贝瑞塔'],[3,'FN57'],[30,'Tec-9'],[36,'P250'],[63,'CZ75'],[1,'沙漠之鹰'],[64,'R8左轮']],2],
 ['冲锋枪',[[17,'MAC-10'],[34,'MP9'],[33,'MP7'],[23,'MP5-SD'],[24,'UMP-45'],[19,'P90'],[26,'PP-野牛']],1],
 ['中型武器',[[35,'Nova'],[25,'XM1014'],[27,'MAG-7'],[29,'截短霰弹枪'],[14,'M249'],[28,'内格夫']],1],
 ['步枪',[[10,'FAMAS'],[13,'加利尔AR'],[16,'M4A4'],[60,'M4A1-S'],[7,'AK-47'],[8,'AUG'],[39,'SG 553'],[40,'SSG 08'],[9,'AWP'],[38,'SCAR-20'],[11,'G3SG1']],2]];

export default function QueueWeaponMenu({items, selected, onSelect, reason, renderArtwork, draft, presets}){
 const [side,setSide]=useState('ct');
 const visible=items.filter(item=>item.team===side||item.team==='both'||item.team==='unknown');
 const pick=rows=>rows.find(item=>!reason(item))||rows[0];
 function tile(item,label,definition){
  const replacement=draft[item.slot_id]||(item.covered_slots||[]).map(slot=>draft[slot]).find(Boolean);
  const preset=replacement&&presets.get(replacement.preset_id),blocked=reason(item);
  return <button type="button" key={definition||label} data-weapon-definition={definition} className={blocked?'queue-weapon-unavailable':''}
    aria-label={`${blocked?'查看':'更换'} ${item.name}`} aria-pressed={selected?.slot_id===item.slot_id}
    title={`${label}：${blocked||preset?.label||'点击选择皮肤'}`} onClick={()=>onSelect(item)}>
    <strong>{label}</strong>{renderArtwork(item,preset)}<small>{preset?'已装备':blocked?item.reason==='not_observed'?'未使用':'不可替换':'换皮肤'}</small>
  </button>;
 }
 return <div className="queue-buy-menu">
  <div className="queue-buy-sides" aria-label="装备阵营">{[['ct','CT'],['t','T']].map(([value,label])=><button type="button" key={value} aria-pressed={side===value} onClick={()=>{setSide(value);onSelect(null);}}>{label}</button>)}<span>全部枪械 · 点亮的武器可以换皮肤</span></div>
  <div className="queue-buy-columns">{GROUPS.map(([label,definitions,columns])=><section key={label} aria-label={label} style={{gridColumn:`span ${columns}`}}>
   <h3>{label}</h3><div className="queue-buy-slots" style={{gridTemplateColumns:`repeat(${columns},minmax(0,1fr))`}}>
    {definitions.map(([definition,name])=>{
     const item=pick(visible.filter(row=>row.definition===definition))||{slot_id:`catalog:${side}:${definition}`,definition,name,team:side,editable:false,preset_ids:[],reason:'not_observed'};
     return tile(item,name,definition);
    })}
   </div>
  </section>)}<section aria-label="刀／手套"><h3>刀／手套</h3><div className="queue-buy-slots">
   {[[false,'刀'],[true,'手套']].map(([glove,label])=>{
    const rows=visible.filter(item=>glove?item.definition>=5000:item.definition===42||item.definition===59||(item.definition>=500&&item.definition<600));
    const item=pick(rows)||{slot_id:`catalog:${side}:${label}`,definition:glove?5029:42,name:label,team:side,editable:false,preset_ids:[],reason:'not_observed'};
    return tile(item,label);
   })}
  </div></section></div>
 </div>;
}
