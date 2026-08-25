import { useEffect, useState } from "react";
import { Copy, FileText, Minus, Square, X } from "lucide-react";
import API from "../api/api";
import { desktopBridge, isDesktopApp } from "../desktop/desktopBridge";

export default function CustomTitleBar() {
  const [isMaximized, setIsMaximized] = useState(false);

  const runWindowAction = (action) => {
    void action().catch((error) => {
      console.error("Desktop window action failed", error);
    });
  };

  useEffect(() => {
    if (!desktopBridge) return undefined;
    void desktopBridge.isMaximized().then(setIsMaximized);
    return desktopBridge.onMaximizeChange(setIsMaximized);
  }, []);

  if (!isDesktopApp) return null;

  return (
    <div
      className="z-50 flex h-11 w-full shrink-0 items-center justify-between border-b border-cs2-border-subtle bg-cs2-bg-sidebar/88 text-cs2-text-primary backdrop-blur-2xl"
      data-tauri-drag-region
      data-testid="custom-titlebar"
    >
      <div className="flex items-center px-3.5" data-tauri-drag-region>
        <img
          src={`${import.meta.env.BASE_URL}cs2-ultimate-insight-logo.png`}
          alt="Logo"
          className="mr-2 h-5 w-5"
          data-tauri-drag-region
        />
        <span className="text-[12px] font-semibold tracking-[-0.01em]" data-tauri-drag-region>CS2 Ultimate Insight Studio</span>
      </div>

      <div className="flex h-full">
        <button
          type="button"
          aria-label="打开日志目录"
          title="打开 MaxGameStudio 日志目录"
          onClick={() => runWindowAction(() => API.post("config/open-logs"))}
          className="flex h-full w-11 items-center justify-center text-cs2-text-secondary transition-[background-color,color,transform] duration-150 hover:bg-cs2-bg-hover hover:text-cs2-text-primary active:scale-[0.94]"
        >
          <FileText size={15} />
        </button>
        <button
          type="button"
          aria-label="Minimize"
          onClick={() => runWindowAction(() => desktopBridge.minimize())}
          className="flex h-full w-11 items-center justify-center text-cs2-text-secondary transition-[background-color,color,transform] duration-150 hover:bg-cs2-bg-hover hover:text-cs2-text-primary active:scale-[0.94]"
        >
          <Minus size={16} />
        </button>
        <button
          type="button"
          aria-label="Toggle maximize"
          onClick={() => runWindowAction(() => desktopBridge.toggleMaximize())}
          className="flex h-full w-11 items-center justify-center text-cs2-text-secondary transition-[background-color,color,transform] duration-150 hover:bg-cs2-bg-hover hover:text-cs2-text-primary active:scale-[0.94]"
        >
          {isMaximized ? <Copy size={14} /> : <Square size={14} />}
        </button>
        <button
          type="button"
          aria-label="Close"
          onClick={() => runWindowAction(() => desktopBridge.close())}
          className="flex h-full w-11 items-center justify-center text-cs2-text-secondary transition-[background-color,color,transform] duration-150 hover:bg-red-500 hover:text-white active:scale-[0.94]"
        >
          <X size={16} />
        </button>
      </div>
    </div>
  );
}
