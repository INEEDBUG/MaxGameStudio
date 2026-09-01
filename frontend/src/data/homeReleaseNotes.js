// User-facing notes are intentionally bundled so the home page is available offline.
// Keep these readable for non-technical users; engineering details belong in GitHub.
export const HOME_RELEASE_VERSION = "3.0.5";

export const HOME_RELEASE_NOTES = {
  zh: {
    fixed: [
      "更新提示现在会先征求你的确认，不会在倒计时结束后自动安装。",
      "修复首次打开软件时首页仍然只显示 CS2 上手指南的问题。",
    ],
    added: [
      "新增 MaxGameStudio 首页，集中展示版本公告和反馈入口。",
      "新增独立首页侧栏入口，并保留 CS2、英雄联盟与无畏契约原有分区。",
      "无畏契约真拉伸新增 CFG 分辨率同步、完整备份、只读锁定、解锁和恢复。",
    ],
    optimized: [
      "Bug 与需求反馈可以直接打开对应的 GitHub 表单。",
      "新增 Pull Request 与项目仓库入口，方便查看开发进展和参与贡献。",
      "CFG 只修改已有分辨率字段，游戏运行时不会强制写入。",
    ],
  },
  en: {
    fixed: [
      "Updates now wait for your confirmation instead of installing after a countdown.",
      "The first screen no longer opens the CS2-only getting-started guide.",
    ],
    added: [
      "A new MaxGameStudio home page brings release news and feedback links together.",
      "Home now has its own sidebar entry while the existing CS2, League, and VALORANT sections remain available.",
      "VALORANT true stretch can now sync, back up, lock, unlock, and restore the active CFG resolution.",
    ],
    optimized: [
      "Bug reports and feature requests open the matching GitHub forms directly.",
      "Pull Request and repository links make development progress and contribution paths easier to find.",
      "CFG sync only patches existing resolution fields and refuses writes while the game is running.",
    ],
  },
};
