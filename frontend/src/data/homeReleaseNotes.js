// User-facing notes are intentionally bundled so the home page is available offline.
// Keep these readable for non-technical users; engineering details belong in GitHub.
export const HOME_RELEASE_VERSION = "3.1.1";

export const SUPERSEDED_LOCAL_CANDIDATE_VERSION = "3.0.6";

export const HOME_RELEASE_NOTES = {
  zh: {
    fixed: [
      "修复点击内嵌英雄联盟工作台右上角 X 后直接返回 MaxGameStudio 的问题。",
      "修复关闭工作台时未明确选择就离开当前会话的问题。",
      "修复节省内存与后台并行模式的关闭行为不一致问题。",
    ],
    added: [
      "新增关闭确认选项：返回 MaxGameStudio，或最小化英雄联盟工作台并保留运行状态。",
    ],
    optimized: [
      "点击取消不会退出工作台，也不会终止 MaxGameStudio 或游戏后台任务。",
      "关闭确认与节省内存、后台并行两种启动模式保持一致，减少误操作。",
    ],
  },
  en: {
    fixed: [
      "Fixes the embedded League workspace returning to MaxGameStudio immediately when its X button is clicked.",
      "Fixes the close flow leaving the current session without an explicit choice.",
      "Keeps close behavior consistent between Save memory and Run in parallel modes.",
    ],
    added: [
      "Adds a close-choice prompt: return to MaxGameStudio, or minimize the League workspace while keeping it running.",
    ],
    optimized: [
      "Canceling the prompt no longer closes the workspace or stops MaxGameStudio and game background tasks.",
      "The close confirmation now follows the same rules in both startup modes to prevent accidental exits.",
    ],
  },
};
