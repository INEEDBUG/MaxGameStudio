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
      "首次启动可选择数据存放位置，优先推荐非系统盘；以后可在设置的路径页面调整。",
      "支持查看当前和待切换目录，切换前可以取消；下次启动时会复制并校验数据。",
    ],
    optimized: [
      "应用数据、缓存、日志和临时文件统一使用所选位置；磁盘不可用时会提示，不会偷偷改存 C 盘。",
      "升级保留原安装位置。迁移后旧文件暂时保留用于恢复，因此不会立即释放原目录占用的空间。",
      "Windows 系统记录和游戏自身的配置文件不在迁移范围内。",
    ],
  },
  en: {
    fixed: [
      "Fixes Access denied errors when launching the League workspace with administrator privileges on some PCs.",
    ],
    added: [
      "Choose where app data is stored on first launch; a non-system drive is recommended. Change it later in Settings > Paths.",
      "View or cancel a pending storage change before data is copied and verified on the next startup.",
    ],
    optimized: [
      "App data, caches, logs and temporary files follow the selected location. An unavailable drive causes a visible error, not a silent fallback to C:.",
      "Upgrades preserve the installation location. Migration retains old files for recovery, so their disk space is not reclaimed immediately.",
      "Windows system records and game-owned configuration files are not relocated.",
    ],
  },
};
