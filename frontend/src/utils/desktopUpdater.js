import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { invoke } from "@tauri-apps/api/core";

/** Tauri 桌面壳注入 IPC 对象；浏览器 / Vite dev 页面无此对象。 */
export function isTauriDesktop() {
  return Boolean(window.__TAURI_INTERNALS__);
}

/** @param {unknown} value */
export function normalizeUpdateMode(value) {
  return String(value || "").trim().toLowerCase() === "force" ? "force" : "normal";
}

/** 将更新清单中的普通用户说明收敛为稳定的三分类结构。 */
export function normalizeUserReleaseNotes(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const normalized = {};
  for (const category of ["fixed", "added", "optimized"]) {
    normalized[category] = Array.isArray(value[category])
      ? value[category].map((item) => String(item || "").trim()).filter(Boolean)
      : [];
  }
  return Object.values(normalized).some((items) => items.length) ? normalized : null;
}

/**
 * Tauri updater 检查/下载控制器。
 * 状态：checking / available / downloading / installing / not-available / error / cancelled / skipped
 *
 * 发现更新后始终停在 available，必须由用户调用 confirm() 才会下载并安装。
 * autoInstallGraceMs 作为旧调用方的兼容参数保留，但不再启动自动下载倒计时。
 * defer() 表示跳过当前版本，cancel() 只取消本次检查。
 * force 模式同样必须由用户明确 confirm()，但不允许 defer()。
 *
 * 注意：Tauri updater 无法中断已经开始的下载；Windows 会在安装器成功启动后退出当前进程。
 */
export function createDesktopUpdateCheck(
  onStatus,
  {
    checkTimeoutMs = 8000,
    skipVersion = "",
  } = {},
) {
  let cancelled = false;
  let updateMode = "normal";
  let confirmWait = null;
  let startedDownload = false;

  const emit = (payload) => {
    try {
      onStatus?.(payload);
    } catch {
      // 状态回调异常不应中断更新流程
    }
  };

  const waitForUserChoice = () =>
    new Promise((resolve) => {
      confirmWait = resolve;
    });

  const resolveChoice = (choice, allowForceCancel = false) => {
    if (!confirmWait) return false;
    if (
      updateMode === "force" &&
      choice !== "install" &&
      !allowForceCancel &&
      !startedDownload
    ) {
      return false;
    }
    const wait = confirmWait;
    confirmWait = null;
    wait(choice);
    return true;
  };

  const run = async () => {
    emit({ status: "checking", update_mode: "normal" });

    let update = null;
    let checkTimer = null;
    try {
      const timeoutMs = Math.max(1000, Number(checkTimeoutMs) || 8000);
      update = await Promise.race([
        check(),
        new Promise((_, reject) => {
          checkTimer = window.setTimeout(
            () => reject(new Error(`检查更新超时（${Math.round(timeoutMs / 1000)} 秒）`)),
            timeoutMs,
          );
        }),
      ]);
    } catch (error) {
      emit({
        status: "error",
        error_stage: "check",
        error: String(error?.message || error),
        update_mode: "normal",
      });
      return;
    } finally {
      if (checkTimer !== null) window.clearTimeout(checkTimer);
    }
    if (cancelled) {
      emit({ status: "cancelled", update_mode: "normal" });
      return;
    }
    if (!update) {
      emit({ status: "not-available", update_mode: "normal" });
      return;
    }

    updateMode = normalizeUpdateMode(update.rawJson?.update_mode);
    const latest = update.version || null;
    const notes = typeof update.body === "string" ? update.body : "";
    const userNotes = normalizeUserReleaseNotes(update.rawJson?.user_release_notes);
    const base = {
      latest_version: latest,
      release_notes: notes,
      user_release_notes: userNotes,
      update_mode: updateMode,
      // Automatic installation is intentionally disabled. Keep the field in
      // the payload so older UI consumers cannot infer an auto-install path.
      auto_install: false,
    };
    if (
      updateMode !== "force" &&
      latest &&
      String(skipVersion || "").trim() === String(latest).trim()
    ) {
      try {
        await update.close();
      } catch {
        // ignore
      }
      emit({ status: "skipped", ...base, skipped_version: latest });
      return;
    }

    emit({ status: "available", ...base, awaiting_choice: true });

    const choice = await waitForUserChoice();
    if (cancelled || choice === "defer" || choice === "cancel") {
      try {
        await update.close();
      } catch {
        // ignore
      }
      emit({
        status: "cancelled",
        ...base,
        ...(choice === "defer" && latest ? { skipped_version: latest } : {}),
      });
      return;
    }

    startedDownload = true;
    let total = 0;
    let received = 0;
    try {
      await update.download((event) => {
        if (event.event === "Started") {
          total = Number(event.data?.contentLength) || 0;
          emit({ status: "downloading", ...base, progress: { percent: 0 } });
        } else if (event.event === "Progress") {
          received += Number(event.data?.chunkLength) || 0;
          emit({
            status: "downloading",
            ...base,
            progress: { percent: total > 0 ? (received / total) * 100 : NaN },
          });
        }
      });
    } catch (error) {
      emit({
        status: "error",
        ...base,
        error_stage: "download",
        error: String(error?.message || error),
      });
      return;
    }

    emit({ status: "installing", ...base });
    try {
      await invoke("persist_desktop_window_state");
      await update.install();
      // Windows 上 install() 成功启动 NSIS 后，Tauri updater 会直接退出当前进程。
      // 其他平台或测试环境若返回，则显式重启以载入新版本。
      await relaunch();
    } catch (error) {
      emit({
        status: "error",
        ...base,
        error_stage: "install",
        error: String(error?.message || error),
      });
    }
  };

  return {
    start: () => run(),
    /** 用户确认立即更新 */
    confirm: () => {
      resolveChoice("install");
    },
    /** 稍后再说（force 且尚未开始下载时无效） */
    defer: () => {
      if (startedDownload || updateMode === "force") return false;
      cancelled = true;
      resolveChoice("defer");
      return true;
    },
    cancel: () => {
      cancelled = true;
      // Internal replacement checks may cancel a force-update controller; this
      // does not expose a user-visible skip path for the force update itself.
      return resolveChoice("cancel", true);
    },
  };
}
