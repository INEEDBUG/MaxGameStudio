import { useMemo, useState } from "react";
import { Clock3, Eye, Pin, PinOff, Search, Users, X } from "lucide-react";
import { getLeagueProfileIconUrl } from "../../api/api";
import { maskLeagueName } from "../../utils/leagueStreamerMode";

const STATUS_TONE = {
  chat: "bg-emerald-400",
  dnd: "bg-cyan-400",
  away: "bg-rose-400",
  offline: "bg-slate-500",
};

function visibleName(row, index, streamerMode, useAliases) {
  return streamerMode
    ? maskLeagueName(row.game_name, index, useAliases, row.puuid)
    : (row.game_name || "未知玩家");
}

export default function LeaguePlayerSearchBrowser({
  history = [],
  friends = [],
  streamerMode = false,
  useAliases = false,
  onOpen,
  onPin,
  onDelete,
  onSpectate,
}) {
  const [historyQuery, setHistoryQuery] = useState("");
  const [friendQuery, setFriendQuery] = useState("");
  const filteredHistory = useMemo(() => history.filter((row) => {
    const query = historyQuery.trim().toLocaleLowerCase();
    return !query || `${row.game_name || ""}#${row.tag_line || ""}`.toLocaleLowerCase().includes(query);
  }), [history, historyQuery]);
  const filteredFriends = useMemo(() => friends.filter((row) => {
    const query = friendQuery.trim().toLocaleLowerCase();
    return !query || `${row.game_name || ""}#${row.tag_line || ""}`.toLocaleLowerCase().includes(query);
  }), [friends, friendQuery]);

  const filterInput = (label, value, onChange) => <label className="relative block min-w-0 flex-1">
    <Search className="absolute left-2 top-1.5 h-3.5 w-3.5 text-cs2-text-muted"/>
    <input aria-label={label} value={streamerMode ? "" : value} disabled={streamerMode} onChange={(event) => onChange(event.target.value)} placeholder={streamerMode ? "隐私模式" : "筛选"} className="w-full rounded-lg border border-cs2-border bg-cs2-bg-input py-1 pl-7 pr-2 text-[11px] disabled:opacity-50"/>
  </label>;

  return <section className="grid overflow-hidden rounded-2xl border border-cs2-border bg-cs2-bg-elevated lg:grid-cols-2">
    <div className="min-w-0 border-b border-cs2-border lg:border-b-0 lg:border-r">
      <header className="flex items-center gap-2 border-b border-cs2-border-subtle p-3 text-xs font-semibold"><Clock3 className="h-4 w-4 text-cyan-300"/><span>最近访问（{history.length}）</span>{filterInput("筛选最近访问", historyQuery, setHistoryQuery)}</header>
      <div className="max-h-56 overflow-y-auto p-2">
        {filteredHistory.map((row, index) => <div key={`${row.server_id || "local"}:${row.puuid}`} className="group flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-white/[.04]">
          <button type="button" onClick={() => onOpen(row.puuid, row.server_id || "")} className="flex min-w-0 flex-1 items-center gap-2 text-left">
            <img src={getLeagueProfileIconUrl(row.profile_icon_id || 29)} alt="召唤师头像" className="h-8 w-8 rounded-full object-cover"/>
            <span className="min-w-0"><b className="block truncate text-xs">{visibleName(row, index, streamerMode, useAliases)}</b>{!streamerMode ? <span className="block truncate text-[10px] text-cs2-text-muted">#{row.tag_line || "—"}{row.server_id ? ` · ${row.server_id}` : ""}</span> : null}</span>
          </button>
          <button type="button" aria-label={row.pinned ? "取消置顶最近访问" : "置顶最近访问"} onClick={() => onPin(row.puuid, !row.pinned, row.server_id || "")} className={`rounded p-1 ${row.pinned ? "text-amber-300" : "text-cs2-text-muted opacity-0 group-hover:opacity-100"}`}>{row.pinned ? <PinOff className="h-3.5 w-3.5"/> : <Pin className="h-3.5 w-3.5"/>}</button>
          <button type="button" aria-label="删除最近访问" onClick={() => onDelete(row.puuid, row.server_id || "")} className="rounded p-1 text-cs2-text-muted opacity-0 hover:text-rose-300 group-hover:opacity-100"><X className="h-3.5 w-3.5"/></button>
        </div>)}
        {!filteredHistory.length ? <div className="py-8 text-center text-xs text-cs2-text-muted">暂无最近访问</div> : null}
      </div>
    </div>
    <div className="min-w-0">
      <header className="flex items-center gap-2 border-b border-cs2-border-subtle p-3 text-xs font-semibold"><Users className="h-4 w-4 text-emerald-300"/><span>好友（{friends.length}）</span>{filterInput("筛选好友", friendQuery, setFriendQuery)}</header>
      <div className="max-h-56 overflow-y-auto p-2">
        {filteredFriends.map((row, index) => <div key={row.puuid} className="group flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-white/[.04]">
          <button type="button" onClick={() => onOpen(row.puuid, "")} className="flex min-w-0 flex-1 items-center gap-2 text-left">
            <span className="relative h-8 w-8 shrink-0"><img src={getLeagueProfileIconUrl(row.profile_icon_id || 29)} alt="好友头像" className="h-8 w-8 rounded-full object-cover"/><span className={`absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-cs2-bg-elevated ${STATUS_TONE[row.availability] || STATUS_TONE.offline}`}/></span>
            <span className="min-w-0"><b className="block truncate text-xs">{visibleName(row, index, streamerMode, useAliases)}</b>{!streamerMode ? <span className="block truncate text-[10px] text-cs2-text-muted">#{row.tag_line || "—"} · {row.game_status || row.availability}</span> : null}</span>
          </button>
          {row.spectatable ? <button type="button" aria-label={`观战 ${row.game_name || "好友"}`} onClick={() => onSpectate(row.puuid)} className="inline-flex items-center gap-1 rounded-lg border border-cyan-400/25 bg-cyan-400/10 px-2 py-1 text-[10px] font-semibold text-cyan-200"><Eye className="h-3.5 w-3.5"/>观战</button> : null}
        </div>)}
        {!filteredFriends.length ? <div className="py-8 text-center text-xs text-cs2-text-muted">暂无好友</div> : null}
      </div>
    </div>
  </section>;
}
