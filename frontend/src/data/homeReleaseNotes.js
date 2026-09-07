// Bundled offline notes; keep in sync with docs/update-notes/v3.1.4.json.
export const HOME_RELEASE_VERSION = "3.1.4";
export const SUPERSEDED_LOCAL_CANDIDATE_VERSION = "3.0.6";
export const HOME_RELEASE_NOTES = {
  zh: {
    fixed: ["后端启动失败不再关闭整个软件，可返回首页或英雄联盟。"],
    added: ["RESG 数据窗口新增可记忆的置顶开关，窗口管理中的设置同步生效。", "首页可直接检查更新，无需启动 Python 后端。"],
    optimized: ["后端改为按需启动：首页与英雄联盟入口不再等待 Python，依赖后端的页面首次使用时才启动服务。", "从英雄联盟返回主程序不再等待后端启动，切换页面不会中断已有后台任务。", "保留 GitHub 更新与签名校验，改善超时和取消安装提示；不接入付费国内加速服务。"],
  },
  en: {
    fixed: ["Backend startup failures no longer close the application. Home and League remain available."],
    added: ["RESG has a remembered Keep on top toggle, synchronized with window settings.", "Check updates directly from Home without starting Python."],
    optimized: ["Python starts on demand for service-dependent pages, not on Home or League entry.", "Returning from League no longer waits for Python; navigation does not interrupt existing backend jobs.", "GitHub updates retain signature verification with improved timeout and cancellation handling. No paid domestic acceleration service is added."],
  },
};
