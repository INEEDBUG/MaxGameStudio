import { ChevronLeft, ChevronRight, Download, Edit3, RefreshCw, Search, Trash2, Upload, UserRoundSearch } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { deleteLeaguePlayerTag, fetchLeaguePlayerTags, importLeaguePlayerTags, updateLeaguePlayerTag } from "../../api/leagueLabApi";
import { getLeagueProfileIconUrl } from "../../api/api";

const EXPORT_FORMAT = "max-game-studio/league-player-tags";
const LEGACY_EXPORT_FORMAT = "cs2-ultimate-insight-studio/league-player-tags";
const MAX_IMPORT_BYTES = 1024 * 1024;

export function buildLeaguePlayerTagsExport(rows, exportedAt = new Date().toISOString()) {
  return {
    format: EXPORT_FORMAT,
    schema_version: 1,
    exported_at: exportedAt,
    rows: (rows || []).map((row) => ({
      owner_puuid: String(row.owner_puuid || ""),
      puuid: String(row.puuid || ""),
      label: String(row.tag?.label || "").slice(0, 40),
      note: String(row.tag?.note || "").slice(0, 500),
      color: String(row.tag?.color || "emerald").slice(0, 24),
    })).filter((row) => row.puuid),
  };
}

export function parseLeaguePlayerTagsImport(text) {
  let document;
  try {
    document = JSON.parse(text);
  } catch {
    throw new Error("标签文件不是有效的 JSON");
  }
  if (![EXPORT_FORMAT, LEGACY_EXPORT_FORMAT].includes(document?.format) || Number(document.schema_version) !== 1 || !Array.isArray(document.rows)) {
    throw new Error("无法识别该 League 玩家标签文件");
  }
  if (document.rows.length > 1000) throw new Error("标签文件超过 1000 条，已拒绝导入");
  return document.rows.map((row) => ({
    owner_puuid: String(row?.owner_puuid || "").slice(0, 200),
    puuid: String(row?.puuid || "").slice(0, 200),
    label: String(row?.label || "").slice(0, 40),
    note: String(row?.note || "").slice(0, 500),
    color: String(row?.color || "emerald").slice(0, 24),
  })).filter((row) => row.puuid && (row.label || row.note));
}

function downloadJson(payload, filename) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(payload, null, 2)], { type: "application/json;charset=utf-8" }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}

export default function LeaguePlayerTagManager({ streamerMode = false, onOpenPlayer, onError }) {
  const [rows, setRows] = useState([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [query, setQuery] = useState("");
  const [currentOnly, setCurrentOnly] = useState(true);
  const [busy, setBusy] = useState(false);
  const [revealed, setRevealed] = useState(false);
  const [editing, setEditing] = useState(null);
  const inputRef = useRef(null);
  const pageSize = 20;

  const load = useCallback(async (targetPage = page) => {
    setBusy(true);
    try {
      const result = await fetchLeaguePlayerTags({ page: targetPage, pageSize, query, currentAccountOnly: currentOnly });
      setRows(result.rows || []);
      setTotal(Number(result.total || 0));
      setPage(Number(result.page || targetPage));
    } catch (error) {
      onError?.(error?.response?.data?.detail || "玩家标签读取失败");
    } finally {
      setBusy(false);
    }
  }, [currentOnly, onError, page, query]);

  useEffect(() => {
    if (streamerMode && !revealed) return;
    void load(1);
  }, [currentOnly, revealed, streamerMode]);
  useEffect(() => { if (!streamerMode) setRevealed(false); }, [streamerMode]);

  const saveEdit = async () => {
    if (!editing) return;
    setBusy(true);
    try {
      await updateLeaguePlayerTag(editing.key, editing.tag);
      setEditing(null);
      await load(page);
    } catch (error) {
      onError?.(error?.response?.data?.detail || "玩家标签保存失败");
    } finally {
      setBusy(false);
    }
  };

  const remove = async (row) => {
    if (!window.confirm(`删除“${row.tag?.label || row.puuid.slice(0, 8)}”的本地标签？此操作不会影响 Riot 账号。`)) return;
    setBusy(true);
    try {
      await deleteLeaguePlayerTag(row.key);
      await load(rows.length === 1 && page > 1 ? page - 1 : page);
    } catch (error) {
      onError?.(error?.response?.data?.detail || "玩家标签删除失败");
    } finally {
      setBusy(false);
    }
  };

  const exportAll = async () => {
    setBusy(true);
    try {
      const collected = [];
      for (let index = 1; index <= 10; index += 1) {
        const result = await fetchLeaguePlayerTags({ page: index, pageSize: 100, query, currentAccountOnly: currentOnly });
        collected.push(...(result.rows || []));
        if (collected.length >= Number(result.total || 0) || !(result.rows || []).length) break;
      }
      downloadJson(buildLeaguePlayerTagsExport(collected), `league-player-tags-${new Date().toISOString().slice(0, 10)}.json`);
    } catch (error) {
      onError?.(error?.response?.data?.detail || "玩家标签导出失败");
    } finally {
      setBusy(false);
    }
  };

  const importFile = async (event) => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    try {
      if (file.size > MAX_IMPORT_BYTES) throw new Error("标签文件超过 1 MB，已拒绝导入");
      const imported = parseLeaguePlayerTagsImport(await file.text());
      if (!imported.length) throw new Error("标签文件中没有可导入的记录");
      if (!window.confirm(`导入并合并 ${imported.length} 条本地玩家标签？同账号、同 PUUID 的标签会被覆盖。`)) return;
      setBusy(true);
      await importLeaguePlayerTags(imported);
      await load(1);
    } catch (error) {
      onError?.(error?.response?.data?.detail || String(error?.message || error || "玩家标签导入失败"));
    } finally {
      setBusy(false);
    }
  };

  if (streamerMode && !revealed) {
    return <section className="rounded-2xl border border-amber-400/25 bg-amber-400/[.06] p-5 text-center"><p className="text-sm font-semibold text-amber-200">直播隐私模式已遮挡玩家标签管理</p><button type="button" onClick={() => setRevealed(true)} className="mt-3 rounded-xl border border-amber-400/30 px-3 py-2 text-xs text-amber-100">仅本次查看</button></section>;
  }

  const pages = Math.max(1, Math.ceil(total / pageSize));
  return (
    <section className="rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-4">
      <div className="flex flex-wrap items-center gap-2"><h3 className="mr-auto text-sm font-bold">本地玩家标签管理</h3><button type="button" disabled={busy} onClick={exportAll} className="inline-flex items-center gap-1 rounded-lg border border-cyan-400/25 px-2.5 py-1.5 text-xs text-cyan-200 disabled:opacity-40"><Download className="h-3.5 w-3.5"/>导出</button><button type="button" disabled={busy} onClick={() => inputRef.current?.click()} className="inline-flex items-center gap-1 rounded-lg border border-emerald-400/25 px-2.5 py-1.5 text-xs text-emerald-200 disabled:opacity-40"><Upload className="h-3.5 w-3.5"/>导入</button><input ref={inputRef} aria-label="选择 League 玩家标签文件" type="file" accept="application/json,.json" className="hidden" onChange={importFile}/></div>
      <div className="mt-3 flex flex-wrap gap-2"><label className="flex min-w-56 flex-1 items-center gap-2 rounded-xl border border-cs2-border bg-cs2-bg-input px-3"><Search className="h-4 w-4 text-cs2-text-muted"/><input value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => event.key === "Enter" && load(1)} placeholder="搜索 Riot ID、PUUID、标签或备注" className="w-full bg-transparent py-2 text-xs outline-none"/></label><label className="flex items-center gap-2 rounded-xl border border-cs2-border px-3 text-xs"><input type="checkbox" checked={currentOnly} onChange={(event) => { setCurrentOnly(event.target.checked); setPage(1); }}/>仅当前账号</label><button type="button" disabled={busy} onClick={() => load(1)} className="rounded-xl border border-cs2-border p-2 disabled:opacity-40"><RefreshCw className={`h-4 w-4 ${busy ? "animate-spin" : ""}`}/></button></div>
      <div className="mt-3 divide-y divide-cs2-border-subtle overflow-hidden rounded-xl border border-cs2-border-subtle">{rows.map((row) => {
        const playerName = row.player?.game_name ? `${row.player.game_name}${row.player.tag_line ? `#${row.player.tag_line}` : ""}` : `${row.puuid.slice(0, 12)}…`;
        const isEditing = editing?.key === row.key;
        return <article key={row.key} className="p-3">{isEditing ? <div className="grid gap-2 md:grid-cols-[180px_1fr_auto]"><input aria-label="标签名称" value={editing.tag.label} maxLength={40} onChange={(event) => setEditing({ ...editing, tag: { ...editing.tag, label: event.target.value } })} className="rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><textarea aria-label="标签备注" value={editing.tag.note} maxLength={500} onChange={(event) => setEditing({ ...editing, tag: { ...editing.tag, note: event.target.value } })} className="min-h-10 resize-y rounded-lg border border-cs2-border bg-cs2-bg-input px-3 py-2 text-xs"/><div className="flex gap-1"><button type="button" disabled={busy} onClick={saveEdit} className="rounded-lg bg-emerald-500 px-3 py-2 text-xs font-bold text-black">保存</button><button type="button" onClick={() => setEditing(null)} className="rounded-lg border border-cs2-border px-3 py-2 text-xs">取消</button></div></div> : <div className="flex items-center gap-3">{row.player?.profile_icon_id != null ? <img src={getLeagueProfileIconUrl(row.player.profile_icon_id)} alt="" className="h-9 w-9 rounded-lg object-cover"/> : <span className="grid h-9 w-9 place-items-center rounded-lg bg-white/5"><UserRoundSearch className="h-4 w-4"/></span>}<button type="button" onClick={() => onOpenPlayer?.(row.puuid)} className="min-w-0 flex-1 text-left"><b className="block truncate text-xs text-cyan-200 hover:underline">{playerName}</b><span className="mt-1 block truncate text-[11px] text-cs2-text-muted">{row.tag?.label || "未命名"}{row.tag?.note ? ` · ${row.tag.note}` : ""}</span></button><span className="hidden max-w-28 truncate text-[10px] text-cs2-text-muted lg:block">账号 {row.owner_puuid ? `${row.owner_puuid.slice(0, 8)}…` : "旧版全局"}</span><button type="button" aria-label={`编辑 ${playerName}`} onClick={() => setEditing({ key: row.key, tag: { ...row.tag } })} className="rounded-lg border border-cs2-border p-2"><Edit3 className="h-3.5 w-3.5"/></button><button type="button" aria-label={`删除 ${playerName}`} onClick={() => remove(row)} className="rounded-lg border border-rose-400/25 p-2 text-rose-300"><Trash2 className="h-3.5 w-3.5"/></button></div>}</article>;
      })}{!rows.length && <p className="p-8 text-center text-xs text-cs2-text-muted">当前条件下没有本地玩家标签</p>}</div>
      <div className="mt-3 flex items-center justify-end gap-2 text-xs text-cs2-text-muted"><span>共 {total} 条 · 第 {page}/{pages} 页</span><button type="button" disabled={busy || page <= 1} onClick={() => load(page - 1)} className="rounded-lg border border-cs2-border p-1.5 disabled:opacity-30"><ChevronLeft className="h-4 w-4"/></button><button type="button" disabled={busy || page >= pages} onClick={() => load(page + 1)} className="rounded-lg border border-cs2-border p-1.5 disabled:opacity-30"><ChevronRight className="h-4 w-4"/></button></div>
    </section>
  );
}
