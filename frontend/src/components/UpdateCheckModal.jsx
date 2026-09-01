import { useT } from "../i18n/useT.js";
import { normalizeUpdateMode } from "../utils/desktopUpdater";

/** GitHub Release / Tauri updater 检查更新弹窗 */
export default function UpdateCheckModal({ open, info, onClose, onCancel, onConfirm, title }) {
  const t = useT();
  if (!open || !info) return null;

  const status = String(info.status || "");
  const err = info.error ? String(info.error) : "";
  const latest = info.latest_version ? String(info.latest_version) : "";
  const current = info.current_version ? String(info.current_version) : "";
  const notes = String(info.release_notes || "").trim();
  const userNotes = info.user_release_notes;
  const noteSections = [
    ["fixed", t("dialog.updateFixed")],
    ["added", t("dialog.updateAdded")],
    ["optimized", t("dialog.updateOptimized")],
  ].filter(([key]) => Array.isArray(userNotes?.[key]) && userNotes[key].length > 0);
  const hasUserNotes = noteSections.length > 0;
  const updateMode = normalizeUpdateMode(info.update_mode);
  const isForce = updateMode === "force";
  const percent = Number(info.progress?.percent);
  const hasPercent = Number.isFinite(percent);
  const upToDate = status === "not-available";
  const isAvailable = status === "available";
  const isUpdating = status === "downloading" || status === "installing";
  const forceLocked =
    isUpdating ||
    (isForce && isAvailable);

  let body = null;
  if (err || status === "error") {
    body = (
      <div className="space-y-2">
        <p className="text-[12px] font-semibold text-red-400">
          {info.error_stage === "install"
            ? t("dialog.updateInstallFailed")
            : info.error_stage === "download"
              ? t("dialog.updateDownloadFailed")
              : t("app.updateConnectFail")}
        </p>
        {err ? <p className="break-words text-[11px] text-zinc-400">{err}</p> : null}
      </div>
    );
  } else if (status === "checking") {
    body = <p className="text-sm text-zinc-300">{t("settings.updateChecking")}</p>;
  } else if (upToDate) {
    body = <p className="text-sm text-zinc-300">{t("dialog.updateUpToDate")}</p>;
  } else if (isAvailable) {
    body = (
      <div className="space-y-2">
        {isForce ? (
          <p className="text-sm font-semibold text-cs2-orange">{t("dialog.updateForceRequired")}</p>
        ) : (
          <p className="text-sm text-zinc-300">{t("dialog.updateAvailablePrompt")}</p>
        )}
      </div>
    );
  } else if (status === "downloading") {
    body = (
      <div className="space-y-2">
        <p className="text-sm text-zinc-300">
          {t("dialog.updateDownloading")}
          {hasPercent ? ` ${Math.round(percent)}%` : ""}
        </p>
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
          <div
            className="h-full bg-cs2-orange transition-all duration-300"
            style={{ width: `${Math.max(0, Math.min(100, hasPercent ? percent : 0))}%` }}
          />
        </div>
      </div>
    );
  } else if (status === "installing") {
    body = <p className="text-sm text-zinc-300">{t("dialog.updateInstalling")}</p>;
  } else if (status === "cancelled") {
    body = <p className="text-sm text-zinc-300">{t("dialog.updateCancelled")}</p>;
  }

  const showNotes =
    !err &&
    status !== "error" &&
    status !== "cancelled" &&
    status !== "checking" &&
    status !== "not-available" &&
    (hasUserNotes || notes);

  return (
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-black/60 p-4"
      role="dialog"
      aria-modal="true"
      onMouseDown={(e) => {
        if (forceLocked) e.stopPropagation();
      }}
    >
      <div className="max-h-[85vh] w-full max-w-lg overflow-hidden rounded-xl border border-white/10 bg-cs2-bg-card shadow-2xl">
        <div className="border-b border-white/10 px-4 py-3">
          <h2 className="text-sm font-bold text-white">{title || t("dialog.updateTitle")}</h2>
          {latest || current ? (
            <p className="mt-1 font-mono text-[11px] text-zinc-400">
              {latest ? (
                <>
                  {t("dialog.updateLatestVersion")} <span className="text-cs2-orange">{latest}</span>
                </>
              ) : null}
              {latest && current ? " · " : null}
              {current ? (
                <>
                  {t("dialog.updateCurrentVersion")} <span className="text-zinc-300">{current}</span>
                </>
              ) : null}
            </p>
          ) : null}
          <p className="mt-1 text-[10px] text-zinc-500">{t("dialog.updateViaCloudflare")}</p>
        </div>
        <div className="max-h-[45vh] overflow-y-auto px-4 py-3">
          {body}
          {showNotes && hasUserNotes ? (
            <div className="mt-3 space-y-2" aria-label={t("dialog.updateSummary")}>
              {noteSections.map(([key, label]) => (
                <section key={key} className="rounded-md border border-white/5 bg-black/20 p-3">
                  <h3 className="text-[12px] font-semibold text-white">{label}</h3>
                  <ul className="mt-1.5 space-y-1 text-[12px] leading-relaxed text-zinc-300">
                    {userNotes[key].map((item, index) => (
                      <li key={`${key}-${index}`} className="flex gap-2">
                        <span className="text-cs2-orange" aria-hidden="true">•</span>
                        <span>{item}</span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          ) : showNotes ? (
            <pre className="mt-3 whitespace-pre-wrap break-words rounded-md border border-white/5 bg-black/20 p-3 font-sans text-[12px] leading-relaxed text-zinc-300">
              {notes}
            </pre>
          ) : null}
          {isAvailable && !hasUserNotes && !notes ? (
            <p className="mt-2 text-[12px] text-zinc-500">{t("dialog.updateNoNotes")}</p>
          ) : null}
        </div>
        <div className="flex items-center justify-end gap-3 border-t border-white/10 px-4 py-2">
          {isAvailable ? (
            <>
              {!isForce ? (
                <button
                  type="button"
                  className="min-h-9 cursor-pointer rounded-md border border-white/15 px-3 py-1.5 text-[11px] font-semibold text-zinc-300 transition-colors hover:border-white/30 hover:bg-white/5 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cs2-orange"
                  onClick={() => onClose?.()}
                >
                  {t("dialog.updateSkip")}
                </button>
              ) : null}
              <button
                type="button"
                className="min-h-9 cursor-pointer rounded-md bg-cs2-orange px-4 py-1.5 text-[11px] font-semibold text-black transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/80"
                onClick={() => onConfirm?.()}
              >
                {t("dialog.updateNow")}
              </button>
            </>
          ) : null}
          {!forceLocked && !isAvailable ? (
            <button
              type="button"
              className="text-[11px] font-semibold text-zinc-500 hover:text-white"
              onClick={() => onClose?.()}
            >
              {t("dialog.updateClose")}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
