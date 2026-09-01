// User-facing notes are intentionally bundled so the home page is available offline.
// Keep these readable for non-technical users; engineering details belong in GitHub.
export const HOME_RELEASE_VERSION = "3.0.6";

export const HOME_RELEASE_NOTES = {
  zh: {
    fixed: [
      "Mini 的置顶按钮现在可以正常取消置顶，并会记住你的选择。",
      "秒退不再把普通请求返回误报为成功；只有客户端确实离开选人阶段才显示完成。",
      "VALORANT 配置自动发现失败时会说明原因，不再只显示“未找到”。",
    ],
    added: [
      "VALORANT 真拉伸可以手动选择当前账号的 GameUserSettings.ini，并继续使用同一文件完成同步、锁定和恢复。",
      "Mini 新增透明度调节，最低保留 40% 可见度，避免窗口完全不可见。",
      "进入 League 对局阶段后会自动显示独立实时对局窗口，离开对局或断线后自动隐藏。",
    ],
    optimized: [
      "实时对局先显示阵容，再逐名补齐战绩；较慢的玩家不会挡住其他卡片。",
      "移除游戏内发送、房间/无尽狂潮、本地标签管理和设置迁移等不再保留的工具入口。",
      "独立实时对局窗口在进入对局时直接显示深色启动页，避免隐藏 WebView 在 Windows 上恢复为白窗。",
    ],
  },
  en: {
    fixed: [
      "Mini can now be unpinned correctly and remembers the choice.",
      "Dodge no longer reports success until the client actually leaves champion select.",
      "VALORANT CFG discovery now explains why no file was found.",
    ],
    added: [
      "VALORANT true stretch can manually select the active GameUserSettings.ini and reuse it for sync, lock, and restore.",
      "Mini now includes an opacity control with a safe 40% minimum.",
      "The standalone League ongoing-game window automatically follows live game phases.",
    ],
    optimized: [
      "Ongoing-game cards show the roster first and enrich players independently.",
      "Obsolete in-game-send, room/Swarm, tag-manager, and settings-transfer surfaces were removed.",
      "The ongoing-game window now opens on a dark boot surface instead of restoring a hidden WebView as a white window.",
    ],
  },
};
