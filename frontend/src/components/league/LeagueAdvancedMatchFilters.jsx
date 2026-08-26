import { Braces, FolderPlus, Plus, Trash2 } from "lucide-react";

const FIELDS=[
  ["champion_name","英雄名称"],["champion_id","英雄 ID"],["game_mode","游戏模式"],["game_type","游戏类型"],["position","位置"],["role","角色"],["queue_id","队列 ID"],
  ["win","胜利"],["is_remake","重开局"],["is_matched_game","匹配对局"],["is_pve_game","人机对局"],["duration_seconds","游戏时长（秒）"],["played_at","比赛时间戳"],
  ["kills","击杀"],["deaths","死亡"],["assists","助攻"],["kda","KDA"],["kill_participation","参团率"],["solo_kills","单杀"],["double_kills","双杀"],["triple_kills","三杀"],["quadra_kills","四杀"],["penta_kills","五杀"],
  ["damage","英雄伤害"],["damage_taken","承受伤害"],["tower_damage","防御塔伤害"],["healing","治疗与护盾"],["time_ccing","控制次数"],["vision_score","视野分"],
  ["gold","获得金币"],["gold_spent","花费金币"],["cs","补刀"],["level","等级"],["has_item","包含装备 ID"],["has_spell","包含召唤师技能 ID"],["has_perk","包含符文 ID"],["has_augment","包含强化符文 ID"],
];
const OPS=[["eq","等于"],["neq","不等于"],["contains","包含"],["gte","大于等于"],["lte","小于等于"]];
const SCOPES=[["self","当前玩家"],["any-allies","任一队友"],["every-allies","全部队友"],["any-enemies","任一敌人"],["every-enemies","全部敌人"],["any-all","任意其他玩家"],["every-all","全部其他玩家"]];
const newRule=()=>({type:"rule",scope:"self",field:"kills",operator:"gte",value:""});
const newGroup=()=>({type:"group",logic:"and",negate:false,children:[]});

function GroupEditor({node,onChange,onRemove,root=false,depth=0}) {
  const patchChild=(index,next)=>onChange({...node,children:node.children.map((child,i)=>i===index?next:child)});
  const removeChild=(index)=>onChange({...node,children:node.children.filter((_,i)=>i!==index)});
  return <div className={`${root?"":"ml-3"} rounded-xl border ${depth%2?"border-violet-400/20":"border-cyan-400/20"} bg-black/[.06] p-3`}>
    <div className="flex flex-wrap items-center gap-2"><Braces className="h-4 w-4 text-cyan-300"/><select aria-label={`第 ${depth+1} 层逻辑`} value={node.logic||"and"} onChange={(e)=>onChange({...node,logic:e.target.value})} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-1.5 text-xs"><option value="and">全部满足 AND</option><option value="or">任一满足 OR</option></select><label className="text-xs"><input type="checkbox" checked={Boolean(node.negate)} onChange={(e)=>onChange({...node,negate:e.target.checked})}/> 结果取反 NOT</label><button type="button" onClick={()=>onChange({...node,children:[...(node.children||[]),newRule()]})} className="ml-auto rounded-lg border border-cyan-400/25 px-2 py-1.5 text-xs text-cyan-200"><Plus className="mr-1 inline h-3.5 w-3.5"/>条件</button><button type="button" onClick={()=>onChange({...node,children:[...(node.children||[]),newGroup()]})} className="rounded-lg border border-violet-400/25 px-2 py-1.5 text-xs text-violet-200"><FolderPlus className="mr-1 inline h-3.5 w-3.5"/>条件组</button>{!root&&<button type="button" aria-label="删除条件组" onClick={onRemove} className="rounded-lg p-1.5 text-rose-300"><Trash2 className="h-4 w-4"/></button>}</div>
    <div className="mt-2 space-y-2">{(node.children||[]).map((child,index)=>child.type==="group"?<GroupEditor key={index} node={child} depth={depth+1} onChange={(next)=>patchChild(index,next)} onRemove={()=>removeChild(index)}/>:<div key={index} className="grid gap-2 md:grid-cols-[1fr_1.2fr_1fr_1.2fr_auto]"><select aria-label={`条件作用域 ${index+1}`} value={child.scope||"self"} onChange={(e)=>patchChild(index,{...child,scope:e.target.value})} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-2 text-xs">{SCOPES.map(([id,label])=><option key={id} value={id}>{label}</option>)}</select><select value={child.field} onChange={(e)=>patchChild(index,{...child,field:e.target.value})} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-2 text-xs">{FIELDS.map(([id,label])=><option key={id} value={id}>{label}</option>)}</select><select value={child.operator} onChange={(e)=>patchChild(index,{...child,operator:e.target.value})} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-2 py-2 text-xs">{OPS.map(([id,label])=><option key={id} value={id}>{label}</option>)}</select><input value={child.value} onChange={(e)=>patchChild(index,{...child,value:e.target.value})} placeholder="比较值" className="min-w-0 rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><button type="button" aria-label="删除条件" onClick={()=>removeChild(index)} className="rounded-lg px-2 text-rose-300 hover:bg-rose-400/10"><Trash2 className="h-4 w-4"/></button></div>)}</div>
  </div>;
}

export default function LeagueAdvancedMatchFilters({tree,onChange}) {
  const value=tree?.type==="group"?tree:newGroup();
  return <section className="rounded-xl border border-cs2-border bg-cs2-bg-elevated p-3"><div className="mb-2"><b className="text-sm">嵌套组合筛选器</b><span className="ml-2 text-[10px] text-cs2-text-muted">支持 AND / OR / NOT 与当前玩家、队友、敌人任意层级组合</span></div><GroupEditor root node={value} onChange={onChange}/></section>;
}
