import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";

export const isDesktopApp = Boolean(window.__TAURI_INTERNALS__);

const currentWindow = isDesktopApp ? getCurrentWindow() : null;

export const desktopBridge = isDesktopApp
  ? {
      minimize: () => currentWindow.minimize(),
      toggleMaximize: () => currentWindow.toggleMaximize(),
      close: () => currentWindow.close(),
      startDragging: () => currentWindow.startDragging(),
      isMaximized: () => currentWindow.isMaximized(),
      onMaximizeChange(callback) {
        let active = true;
        const unlistenPromise = currentWindow.onResized(async () => {
          if (active) callback(await currentWindow.isMaximized());
        });
        return () => {
          active = false;
          void unlistenPromise.then((unlisten) => unlisten());
        };
      },
      getVersion,
      setCloseToTray: (enabled) => invoke("set_close_to_tray", { enabled: Boolean(enabled) }),
      getCloseToTray: () => invoke("get_close_to_tray"),
      setCloseAction: (action) => invoke("set_close_action", { action }),
      getCloseAction: () => invoke("get_close_action"),
      hideToTray: () => invoke("hide_to_tray"),
      quitApp: () => invoke("quit_app"),
      onCloseChoiceRequested: (callback) => listen("desktop-close-choice-requested", callback),
      getLeagueRuntimeStatus: () => invoke("get_league_runtime_status"),
      prepareLeagueRuntime: () => invoke("prepare_league_runtime"),
      cancelLeaguePrewarm: () => invoke("cancel_league_prewarm"),
      // Keep the frontend call site optional while always satisfying the
      // native command's required boolean argument.
      launchLeagueRuntime: (mode, options = {}) => invoke("launch_league_runtime", {
        mode,
        administrator: Boolean(options?.administrator),
      }),
      stopLeagueRuntime: () => invoke("stop_league_runtime"),
      getDesktopStorage: () => invoke("get_desktop_storage"),
      chooseDesktopStorage: () => invoke("choose_desktop_storage"),
      cancelDesktopStorageChange: () => invoke("cancel_desktop_storage_change"),
      async showOpenDialog(options = {}) {
        const properties = Array.isArray(options.properties) ? options.properties : [];
        const selected = await open({
          title: options.title,
          defaultPath: options.defaultPath,
          filters: options.filters,
          directory: properties.includes("openDirectory"),
          multiple: properties.includes("multiSelections"),
        });
        const filePaths = selected == null ? [] : Array.isArray(selected) ? selected : [selected];
        return { canceled: filePaths.length === 0, filePaths };
      },
      async chooseDemoFiles() {
        try {
          const selected = await open({
            title: "选择 CS2 Demo",
            filters: [{ name: "CS2 Demo", extensions: ["dem"] }],
            multiple: true,
          });
          return selected == null ? [] : Array.isArray(selected) ? selected : [selected];
        } catch {
          return [];
        }
      },
      async chooseDirectory(defaultPath = "", title = "选择文件夹") {
        try {
          const selected = await open({
            title,
            defaultPath: typeof defaultPath === "string" && defaultPath.trim() ? defaultPath.trim() : undefined,
            directory: true,
            multiple: false,
          });
          return typeof selected === "string" ? selected : "";
        } catch {
          return "";
        }
      },
      async showItemInFolder(itemPath) {
        if (typeof itemPath !== "string" || !itemPath.trim()) return false;
        try {
          await revealItemInDir(itemPath.trim());
          return true;
        } catch {
          return false;
        }
      },
      openExternal: (url) => openUrl(url),
    }
  : null;
