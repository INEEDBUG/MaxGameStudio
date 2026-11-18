param([Parameter(Mandatory = $true)][string]$SourceRoot)

$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path

function Update-StandaloneSource([string]$RelativePath, [string]$Anchor, [string]$Replacement) {
  $target = Join-Path $SourceRoot $RelativePath
  $original = [IO.File]::ReadAllText($target)
  $newline = if ($original.Contains("`r`n")) { "`r`n" } else { "`n" }
  # Shared staging can insert Windows newlines into an LF source checkout.
  $original = ($original -replace "\r?\n", $newline)
  $anchorText = ($Anchor -replace "\r?\n", $newline)
  $replacementText = ($Replacement -replace "\r?\n", $newline)
  if ([regex]::Matches($original, [regex]::Escape($anchorText)).Count -ne 1) {
    throw "Expected exactly one standalone patch anchor in $RelativePath ($Anchor)."
  }
  [IO.File]::WriteAllText($target, $original.Replace($anchorText, $replacementText), $utf8)
}

$profileDir = Join-Path $SourceRoot 'src\main\standalone-profile'
New-Item -ItemType Directory -Path $profileDir -Force | Out-Null
foreach ($name in @('profile-bootstrap.ts', 'league-profile.ts')) {
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $profileDir
}
$overviewDir = Join-Path $SourceRoot 'src\renderer\src-main-window\views\league-overview'
New-Item -ItemType Directory -Path $overviewDir -Force | Out-Null
foreach ($name in @('LeagueOverview.vue', 'LeagueUpdateButton.vue', 'league-update.ts')) {
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $overviewDir
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'frontend\public\brand-icons\leagueoflegends.svg') -Destination $overviewDir
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'installer.nsh') -Destination (Join-Path $SourceRoot 'build\mgs-league-installer.nsh')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'destination-guard.ps1') -Destination (Join-Path $SourceRoot 'build\destination-guard.ps1')

$main = 'src\main\main.ts'
Update-StandaloneSource $main "import { bootstrap } from './bootstrap'" @'
import { bootstrap } from './bootstrap'
import { prepareLeagueProfile } from './standalone-profile/profile-bootstrap'

app.setName('MaxGameStudio League')
'@
Update-StandaloneSource $main 'if (gotTheLock) {' 'if (gotTheLock && (!app.isPackaged || prepareLeagueProfile())) {'

$logger = 'src\main\logger\index.ts'
$oldLoggerPath = @'
  // macOS 下应用通常安装在 /Applications（对普通用户不可写），不能把日志写进 .app bundle 内。
  // Windows（NSIS per-user）等场景下保留原逻辑：写到可执行文件目录旁的 logs。
  const logsDir =
    (process.platform === 'darwin' || process.env.MAXGAMESTUDIO_EMBEDDED === '1')
      ? app.getPath('logs')
      : path.join(path.join(app.getPath('exe'), '..'), 'logs')
'@
Update-StandaloneSource $logger $oldLoggerPath "  const logsDir = app.getPath('logs')"

$protocol = 'src\main\utils\deep-link.ts'
Update-StandaloneSource $protocol "is.dev ? 'league-akari-dev' : 'league-akari'" "is.dev ? 'maxgamestudio-league-dev' : 'maxgamestudio-league'"
Update-StandaloneSource $protocol "DEEP_LINK_PROTOCOL_PROD = 'league-akari'" "DEEP_LINK_PROTOCOL_PROD = 'maxgamestudio-league'"
Update-StandaloneSource $protocol "DEEP_LINK_PROTOCOL_DEV = 'league-akari-dev'" "DEEP_LINK_PROTOCOL_DEV = 'maxgamestudio-league-dev'"

$manifest = 'electron-builder.yml'
Update-StandaloneSource $manifest 'appId: com.maxgamestudio.league' 'appId: com.maxgamestudio.league.toolbox'
Update-StandaloneSource $manifest 'executableName: MaxGameStudioLeague' 'executableName: MaxGameStudioLeagueToolbox'
Update-StandaloneSource $manifest "    - 7z" "    - nsis"
Update-StandaloneSource $manifest 'asar: true' @'
nsis:
  artifactName: 'MaxGameStudioLeague_${version}_x64-setup.${ext}'
  oneClick: false
  perMachine: false
  allowElevation: false
  selectPerMachineByDefault: false
  allowToChangeInstallationDirectory: false
  deleteAppDataOnUninstall: false
  installerLanguages: [zh_CN]
  language: '2052'
  shortcutName: 英雄联盟工具箱
  uninstallDisplayName: 英雄联盟工具箱
  include: build/mgs-league-installer.nsh
asar: true
'@

$renderer = 'src\renderer\src-main-window'
$routes = Join-Path $renderer 'routes\index.ts'
foreach ($pair in @(
  @('Automation', 'automation/Automation'), @('OngoingGame', 'ongoing-game/OngoingGame'),
  @('PlayerTabs', 'player-tabs/PlayerTabs'), @('Test', 'test/Test'), @('Toolkit', 'toolkit/Toolkit')
)) {
  Update-StandaloneSource $routes "import $($pair[0]) from '@main-window/views/$($pair[1]).vue'" ''
  Update-StandaloneSource $routes "component: $($pair[0])" "component: () => import('@main-window/views/$($pair[1]).vue')"
}
Update-StandaloneSource $routes "redirect: { name: 'player-tabs' }" @'
redirect: { name: 'league-overview' }
    },
    {
      name: 'league-overview',
      path: '/overview',
      component: () => import('@main-window/views/league-overview/LeagueOverview.vue')
'@
Update-StandaloneSource (Join-Path $renderer 'App.vue') '<KeepAlive>' '<KeepAlive :max="3">'
$mainWindow = 'src\main\shards\window-manager\main-window\window.ts'
Update-StandaloneSource $mainWindow "static readonly TITLE = 'MaxGameStudio'" "static readonly TITLE = '英雄联盟工具箱'"
Update-StandaloneSource 'src\renderer\main-window.html' '<title>MaxGameStudio</title>' '<title>英雄联盟工具箱</title>'
Update-StandaloneSource $mainWindow '        maximizable: true,' @'
        maximizable: true,
        webPreferences: { backgroundThrottling: true },
'@

$windowManager = 'src\main\shards\window-manager'
$baseWindow = Join-Path $windowManager 'base-akari-window.ts'
Update-StandaloneSource $baseWindow '  htmlEntry: string | { path: string; hash: string }' @'
  htmlEntry: string | { path: string; hash: string }
  canCreateOnDemand?: () => boolean
'@
Update-StandaloneSource $baseWindow '  createWindow() {' @'
  private _prepareManualShow() {
    const canCreate = this._config.canCreateOnDemand
    if (!canCreate) return true
    if (!canCreate()) return false
    this.createWindow()
    return true
  }

  createWindow() {
'@
foreach ($method in @('showOrRestore', 'show')) {
  Update-StandaloneSource $baseWindow ("  $method(inactive = false) {" + [Environment]::NewLine + '    if (isPrewarming()) return') (
    "  $method(inactive = false) {" + [Environment]::NewLine + '    if (isPrewarming() || !this._prepareManualShow()) return'
  )
}

Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'companion-window-demand.ts') -Destination (Join-Path $SourceRoot $windowManager)
foreach ($kind in @('aux', 'opgg')) {
  $companion = Join-Path $windowManager "$kind-window\window.ts"
  Update-StandaloneSource $companion "import { comparer, computed } from 'mobx'" "import { computed } from 'mobx'"
  Update-StandaloneSource $companion "import { BaseAkariWindow } from '../base-akari-window'" @'
import { BaseAkariWindow } from '../base-akari-window'
import { companionWindowDemand } from '../companion-window-demand'
'@
  Update-StandaloneSource $companion '      repositionWindowIfInvisible: true,' @'
      repositionWindowIfInvisible: true,
      canCreateOnDemand: () => settings.enabled && _context.windowManager.state.isManagerFinishedInit,
'@
  $companionSource = [IO.File]::ReadAllText((Join-Path $SourceRoot $companion))
  $oldWatcher = [regex]::Match($companionSource, '(?s)    const showTiming = computed\(\(\) => \{.*?\{ fireImmediately: true, delay: 500, equals: comparer\.shallow \}\r?\n    \)')
  if (-not $oldWatcher.Success) { throw "Pinned companion startup watcher was not found: $companion" }
  $watcher = @"
    const showTiming = computed(() =>
      companionWindowDemand('$kind', {
        enabled: this.settings.enabled,
        initialized: this._windowManager.state.isManagerFinishedInit,
        connected: this._leagueClient.state.connectionState === 'connected',
        autoShow: this.settings.autoShow,
        ready: this.state.ready,
        phase: this._leagueClient.data.gameflow.phase,
        spectating: this._leagueClient.data.champSelect.session?.isSpectating
      })
    )

    this._mobxUtils.reaction(
      () => showTiming.get(),
      (timing) => {
        if (timing === 'prepare') this.createWindow()
        if (timing === 'show') this.showOrRestore()
        if (timing === 'hide') this.hide()
      },
      { fireImmediately: true }
    )

    this._mobxUtils.reaction(
      () => this.settings.enabled,
      (enabled) => {
        if (!enabled) this.close(true)
      }
    )
"@
  Update-StandaloneSource $companion $oldWatcher.Value $watcher

  # The standard tray item opens lazily. The developer item still needs a renderer.
  $trayWatcher = 'src\main\shards\tray\tray-state-watcher.ts'
  $oldTray = @"
        if (enabled && ready) {
          this.menuController.$($kind)WindowDevTrayItem.enabled = true
          this.menuController.$($kind)WindowTrayItem.enabled = true
        } else {
          this.menuController.$($kind)WindowDevTrayItem.enabled = false
          this.menuController.$($kind)WindowTrayItem.enabled = false
        }
"@
  $newTray = @"
        this.menuController.$($kind)WindowDevTrayItem.enabled = enabled && ready
        this.menuController.$($kind)WindowTrayItem.enabled = enabled
"@
  Update-StandaloneSource $trayWatcher $oldTray $newTray
}

$sidebar = Join-Path $renderer 'components\sidebar\Sidebar.vue'
Update-StandaloneSource (Join-Path $renderer 'shards\main-window-ui\store.ts') 'sidebarCollapsed: true,' 'sidebarCollapsed: false,'
Update-StandaloneSource $sidebar "import AkariLogo from '@renderer-shared/assets/icon/AkariLogo.vue'" @'
import leagueLogo from '@main-window/views/league-overview/leagueoflegends.svg?url'
import { Home24Regular as HomeIcon } from '@vicons/fluent'
import { useMainWindowAppContext } from '@main-window/context'
'@
Update-StandaloneSource $sidebar '<AkariLogo />' '<span class="league-sidebar-mark" :style="{ maskImage: ''url('' + leagueLogo + '')'' }" aria-hidden="true"></span>'
Update-StandaloneSource $sidebar "{{ t('appName', { ns: 'common' }) }}" "{{ english ? 'League Toolbox' : '英雄联盟工具箱' }}"
Update-StandaloneSource $sidebar 'const as = useAppCommonStore()' @'
const as = useAppCommonStore()
const english = computed(() => as.settings.locale.startsWith('en'))
const { openSettingsModal } = useMainWindowAppContext()
'@
Update-StandaloneSource $sidebar "    {`n      key: 'player-tabs'," @'
    {
      key: 'league-overview',
      icon: renderIcon(HomeIcon),
      name: english.value ? 'Overview' : '总览'
    },
    {
      key: 'player-tabs',
'@
Update-StandaloneSource $sidebar '      name: t(''navigation.sidebar.menu.toolkit'')' @'
      name: t('navigation.sidebar.menu.toolkit')
    },
    {
      key: 'league-lookup',
      icon: renderIcon(AnalyticsRoundIcon),
      name: english.value ? 'Player lookup' : '玩家查询'
    },
    {
      key: 'league-runes',
      icon: renderIcon(ToolFilledIcon),
      name: english.value ? 'Runes and builds' : '符文与英雄配置'
    },
    {
      key: 'league-windows',
      icon: renderIcon(Games24FilledIcon),
      name: english.value ? 'Companion windows' : '辅助窗口'
'@
Update-StandaloneSource $sidebar '    await router.replace({ name: val })' @'
    if (val === 'league-windows') return openSettingsModal('multi-window')
    if (val === 'league-runes') return router.replace({ name: 'automation', params: { section: 'auto-champ-config' } })
    if (val === 'league-lookup') return router.replace({ name: 'league-overview', hash: '#league-lookup' })
    await router.replace({ name: val })
'@
Update-StandaloneSource $sidebar '<style scoped>' @'
<style scoped>
.league-sidebar-mark {
  display: block;
  width: 26px;
  height: 26px;
  background: #906a21;
  mask-size: contain;
  mask-position: center;
  mask-repeat: no-repeat;
}
:global([data-theme='dark']) .league-sidebar-mark { background: #d5b56c; }
'@

$about = Join-Path $renderer 'components\settings-modal\AboutPane.vue'
Update-StandaloneSource $about '<div class="flex justify-center">' @'
<div class="flex flex-wrap items-center justify-center gap-3">
          <LeagueUpdateButton />
'@
Update-StandaloneSource $about '<script setup lang="tsx">' @'
<script setup lang="tsx">
import LeagueUpdateButton from '@main-window/views/league-overview/LeagueUpdateButton.vue'
'@

$settings = Join-Path $renderer 'components\settings-modal\AppSettings.vue'
$settingsPath = Join-Path $SourceRoot $settings
$settingsSource = [IO.File]::ReadAllText($settingsPath)
$oldSection = [regex]::Match($settingsSource, '(?s)<SettingsSection\s+:title="t\(''settings\.app\.selfUpdate\.title''\)".*?</SettingsSection>')
if (-not $oldSection.Success) { throw 'Pinned embedded updater settings block not found.' }
Update-StandaloneSource $settings $oldSection.Value @'
<SettingsSection :title="t('settings.app.selfUpdate.title')">
        <SettingsRow :label="t('settings.app.selfUpdate.autoCheckUpdates.label')">
          <NSwitch :value="sus.settings.autoCheckUpdates" @update:value="(val) => su.setAutoCheckUpdates(val)" />
        </SettingsRow>
        <LeagueUpdateButton />
      </SettingsSection>
'@
$settingsSource = [IO.File]::ReadAllText($settingsPath)
$oldHandlers = [regex]::Match($settingsSource, '(?s)const message = useMessage\(\).*?(?=</script>)')
if (-not $oldHandlers.Success) { throw 'Pinned embedded updater settings handlers not found.' }
Update-StandaloneSource $settings $oldHandlers.Value ''
foreach ($unused in @(
  "import { formatSeconds } from '@shared/utils/format'",
  "import { useMediaQuery } from '@vueuse/core'",
  "import { SimpleNotificationsRenderer } from '@main-window/shards/simple-notifications'",
  "const sn = useInstance(SimpleNotificationsRenderer)",
  "  NStep,", "  NSteps,", "  useMessage"
)) {
  Update-StandaloneSource $settings $unused ''
}
Update-StandaloneSource $settings '<script setup lang="ts">' @'
<script setup lang="ts">
import LeagueUpdateButton from '@main-window/views/league-overview/LeagueUpdateButton.vue'
'@
