import { useEffect } from "react";
import LeagueDetailedMatchCard from "./LeagueDetailedMatchCard";

/**
 * A small, independent equivalent of LeagueAkari's MatchPreviewer.
 *
 * Keeping the preview shell separate means history lists, player cards and
 * toolkit results can all open the same full match card without duplicating
 * summary/details loading or dry-run wiring.
 */
export default function MatchPreviewer({
  show = false,
  isOpen,
  match = null,
  details = null,
  loadingSummary = false,
  loadingDetails = false,
  streamerMode = false,
  useAliases = false,
  onClose = () => {},
  onShowChange,
  onOpenPlayer,
  onError,
  onDryRunGame,
}) {
  const visible = Boolean(isOpen ?? show);

  useEffect(() => {
    if (!visible) return undefined;
    const handleKeyDown = (event) => {
      if (event.key === "Escape") {
        onClose();
        onShowChange?.(false);
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [visible, onClose, onShowChange]);

  if (!visible) return null;

  const close = () => {
    onClose();
    onShowChange?.(false);
  };

  return <div data-testid="league-match-previewer" role="dialog" aria-modal="true" aria-label="对局预览" className="fixed inset-0 z-[80] flex items-start justify-center overflow-y-auto bg-black/65 p-4 backdrop-blur-sm" onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}>
    <div className="my-4 w-full max-w-[1120px] rounded-2xl border border-cs2-border bg-cs2-bg-elevated p-3 shadow-2xl" onMouseDown={(event) => event.stopPropagation()}>
      <header className="mb-2 flex flex-wrap items-center justify-between gap-2 px-1">
        <div className="min-w-0 text-xs"><b className="text-cs2-text-primary">对局预览</b>{match ? <span className="ml-2 text-cs2-text-muted">Game {match.game_id ?? match.gameId ?? "未知"} · {String(match.source || "lcu").toUpperCase()}</span> : null}</div>
        <button type="button" aria-label="关闭对局预览" onClick={close} className="rounded-lg border border-cs2-border px-3 py-1.5 text-xs text-cs2-text-secondary hover:text-cs2-text-primary">关闭</button>
      </header>
      {loadingSummary ? <div data-testid="league-match-preview-loading" className="flex min-h-52 items-center justify-center rounded-xl border border-cs2-border-subtle text-xs text-cs2-text-muted">正在读取对局摘要…</div> : match ? <LeagueDetailedMatchCard match={match} initialDetails={details} streamerMode={streamerMode} useAliases={useAliases} onOpenPlayer={onOpenPlayer} onError={onError} onDryRunGame={onDryRunGame} /> : <div data-testid="league-match-preview-empty" className="flex min-h-52 items-center justify-center rounded-xl border border-dashed border-cs2-border-subtle text-xs text-cs2-text-muted">暂无可用对局</div>}
      {loadingDetails ? <div className="mt-2 text-center text-[10px] text-cs2-text-muted">正在读取详细数据…</div> : null}
    </div>
  </div>;
}

export { MatchPreviewer };
