// User-facing notes are intentionally bundled so the home page is available offline.
// Keep these readable for non-technical users; engineering details belong in GitHub.
export const HOME_RELEASE_VERSION = "3.1.0";

export const SUPERSEDED_LOCAL_CANDIDATE_VERSION = "3.0.6";

export const HOME_RELEASE_NOTES = {
  zh: {
    fixed: [
      "修复后台并行模式错误关闭 MaxGameStudio 主窗口的问题；现在只有节省内存模式会收起主程序。",
      "修复记住英雄联盟启动方式后无法再次选择的问题，手动进入页面始终可以修改或清除选择。",
      "修复英雄联盟工作台浅色/深色主题下按钮不可见与侧栏异常红线，并移除不再保留的游戏内发送入口。",
      "修复内嵌工作台运行时升级覆盖风险；工作台未退出时安装程序会安全中止。",
    ],
    added: [
      "英雄联盟入口升级为由 MaxGameStudio 管理的单一内嵌工作台，无需同时维护两套重复界面。",
      "新增每次询问、节省内存和后台并行三种启动方式，并支持可逆的记住与清除选择。",
      "新增仅提升英雄联盟工作台权限的管理员启动选项，MaxGameStudio 主程序仍保持普通权限。",
      "新增独立英雄联盟工作台图标，便于在任务栏和窗口中识别。",
    ],
    optimized: [
      "节省内存模式会暂停 MaxGameStudio 后端以及 CS2/无畏契约后台任务，退出工作台后自动恢复。",
      "后台并行模式会保留主窗口、WebView、后端及其他游戏任务，并在启动前提示额外内存开销。",
      "管理员工作台从经过清单和 SHA-256 双重验证的受保护临时会话启动，并使用独立受保护配置。",
    ],
  },
  en: {
    fixed: [
      "Fixes Run in parallel incorrectly closing the MaxGameStudio main window; only Save memory now suspends the host UI.",
      "Fixes remembered League launch choices hiding the chooser permanently; manual entry can always change or clear the choice.",
      "Fixes invisible controls in both themes and the stray sidebar line, and removes the retired in-game-send entry.",
      "Prevents an installer from overwriting the embedded workspace while it is still running.",
    ],
    added: [
      "Replaces duplicated League surfaces with one embedded workspace managed by MaxGameStudio.",
      "Adds Ask every time, Save memory, and Run in parallel startup modes with reversible remembered choices.",
      "Adds optional elevation for the League workspace only while keeping the MaxGameStudio host at standard privileges.",
      "Adds a dedicated League workspace icon for clearer taskbar and window identification.",
    ],
    optimized: [
      "Save memory pauses the MaxGameStudio backend and CS2/VALORANT background tasks, then restores them after the workspace exits.",
      "Run in parallel keeps the host window, WebView, backend, and other game tasks alive and shows the expected memory cost first.",
      "Administrator launches use a protected temporary session verified against the embedded manifest and SHA-256 hashes, plus a separate protected profile.",
    ],
  },
};
