// Bundled notes for the installed product, available offline.
export const HOME_RELEASE_VERSION = "3.2.0-beta.1";
export const HOME_RELEASE_NOTES = {
  zh: {
    fixed: ["主程序更新只识别 MaxGameStudio 安装包；英雄联盟工具箱使用自己的更新通道。"],
    added: ["Steam 已保存账户一键切换，保留本机登录设置。", "中文安装向导可选择程序和数据目录，默认沿用正式版数据。"],
    optimized: ["首页直接显示 Steam 账户和常用工具。", "英雄联盟工具箱改为独立安装，主程序不再携带 Electron 运行环境。", "没有下载任务时停止快速轮询，首页保持按需启动后端。"],
  },
  en: {
    fixed: ["Main updates only accept MaxGameStudio installers. League Toolbox uses its own update channel."],
    added: ["Switch between saved Steam accounts while preserving local sign-in settings.", "Chinese setup offers program and data locations and keeps stable-version data by default."],
    optimized: ["Home opens directly to Steam accounts and common tools.", "League Toolbox installs separately; main no longer bundles its Electron runtime.", "Fast download polling stops when idle. Home keeps the backend on demand."],
  },
};
