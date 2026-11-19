# Third-Party Licenses

MaxGameStudio 在编译与运行时依赖以下开源组件。常规编译依赖主要使用 MIT / BSD / Apache-2.0 / ISC 等宽松型许可证；另有下文单独列出的可选 GPL 工具。项目主体仍使用 **PolyForm Noncommercial 1.0.0**。

3.1.4 修复正式版内嵌英雄联盟工作台。测试版从同一仓库分别构建和分发 MaxGameStudio 与 MaxGameStudio League。独立工具箱基于 MIT 许可的 LeagueAkari 1.5.1，安装包保留其版权、许可证和所需第三方通知。

分发本项目编译产物（便携包 / 安装包 / Docker 镜像等）时，请保留各自的版权声明与许可证全文。具体许可证文本可在每个依赖包源码中的 `LICENSE` 文件查阅。

## Backend (Python)

| Package           | License        | Source                                                              |
| ----------------- | -------------- | ------------------------------------------------------------------- |
| fastapi           | MIT            | https://github.com/fastapi/fastapi                                  |
| uvicorn           | BSD-3-Clause   | https://github.com/encode/uvicorn                                   |
| python-multipart  | Apache-2.0     | https://github.com/Kludex/python-multipart                          |
| demoparser2       | MIT            | https://github.com/LaihoE/demoparser                                |
| pandas            | BSD-3-Clause   | https://github.com/pandas-dev/pandas                                |
| obs-websocket-py  | MIT            | https://github.com/Elektordi/obs-websocket-py                       |
| openai            | Apache-2.0     | https://github.com/openai/openai-python                             |
| pydantic          | MIT            | https://github.com/pydantic/pydantic                                |
| aiosqlite         | MIT            | https://github.com/omnilib/aiosqlite                                |
| watchdog          | Apache-2.0     | https://github.com/gorakhargosh/watchdog                            |

## Frontend (Node)

| Package                 | License | Source                                                 |
| ----------------------- | ------- | ------------------------------------------------------ |
| react                   | MIT     | https://github.com/facebook/react                      |
| react-dom               | MIT     | https://github.com/facebook/react-dom                  |
| axios                   | MIT     | https://github.com/axios/axios                         |
| lucide-react            | ISC     | https://github.com/lucide-icons/lucide                 |
| zustand                 | MIT     | https://github.com/pmndrs/zustand                      |
| tailwindcss             | MIT     | https://github.com/tailwindlabs/tailwindcss            |
| @tailwindcss/vite       | MIT     | https://github.com/tailwindlabs/tailwindcss            |
| vite                    | MIT     | https://github.com/vitejs/vite                         |
| @vitejs/plugin-react    | MIT     | https://github.com/vitejs/vite-plugin-react            |
| Tauri                   | MIT / Apache-2.0 | https://github.com/tauri-apps/tauri            |
| Tauri Plugins           | MIT / Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |


## Trademark Notices

- *Counter-Strike 2*, *CS2*, *Steam*, and *Valve* are trademarks of Valve Corporation. This project is **not affiliated with, endorsed by, or sponsored by Valve Corporation**.
- *5E* (5E对战平台) and *完美世界竞技平台 (Perfect World Arena)* are trademarks of their respective owners. This project is not affiliated with these platforms; it only consumes the standard `.dem` files they export.
- *OBS Studio* is a trademark of the OBS Project. This project communicates with OBS over the public WebSocket protocol and does not redistribute any OBS code or assets.

## Bundled / optional Windows runtime pieces

- **CPython Windows runtime** (installer and portable zip): sourced from the Astral `python-build-standalone` project (`install_only` tarball), Python Software Foundation license. See `packaging/windows/python-runtime.json` for the pinned URL.
- **League workspace runtime**: built from pinned [LeagueAkari](https://github.com/LeagueAkari/LeagueAkari) v1.5.1 source at commit `14557723706ccc0e0a9d62c470141d4cb7190fcd` (MIT). In stable 3.1.4 the Tauri host bundles and manages that workspace. In the beta, MaxGameStudio League packages the workspace as a standalone app. The original copyright and MIT license are retained in [LeagueAkari-LICENSE.txt](third_party/licenses/LeagueAkari-LICENSE.txt).
- **Optional FFmpeg** (GPL): when the user selects the matching Inno task, the installer downloads `ffmpeg-8.1.1-essentials_build.zip` from the GyanD `codexffmpeg` GitHub release (see `packaging/windows/ffmpeg-redist.json`). FFmpeg is a trademark of the FFmpeg project.
- **Optional @akiver/boiler-writter 1.7.0** (GPL-3.0): downloaded from the npm registry only after the user explicitly accepts the first-use notice. It remains an unmodified, separately executed Windows helper used to ask the locally signed-in Steam Game Coordinator for the latest eight match summaries or a real demo URL. Its package integrity is pinned by SHA-512, and its complete license is installed next to the executable. Source: https://github.com/akiver/boiler-writter

## Adapted source

- The match share-code decoder in `backend/app/valve_demo_resolver.py` is a Python adaptation of `akiver/csgo-sharecode` (MIT). Source: https://github.com/akiver/csgo-sharecode. The required notice is retained in `third_party/licenses/csgo-sharecode-LICENSE.txt`.
- The League automation lab in earlier MaxGameStudio releases and the League workspace both use [LeagueAkari](https://github.com/LeagueAkari/LeagueAkari) (MIT) for local LCU discovery and game-flow features. MaxGameStudio and MaxGameStudio League retain the applicable [license notice](third_party/licenses/LeagueAkari-LICENSE.txt).

## Game identity icons

Steam, Counter-Strike, VALORANT and League of Legends SVGs are from [Simple Icons 16.0.0](https://github.com/simple-icons/simple-icons/tree/16.0.0), whose icon collection is CC0-1.0. Game names and marks belong to their respective owners; this software is not affiliated with those owners. The beta bundles all four SVGs, and the README uses copies from the same source in [docs/brand-icons](docs/brand-icons).
