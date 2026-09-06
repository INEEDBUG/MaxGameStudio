// Bundled offline notes; keep in sync with docs/update-notes/v3.1.3.json.
export const HOME_RELEASE_VERSION = "3.1.3";
export const SUPERSEDED_LOCAL_CANDIDATE_VERSION = "3.0.6";
export const HOME_RELEASE_NOTES = {
  "zh": {
    "fixed": [
      "英雄联盟工作台关闭时可选择隐藏到系统托盘，不再缩到任务栏；点击托盘图标可恢复窗口。"
    ],
    "added": [
      "新增独立 RESG 数据窗口，仅在海克斯大乱斗中跟随当前选择或换选的英雄，开关与 OP.GG 相互独立。",
      "RESG 窗口可切换全部数据与常用精选，并自动记住选择；精选展示各品质热门海克斯和各出装阶段的前 3 项。"
    ],
    "optimized": [
      "海克斯按场次、装备组合按采用率排序，保留胜率和样本量，不把场次冒充选取率。",
      "保留完整数据入口，支持深浅色主题；网站数据不适配时回退到原页面。",
      "手动关闭 RESG 后不会因换英雄反复弹出；普通大乱斗及其他模式不会启用该窗口。"
    ]
  },
  "en": {
    "fixed": [
      "Closing the League workspace can hide it to the system tray instead of the taskbar. Click its tray icon to restore it."
    ],
    "added": [
      "An independent RESG window follows champion selections and swaps in Hextech ARAM only. OP.GG remains independently configurable.",
      "Switch between All data and remembered Popular picks, showing up to three augments per tier and three combinations per build stage."
    ],
    "optimized": [
      "Augments are ranked by sample count and equipment by adoption rate. Win rates and sample sizes remain visible; no pick percentages are invented.",
      "Supports light and dark compact views, with the original page available as a fallback.",
      "Manually closing RESG suppresses repeated swap popups. Ordinary ARAM and other game modes do not enable it."
    ]
  }
};
