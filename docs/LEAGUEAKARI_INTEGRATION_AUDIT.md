# LeagueAkari integration audit

Baseline reviewed: `LeagueAkari` dev branch at `cb236b6caf196e2505c7dfa6b34185020fd1e570`, plus the locally installed League Akari 1.5.1 shell. The only upstream change after the functional baseline `6e40999728f6408bddbb067fb89a81e086ae7d58` is documentation-only. The upstream remains MIT licensed and is credited in `THIRD_PARTY_LICENSES.md`.

This document prevents the League integration from becoming a collection of unrelated toggles. It maps the upstream product into this project's Python/FastAPI + React/Tauri architecture and records what is actually implemented. “Implemented” below means that the corresponding code path exists; it is not a claim that every account-affecting League state has passed live acceptance. The delivery target is functional and interaction parity with LeagueAkari while retaining this project's React/Tauri implementation; presentation differences are defects unless they are required by the host shell or explicitly approved by the user. MIT attribution and the upstream notice remain required.

## Architecture reviewed

- Native client discovery: process enumeration and `NtQueryInformationProcess(ProcessCommandLineInformation)`; WMI is only an optional elevated fallback.
- LCU transport and state: authenticated local HTTPS, event subscriptions, state initialization, reconnect handling and Riot/Tencent client variants.
- SGP transport: regional service discovery, summoner, ranked statistics, match-history query and game-summary sources.
- Automation: `auto-gameflow`, `auto-select`, `auto-champ-config` and `auto-misc`.
- Player product: multi-player tabs, summary, ranked data, mastery, challenges, match cards, pagination, recent encounters, saved tags and advanced composable filters.
- Live-game product: ongoing-game player cards, premade detection, champion usage, jungle-path analysis, queue filters and auxiliary windows.
- Toolkit: lobby controls, client controls, in-game messages, chat presence/status, rewards, loot and friend tools.
- Desktop shell: tray, window manager, main/mini/auxiliary windows, shortcuts, updater, storage migrations and streamer mode.

## rc.40 本轮验收证据（2026-08-17）

以下只记录本轮在本机候选版上的实测结果，不把尚未实际触发的游戏流程写成已验收：

| 检查项 | 实测结果 | 边界 |
| --- | --- | --- |
| 本地候选版 | `2.5.14-rc.40` 已安装到 `D:\MaxGameStudio` | 本轮未推送 GitHub，也未正式发布 |
| Tencent 客户端连接 | 真实 Tencent `GZ100` 客户端连接成功，LCU 事件流在线 | 仅证明连接与事件订阅，不等于所有流程写操作已通过 |
| 战绩读取 | 成功读取 20 场当前账号战绩 | 仅按本轮真实账号结果记录 |
| Tencent current-player-only 摘要 | 在缺少全队上下文时不再伪造参团率和伤害占比，界面显示 `—` | 不推断缺失的队伍统计 |
| 空闲进行中对局 | `available: false`，`query_stage: idle` | 只验证无对局时的只读状态 |
| 账号写入安全开关 | 手动账号写入共享默认关闭的 toolkit gate；自动写入共享 automation master + feature gate | 未开启任何自动接受、选人、配置、点赞或其他写操作 |
| 辅助窗口 | Mini、ongoing、cooldown 窗口的读取/显示路径已验证 | OP.GG 不属于上游 LeagueAkari 范围，已从候选版窗口生命周期移除 |
| Mini 窗口持久化 | Mini 关闭后立即落盘，强制重启后可恢复 | 仅记录已验证的 Mini 几何/可见性行为 |
| 更新器 | 更新安装前显式落盘已验证 | 不等于已完成签名发布或无需用户确认的正式升级策略 |
| 自动化测试 | 后端 `111`、League 前端 `10`、updater `7`、Rust `3` 通过 | 测试通过不替代真实 Tencent 状态机验收 |
| 安装包签名 | 安装包仍未签名 | Windows SmartScreen/发布信任链仍需单独处理 |

## Current integration status

### Implemented and locally verified

- Native non-elevated LeagueClientUx discovery using the same Windows API strategy as LeagueAkari, with a read-only CIM fallback for Tencent/WeGame or elevated clients that deny the native query.
- Riot and WeGame/Tencent command-line parsing, local-memory-only LCU credentials and authenticated HTTPS calls.
- Current summoner, region/platform and gameflow phase status.
- Automatic ready check, play again, reconnect and basic invitation accept/decline policy.
- Automatic pick/ban priority, availability checks, delay and optional lock-in.
- Per-champion rune page and summoner-spell application.
- Automatic honor submission with ballot completion.
- Current-account LCU match history with champion metadata and core performance fields.
- Independent always-on-top League Mini window with phase/team summary and quick automation controls.
- Queue-group and position-specific pick/ban profiles with searchable ordered champion lists.
- Pick intent conflict handling, all three show/lock strategies, ARAM bench selection delay and champion-trade acceptance.
- Automatic leader handoff plus per-invite-type accept/decline/ignore rules and away-state gating.
- League Mini phase lifecycle parity: auto-show in lobby/matchmaking/ready-check/non-spectating champ select, auto-hide elsewhere, and manual close suppression until the phase changes.
- LCU `OnJsonApiEvent` WebSocket subscription with authenticated local event wakeups; timed polling remains only as recovery/fallback.
- LeagueAkari-equivalent automatic matchmaking gates: leader check, minimum members, pending invitees, penalty wait, start delay and fixed/estimated rematch cancellation.
- All upstream honor strategies: prefer lobby members, lobby-only, allies, allies plus opponents, and automatic opt-out.
- Player center foundation: current/cross-player summoner profile, ranked queues, top mastery, recent matches and durable local player tags.
- Ongoing-game foundation: current Gameflow teams, champion assignments, ranked/profile enrichment, local tags and click-through to player details.
- Event-driven private-chat auto reply, away-only gating, offline-status lock and one-shot ARAM side announcements.
- Event-driven friend auto-invitation queue: waits for an opted-in friend to become online, checks lobby permissions/membership, invites once and removes the completed target.
- Riot ID (`game name#tag`) cross-player lookup through the local LCU alias endpoint, paginated match history and durable recently encountered player indexing.
- Live-game recent-form and current-champion usage summaries, plus LeagueAkari-style premade inference from repeated same-team match history.
- Client toolkit overview plus LeagueAkari-equivalent mission (`SELECT_REWARDS`), reward-grant (`PENDING_SELECTION`) and Event Hub claim flows, and selected-friend deletion. The implementation re-reads live LCU state immediately before every write, never preselects or randomly chooses a reward, requires the account-write master switch and exact confirmation phrase, and deletes only explicitly selected friend IDs.
- LeagueAkari client/lobby/profile toolkit parity: eligible/unavailable queue discovery, revalidated queue-lobby creation, explicit lobby leave, Strawberry champion slot/map/difficulty controls, profile background skin/augment selection, banner accent, prestige-crest removal, challenge-token clearing and account-scope emote clearing. Manual account writes share the toolkit gate (and the applicable exact confirmation/live revalidation); automatic writes use the automation master plus their individual feature gate.
- Friend-tool parity: grouped/searchable Riot IDs, selected-only deletion, background enrichment of last-match and friendship-start dates through LCU/SGP, and one-click routing into the integrated player center. Date enrichment is read-only and intentionally does not delay the toolkit overview.
- Multi-client/startup parity: every readable `LeagueClientUx` process is enumerated with account, region and phase metadata; the user can bind the lab to an exact PID without exposing credentials. Tencent TCLS, the per-installation WeGame launcher, standalone WeGame and Riot Client are detected using LeagueAkari-compatible registry/file rules and are launched only from an explicit user click.
- Match-replay parity: current and searched-player match cards read LCU replay availability, create the required version/queue/game-end metadata, show live `.rofl` download progress and hand a completed replay back to League for playback. Download and playback are always explicit button actions.
- Champion-mastery parity: the profile keeps a fast top-ten summary and exposes the complete LCU mastery catalog on demand with official icons, mastery level/points, point ordering and local search.
- Encounter-history parity: current-account match imports build a per-opponent shared-game index, isolated by the active local account. A searched player's profile exposes paginated head-to-head rows with both champions and K/D/A, and allows removing an individual locally persisted encounter without touching Riot data.
- Expanded match-card foundation: collapsed performance rows now open into both-team scoreboards, a searchable ten-player raw-stat matrix, official spell/rune metadata, filterable major events with champion filters and coordinate previews, per-player skill/item build order, team gold/economic-difference/player-stat timelines and replay actions. LCU and SGP match normalization retains the ten-player identities and scalar raw statistics, while the detail endpoint returns only the timeline fields required by the UI.
- Opt-in local respawn countdown in League Mini through the in-game Live Client Data endpoint; disabled by default.
- Thirty-second enriched live-game cache so frequent UI refreshes do not repeatedly request every player's history.
- League Mini ARAM bench card with current champion, bench choices, reroll count, manual swap and reroll actions.
- Independent resizable real-time match window, sharing the cached team/premade/champion-usage analysis with the main lab.
- SGP match-history fallback for Tencent and supported global regions, using an on-demand in-memory entitlements token and exposing the active LCU/SGP source in the player center.
- Full cross-region Riot ID lookup through the local Riot Client player-account alias endpoint, followed by target-server SGP summoner, ranked, challenge and match-history routing; Riot Client credentials remain memory-only.
- LeagueAkari-equivalent first-14-minute jungle timeline analysis for LCU and SGP details: start-camp inference, top/mid/bottom activity weights, gank participation, level-3/4 pressure, local unsent scouting drafts, player-center profiles and automatic current-jungler enrichment in the live ten-player view.
- League Mini owned-skin selector with chroma support; options come from the current LCU inventory snapshot and unowned/disabled IDs are rejected server-side.
- Visual rune and summoner-spell loadout editor backed by the current LCU catalog; perk selection no longer requires manually typing numeric IDs.
- LeagueAkari-style streamer privacy mode across the main League lab, player center, Mini and independent ongoing-game window, with stable optional aliases, local-tag/PUUID masking and optional native capture protection.
- Reversible `PersistedSettings.json` read-only/writable control using the LCU-reported install directory, including Tencent's separate `Game/Config` layout.
- LeagueClientUx window repair parity: read the live zoom scale, resize both the native `RCLIENT` shell and `CefBrowserWindow`, then center the client; the action is manual and confirmed.
- Card/subset champion-select parity: the server-provided subset list now gates automatic picks and bench swaps during `BAN_PICK`, and Arena's special `-3` bravery action is available as a first-class ordered pick choice.
- Login automation parity for chat-ready state: after `/lol-chat/v1/me` remains available for two seconds, the app can restore the saved status message and displayed ranked queue/tier/division once per client connection. Manual application interrupts that login pass, apex tiers omit division, disconnects reset the state, and every write remains disabled by default behind the master automation switch.
- LeagueAkari-style enemy summoner-spell timer: an independent transparent, always-on-top and non-focusable Tauri overlay follows supported `InProgress` modes, orders the enemy team by position, applies mode ability haste, supports countdown/countup and reversible wheel correction, and can send a generated game-clock callout only after an explicit double-right-click while `League of Legends.exe` is the foreground process. The feature and native input remain disabled by default.
- The previously added OP.GG recommendation window is not an upstream LeagueAkari feature and was explicitly removed from the product surface, native window lifecycle, runtime network routes and current settings model. Legacy OP.GG-shaped keys are accepted only long enough to discard them during settings migration; they are not persisted again and cannot create a window. Rune and spell behavior is provided by the integrated local champion-config workflow.
- Arbitrary Game ID preview and dry-run parity: the toolkit resolves a completed match through LCU with current-region SGP fallback, normalizes both team scoreboards, optionally loads the timeline summary, and can route the historical roster into the existing ongoing-game panel without writing any client state.
- Configurable global game-termination shortcut parity: the Tauri global-shortcut plugin is scoped to the main window, registration is driven by persisted League settings, the feature defaults off, and every trigger still passes the backend foreground-process guard before `League of Legends.exe` can be terminated.
- In-game-send parity: fixed-text presets support ordered multiline content, optional global shortcuts and an independent cancel shortcut, while recent-form, premade and jungle-analysis drafts support friendly/enemy/all targeting with nine independent target shortcuts. Lobby and ChampSelect use the matching LCU conversation; InProgress sends one line at a time only after re-reading the live phase and verifying `League of Legends.exe` remains foreground. The account-write gate defaults on; the feature switch and every preset shortcut default off, while manual sends require the exact confirmation phrase. Global show shortcuts are available for the stateful ongoing-game window and cooldown timer.
- League settings export/import parity: the client exports a versioned local JSON document, recursively excludes credential-like fields, accepts only settings known by the installed version and rejects future schemas. Import never restores the automation, account-write, in-game-send or process-termination master switches; legacy OP.GG auto-apply keys are also forced off during migration.
- Tagged-player storage parity: tags are keyed by the active local account plus target PUUID while retaining a legacy global fallback, and the toolkit provides search, current-account filtering, pagination, player-center routing, inline editing, selected deletion and versioned JSON import/export. Streamer mode defers loading and masks the manager until the user explicitly reveals it for the current session.
- Champion-analysis parity: the player center aggregates every loaded sample by champion, exposes win/loss, KDA line, the MIT-licensed upstream Akari Score model, damage/CS per minute, vision, team damage/taken/gold shares and a position-distribution chart, with a low-sample warning and direct switching among the twelve most-played champions.
- Player-summary parity: the same upstream scoring model is also applied to the complete loaded sample alongside aggregate KDA, win/loss and streak, active-session results, kill participation, CS per minute, team damage/taken/gold shares and blue/red-side counts.
- Ongoing-game settings parity: per-player query concurrency and premade co-play threshold are configurable; live cards can independently show streak and recent-form tags derived from win rate, KDA, CS, vision and solo kills. Team headers expose the sampled average win rate.
- Ongoing-game query parity: Lobby rosters are analyzed before ChampSelect, queue filtering accepts either queue ID or queue type, detail enrichment can be capped independently from displayed history, players support six deterministic sort modes, champion usage can be sourced from recent games, mastery or hidden, and the host can show all-player jungle samples plus match borders.
- Player-search pane parity: searches create account-scoped recent visits with local filtering, pinning and deletion; the same pane lists current LCU friends and their presence state, and exposes a deliberate one-click spectate action only when live LCU state says the friend is spectatable. Spectator keys never leave the backend.
- Player-card tag parity: upstream-style self, local-tag, encountered, privacy, win-rate, streak, great-performance, mixed Flash-position, solo-kill, team-share, CS, vision, damage/gold, kill/damage and optional Akari Score labels are independently configurable. Every computed label carries its sample and formula explanation as a hover tooltip.
- League Mini ChampSelect operations: the user's pick/ban/vote action timeline is visible, automatic selection can be paused only for the current ChampSelect, and explicit dodge is available behind the account-write gate, an inline second confirmation and a fresh live-phase check.
- Auxiliary-window settings parity: Mini opacity, Mini skin-selector visibility and explicit position reset actions are persisted and exposed in the host settings. The Tauri shell persists position and size for Mini, ongoing-game and cooldown windows without persisting visibility; reset restores the documented default size, recenters the existing native window and immediately replaces its saved geometry. Mini renders an immediate dark bootstrap/error surface, exposes pin/minimize/close controls, and automatic phase display no longer steals foreground focus.

### Implemented with deliberate React/Tauri presentation differences

- Pick/ban: core upstream behavior, subset-card modes and Arena bravery are implemented. The editor uses the live LCU champion catalog, official champion artwork, role filtering and ordered selection. Mode routing is expressed as stable semantic profiles instead of copying LeagueAkari's internal queue-group object names.
- Champion config: saved loadouts plus LCU-backed champion artwork search, primary/secondary rune selection, named summoner-spell selection and LeagueAkari-equivalent normal/ranked-position/ARAM/URF/Nexus Blitz/Ultimate Spellbook routing with fallback are implemented.
- Honor: strategy parity is implemented; the UI intentionally keeps the feature opt-in and disabled by default.
- Invitations: per-type strategy, priority ordering, away gating and the complete upstream dynamic queue-type strategy catalog are implemented.
- Match history: current and cross-player rows, Riot ID lookup, pagination, ranked/mastery summary, recently encountered players, local tags, basic filters, named presets, SGP fallback, one-click 100-match SQLite collection, aggregate performance metrics, SGP collection challenges and expanded match cards exist. The rule editor supports arbitrarily nested AND/OR/NOT groups over game identity/result/time, champion/position, spells/items/perks/augments, KDA/multikills, combat, vision and economy fields, targeting the current player or any/every ally, enemy or other participant.
- Mini and auxiliary windows: phase-driven show/hide, safe manual close, ARAM bench swap/reroll, owned-skin selection, respawn countdown, live ReadyCheck/pick-ban/matchmaking/phase-action countdowns and champ-select phase timer are implemented. Mini uses a dedicated dark bootstrap document so packaged WebViews cannot expose an unpainted white surface. Independent ongoing-game and spell-cooldown windows retain separate lifecycle state.

### Deliberately not exposed because upstream is incomplete

- Loot crafting/redeeming is intentionally not exposed: the reviewed upstream `LootTools.vue` labels itself under development and leaves its `craft` handler empty. Read-only inventory parity is retained until upstream itself has a working, auditable user flow.

## User-surface parity audit

This table is stricter than the shard matrix below: a backend capability is not counted as a finished user surface until the corresponding React/Tauri view and interaction have been reviewed. “Partial” is an explicit remaining-work marker, not a completion claim.

| Upstream user surface | Local surface | Current evidence | Status |
| --- | --- | --- | --- |
| Startup and client connection | League lab connection/startup cards | multi-client discovery, exact-PID binding and Riot/WeGame/TCLS launch cards | Equivalent |
| Player tabs and summoner search | integrated player center | current/cross-region Riot ID lookup, account-scoped recent visits with pin/delete, friend presence and explicit spectate, recent tabs, ranked, mastery, challenges, account-scoped tags with full management/import/export, pagination, collection, whole-sample summary/Akari Score and per-champion aggregate/position analysis | Equivalent with host layout; real friend-spectate acceptance pending |
| Match-history overview | current-account history and player-center detailed cards | configurable load count, post-game refresh, collapsed result/performance row, ten-player roster, replay action and expandable tabs | Equivalent foundation |
| Match summary | `双方总览` | both teams, clickable players, K/D/A, KP, damage share, CS/gold and loadouts | Equivalent foundation |
| Raw match details | `详细属性` | searchable horizontal ten-player scalar-stat matrix with sticky headers, game/source/version/map metadata, common Chinese labels, click-open ten-player comparison charts, upstream-compatible grouped taxonomy and internal-field hiding | Equivalent with host layout |
| Runes | `符文` | all participants, official spell/perk artwork, catalog names/descriptions and augment IDs | Equivalent foundation; post-game computed description fidelity still needs real SGP fixtures |
| Events | `事件` | chronological major-event and champion filters, actor/victim context, plate totals and coordinate previews | Equivalent foundation; official Riot map backgrounds remain license-gated |
| Builds | `出装过程` | per-player navigator, numbered skill/evolution order, timestamped purchase/sell/undo operations, 30-second purchase-stage spacers and anvil counts | Equivalent with host layout |
| Timeline | `时间线` | team-gold lines, economic-difference line, selectable per-player gold/level/XP/CS/damage series, native hover values, frame slider, map position and SGP champion-stat panel | Equivalent with host layout |
| Encountered games | integrated encounter history | account-isolated shared games, pagination and local single-row removal | Equivalent with host layout |
| Ongoing game | main and independent ongoing views | Lobby/ChampSelect/InProgress rosters, queue filter, six sort modes, separate detail-sample cap, recent/mastery/hidden champion usage, configurable premade threshold, match borders, independently configurable upstream-style labels with calculation tooltips, team recent win-rate and configurable all-player jungle analysis, plus optional automatic routing at game start | Equivalent foundation; live Tencent Lobby/ChampSelect window acceptance still pending |
| Automation | League automation sections | gameflow, selection/ban, champion config and miscellaneous behavior | Equivalent; real Tencent state-machine acceptance pending |
| Toolkit | League toolkit sections | lobby/client/profile/rewards/friends/in-game-send and preview actions | Equivalent except deliberately excluded unfinished loot crafting |
| Mini and auxiliary windows | Mini, ongoing and cooldown windows | phase lifecycle, pinning, close suppression, ARAM actions, own action timeline, temporary auto-select pause, confirmed dodge, opacity/skin visibility settings, native position/size persistence and reset, and auxiliary tools | Equivalent; real-window lifecycle and geometry-restore acceptance pending |
| Settings and shell | integrated settings plus Tauri shell | theme/privacy/capture/shortcuts/tray/updater, persisted League settings and safe JSON export/import | Host equivalent |

### Explicit remaining gaps

- The current parity candidate adds a normalized `auto_select` evidence contract covering the ten upstream move types, delayed plans, expected pick/ban/swap candidates, trades and actionability. Mini now exposes ReadyCheck, matchmaking, auto-select plans and manual champion-swap decisions; manual account writes remain behind the default-on toolkit gate while automatic writes require the master switch, feature switch, enabled profile and a compatible phase.
- Game ID preview now loads summary data first and fetches full details only on demand, reusing the detailed match card. Ongoing-game player cards and the player center now expose expandable recent matches, richer metrics/tags and dedicated overview/history/mastery/challenge/encounter views without fabricating unavailable values.
- Candidate verification for this batch: backend `895 passed`, frontend `743 passed`, Tauri library `3 passed`, and the frontend production build completed successfully. These are offline/code-level results only and do not replace live Tencent state-machine acceptance.
- Installed `2.5.14-rc.43` Lobby acceptance (2026-08-18): the packaged desktop connected to the live Tencent client, identified the current account and `Lobby` phase, and kept the LCU event stream online. Enabling only the local Mini-window feature produced the expected automatic Lobby window at the upstream 340 x 420 default with rendered dark content rather than a white/blank document. The Mini showed the Lobby context and disabled manual-write controls while `automation_enabled`, all feature automation switches and `toolkit_account_actions_enabled` remained false. This proves the read/display path for Lobby only; it does not prove ReadyCheck, ChampSelect, InProgress or EndOfGame behavior.
- Real Tencent/WeGame acceptance is still required for the account-affecting ReadyCheck, ChampSelect, InProgress and EndOfGame write operations. The rc.40 connection and event-stream result does not count as acceptance of those writes; mocked LCU tests are not counted as that acceptance.
- Lobby roster analysis and friend spectating still require a real, explicitly authorized live-state acceptance.
- Mini close/persistence and the read/display paths for Mini and cooldown were exercised in rc.40, but that evidence does not establish complete lifecycle parity for every auxiliary window or every game phase. OP.GG was later removed from scope and its old rc.40 evidence is not candidate evidence.
- Loot crafting/redeeming remains deliberately excluded because the reviewed upstream handler is incomplete.

## Shard-to-host traceability matrix

This is the completion checklist against the authoritative registration list in upstream `src/main/bootstrap/index.ts`. “Host equivalent” means the responsibility is provided by the existing React/Tauri/FastAPI shell instead of copying Electron-only plumbing.

| Upstream shard | Local evidence surface | Status |
| --- | --- | --- |
| `akari-api` | GitHub updater channel and bundled configuration; no upstream announcement/bootstrap service dependency | Host equivalent |
| `akari-protocol`, `ipc` | FastAPI routes plus Tauri commands/capabilities | Host equivalent |
| `app-common` | theme, locale, streamer privacy and capture protection | Equivalent |
| `logger-factory` | Python/Rust application logging | Host equivalent |
| `mobx-utils` | React/Zustand state and LCU WebSocket event propagation | Host equivalent |
| `config-migrate`, `setting-factory`, `storage` | versioned Pydantic settings, local SQLite/JSON persistence and credential-free settings export/import | Host equivalent |
| `game-client` | foreground guard, termination shortcut, Live Client Data and settings-file control | Equivalent |
| `league-client`, `league-client-ux`, `riot-client` | native all-process command-line discovery, exact-PID chooser, memory-only credentials, HTTPS and WebSocket clients | Equivalent |
| `client-installation` | TCLS, per-installation WeGame launcher, standalone WeGame and Riot Client registry/file detection plus explicit launch cards | Equivalent |
| `window-manager`, `tray` | Tauri main/Mini/ongoing/cooldown windows and close-to-tray lifecycle | Equivalent |
| `keyboard-shortcuts` | Tauri global-shortcut managers with default-off settings | Equivalent |
| `self-update` | signed Tauri GitHub updater | Host equivalent |
| `feature-gating` | features are bundled and locally gated by explicit settings; no remote kill switch | Deliberate host policy |
| `auto-champ-config` | visual rune/spell loadouts with mode/position routing | Equivalent |
| `auto-gameflow` | ready check, honor, rematch, invitations, matchmaking and ARAM side state machines | Equivalent |
| `auto-misc` | reply, presence lock, login status/rank and friend-invite queue | Equivalent |
| `auto-select` | mode/position profiles, intent, subset/Arena, bench and trade behavior | Equivalent |
| `in-game-send` | fixed/form/premade/jungle presets, target shortcuts and foreground guard | Equivalent |
| `ongoing-game` | enriched ten-player view, premade inference, champion usage and jungle timelines | Equivalent |
| `respawn-timer` | opt-in Live Client Data timer in Mini | Equivalent |
| `saved-player` | account-scoped durable tags, tag management/import/export, recent encounters and player-center routing | Equivalent |
| `sgp` | Tencent/global SGP routing with in-memory entitlements token and LCU fallback | Equivalent |
| `statistics` | upstream version telemetry is not transmitted | Deliberately excluded for privacy |
| `extra-assets` | LCU-backed champion/rune/spell/skin image proxy and bundled map metadata | Host equivalent |
| `renderer-debug` | development-only diagnostics are not shipped as user features | Deliberately excluded |

## Porting decisions

1. Port the mature state machines and endpoint behavior, not the Electron/Vue shell wholesale. React/Tauri remains the single desktop shell.
2. Replace one-second polling with LCU WebSocket event subscriptions before adding more automation; polling remains only as recovery/fallback.
3. Add SGP only after its regional authentication lifecycle, expiry and failure fallback are implemented. Do not expose tokens or persist them in plaintext.
4. Treat account-impacting or spam-prone toolkit features as opt-in and disabled by default. Destructive loot/reward operations require a separate safety review.
5. Preserve LeagueAkari attribution and MIT notice for adapted code. Do not copy artwork or third-party assets without verifying their individual licenses.

## Implementation order

1. Connection/state foundation: native discovery hardening, LCU event stream, reconnect state, Tencent/Riot fixtures.
2. Automation parity: matchmaking, leader/invitation rules, position-aware selection, intent/bench/trade behavior, visual rune/spell editor and honor strategies.
3. Player center: SGP + LCU source routing, full match cards, pagination, filters, ranked/mastery/challenges, recent players and tags.
4. Live assistant: ongoing-game analysis, premade/team insights and phase-specific Mini window.
5. Optional toolkit: only individually reviewed, clearly labeled and opt-in modules.

The formal release remains blocked: the installed candidate is unsigned, and ReadyCheck, ChampSelect, InProgress and EndOfGame account-affecting write operations have not been live-accepted on the user's Tencent account. No “1:1 complete” claim is authorized by this audit.
