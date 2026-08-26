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

/**
 * Tauri updater 检查/下载控制器。
 * 状态：checking / available / downloading / installing / not-available / error / cancelled
 *
 * 默认在发现更新后自动下载并安装。传入 autoInstall: false 时会停在 available，
 * 等待 confirm() 再继续；defer()/cancel() 表示稍后再说。
 * force 模式下 defer/cancel 在开始下载前会被忽略。
 *
 * 注意：Tauri updater 无法中断已经开始的下载；Windows 会在安装器成功启动后退出当前进程。
 */
export function createDesktopUpdateCheck(
  onStatus,
  { autoInstall = true, checkTimeoutMs = 8000 } = {},
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

  const resolveChoice = (choice) => {
    if (!confirmWait) return false;
    if (updateMode === "force" && choice !== "install" && !startedDownload) {
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
    const base = {
      latest_version: latest,
      release_notes: notes,
      update_mode: updateMode,
      auto_install: autoInstall,
    };
    emit({ status: "available", ...base });

    if (!autoInstall) {
      const choice = await waitForUserChoice();
      if (cancelled || choice !== "install") {
        try {
          await update.close();
        } catch {
          // ignore
        }
        emit({ status: "cancelled", ...base });
        return;
      }
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
      cancelled = true;
      resolveChoice("defer");
    },
    cancel: () => {
      cancelled = true;
      resolveChoice("defer");
    },
  };
}
