// User-facing notes are intentionally bundled so the home page is available offline.
// Keep these readable for non-technical users; engineering details belong in GitHub.
export const HOME_RELEASE_VERSION = "3.1.2";

export const SUPERSEDED_LOCAL_CANDIDATE_VERSION = "3.0.6";

export const HOME_RELEASE_NOTES = {
  zh: {
    fixed: [
      "修复部分电脑以管理员权限启动英雄联盟工作台时提示“拒绝访问”的问题。",
    ],
    added: [
      "启动时优先复用已有数据位置，不会自动迁移、复制或删除旧数据；新安装可使用推荐的非系统盘位置。",
      "设置中显示数据、日志、缓存、WebView、英雄联盟运行时和临时文件的实际位置；可手动切换目录，空目录从新设置开始。",
      "普通模式会预热英雄联盟工作台（不连接客户端、不运行自动化）；管理员模式需要点击 UAC，不进行普通预热。节省内存模式退出后恢复主程序，后台并行模式保留宿主。",
    ],
    optimized: [
      "设置会如实显示已有数据的位置；磁盘不可用时会提示，不会偷偷改存 C 盘。",
      "切换失败会取消本次切换并继续使用原目录；原目录也不可用时会停止启动并明确提示。",
      "Windows 系统记录和游戏自身的配置文件不在迁移范围内。",
      "首次或未预热启动英雄联盟工作台仍可能需要数秒，不保证 1 秒内打开。",
    ],
  },
  en: {
    fixed: [
      "Fixes Access denied errors when launching the League workspace with administrator privileges on some PCs.",
    ],
    added: [
      "Reuse an existing data location at startup; the app does not automatically migrate, copy, or delete old data. New installs can use the recommended non-system drive.",
      "Settings shows the actual paths for data, logs, cache, WebView, League runtime, and temporary files. You can switch manually; an empty target starts with fresh settings.",
      "Standard mode prewarms the League workspace without connecting to the client or running automation. Administrator mode requires UAC and skips prewarming; memory-saving mode restores the host, while parallel mode keeps it running.",
    ],
    optimized: [
      "Settings reports where existing data is actually stored. An unavailable drive causes a visible error, not a silent fallback to C:.",
      "A failed switch is cancelled and the usable original location remains active; if the original location is also unavailable, startup stops with an explicit error.",
      "Windows system records and game-owned configuration files are not relocated.",
      "Starting the League workspace for the first time or before prewarming finishes can still take several seconds; opening within one second is not guaranteed.",
    ],
  },
};
