# 私人 Windows 候选包

## 0.1.0-rc.2（2026-08-02，当前候选）

- GitHub Actions：<https://github.com/INEEDBUG/MaxGameStudio/actions/runs/30728334503>
- Artifact：`MaxGameStudio-0.1.0-rc.2`（私人仓库，保留 14 天）
- 安装包：`MaxGameStudio_0.1.0-rc.2_x64-setup.exe`
- 安装包大小：`39,188,332` 字节（约 37.37 MiB）
- SHA-256：`990dd4cc590576cc2f6a01af9a802d85052e7ee3b0a84a436d3d1df34bde8dc2`
- 内嵌 resources：87.19 MiB（预算 100 MiB）
- 本机归档目录：`D:\CodexProject\Releases\MaxGameStudio-0.1.0-rc.2`
- 相比 rc.1，新增官匹 Demo 过期、Valve 回放服务器繁忙、Steam 未登录和 CS2 占用 Game Coordinator 的分层诊断。

## 已通过的构建门

- patched demoparser wheel 构建、导入和运行时能力校验。
- 后端 706 项测试。
- 前端 590 项测试与生产构建。
- Rust fmt、clippy（warnings as errors）和 Rust tests。
- Tauri release 编译、NSIS 生成、内嵌 Python 启动与依赖瘦身校验。
- 安装包体积、预计安装占用和本地 SHA-256 复核。

## 候选包限制

- 这是私人验收包，尚未配置 Authenticode 代码签名；Windows 可能显示“未知发布者”。
- 原项目 R2 自动更新通道已停用，避免定制版被上游安装包覆盖。
- 尚需在实际安装后完成首次启动、页面巡检和真实 Steam Share Code 下载验收。
- 推荐将应用安装到 D 盘，例如 `D:\MaxGameStudio`；升级程序会自动兼容并迁移旧版持久化配置与 SQLite 数据。

## 旧候选

- `0.1.0-rc.1`：GitHub Actions `30727120892`，SHA-256 `48fd15f0f227ba78c9d9dc82caf96f526b7863020dd094f19e48cf7dc85aac30`。仅保留作构建链基线，不再作为安装验收版本。
