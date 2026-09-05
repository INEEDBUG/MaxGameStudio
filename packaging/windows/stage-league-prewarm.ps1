#Requires -Version 5.1
<##
.SYNOPSIS
  Inject the opt-in, local-file embedded prewarm gate into the pinned League source.

.DESCRIPTION
  This helper is called only by stage-league-runtime.ps1 after the pinned source
  has been copied to its private staging tree. It deliberately patches exact
  upstream anchors and refuses to guess when the pinned source changes.
##>
param(
  [Parameter(Mandatory = $true)]
  [string]$SourceRoot
)

$ErrorActionPreference = 'Stop'

function Replace-TextExactlyOnce([string]$PathValue, [string]$OldValue, [string]$NewValue) {
  if (-not (Test-Path -LiteralPath $PathValue -PathType Leaf)) {
    throw "Prewarm patch target is missing: $PathValue"
  }
  $text = Get-Content -LiteralPath $PathValue -Raw -Encoding utf8
  Write-Verbose "prewarm patch $PathValue old=[$OldValue]"
  $hits = ([regex]::Matches($text, [regex]::Escape($OldValue))).Count
  if ($hits -ne 1) {
    throw "Expected exactly one prewarm patch match in $PathValue (found $hits)."
  }
  [IO.File]::WriteAllText($PathValue, $text.Replace($OldValue, $NewValue), [Text.UTF8Encoding]::new($false))
}

function Add-PrewarmImport([string]$PathValue, [string]$ImportLine) {
  $text = Get-Content -LiteralPath $PathValue -Raw -Encoding utf8
  if (-not $text.Contains($ImportLine)) {
    $firstImport = [regex]::Match($text, '(?m)^import .*$')
    if (-not $firstImport.Success) { throw "No import anchor found in $PathValue" }
    $inserted = $text.Insert($firstImport.Index, $ImportLine + [Environment]::NewLine)
    [IO.File]::WriteAllText($PathValue, $inserted, [Text.UTF8Encoding]::new($false))
  }
}

$modulePath = Join-Path $SourceRoot 'src\main\mgs-prewarm.ts'
$module = @'
import fs from 'node:fs'
import path from 'node:path'

const prewarmRequested = process.argv.includes('--maxgamestudio-prewarm')
const sessionArgument = process.argv.find((argument) => argument.startsWith('--maxgamestudio-prewarm-session='))
const profileArgument = process.argv.find((argument) => argument.startsWith('--user-data-dir='))
const session = sessionArgument?.slice('--maxgamestudio-prewarm-session='.length) || ''
const profile = profileArgument?.slice('--user-data-dir='.length) || ''
const validSession = /^[0-9a-f]{32}$/i.test(session)
const validProfile = path.isAbsolute(profile) || path.win32.isAbsolute(profile)
if (prewarmRequested && (!validSession || !validProfile)) {
  throw new Error('Invalid MaxGameStudio prewarm session or profile path')
}
const prewarming = prewarmRequested
const activateFileName = `prewarm-${session}.activate`
const profileMarker = (name: string) => path.join(profile, `prewarm-${session}.${name}`)
let activated = !prewarming
let activationRequested = false
let rendererReady = false
let readyReported = false
let shownReported = false
const activationQueue: Array<{ callback: () => void; priority: number }> = []
let watcher: fs.FSWatcher | null = null

export function isPrewarming() {
  return prewarming && !activated
}

export function onActivated(callback: () => void, priority = 0) {
  if (isPrewarming()) {
    activationQueue.push({ callback, priority })
  } else {
    callback()
  }
}

export function reportReady() {
  if (!prewarming || readyReported) return
  rendererReady = true
  readyReported = true
  try {
    fs.writeFileSync(profileMarker('ready'), '', { flag: 'wx' })
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') readyReported = false
  }
  activateWhenReady()
}

export function reportShown() {
  if (!prewarming || shownReported) return
  shownReported = true
  try {
    fs.writeFileSync(profileMarker('shown'), '', { flag: 'wx' })
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') shownReported = false
  }
}

function activate() {
  if (!isPrewarming()) return
  activationRequested = true
  watcher?.close()
  watcher = null
  activateWhenReady()
}

function activateWhenReady() {
  // A quick click must not start client discovery/automation while Chromium
  // is still creating the main page. Show the real window first in BOTH paths.
  if (!isPrewarming() || !activationRequested || !rendererReady) return
  activated = true
  const callbacks = activationQueue.splice(0).sort((a, b) => b.priority - a.priority)
  callbacks.forEach(({ callback, priority }) => {
    if (priority > 0) {
      callback()
    } else {
      setImmediate(callback)
    }
  })
}

if (prewarming) {
  watcher = fs.watch(profile, (_event, filename) => {
    if (filename?.toString() === activateFileName && fs.existsSync(path.join(profile, activateFileName))) {
      activate()
    }
  })
  if (fs.existsSync(path.join(profile, activateFileName))) activate()
}
'@
New-Item -ItemType Directory -Path (Split-Path -Parent $modulePath) -Force | Out-Null
[void](Get-Item -LiteralPath $modulePath -ErrorAction SilentlyContinue)
[IO.File]::WriteAllText($modulePath, $module, [Text.UTF8Encoding]::new($false))

$nl = [Environment]::NewLine
$baseWindow = Join-Path $SourceRoot 'src\main\shards\window-manager\base-akari-window.ts'
Add-PrewarmImport $baseWindow "import { isPrewarming, reportReady } from '@main/mgs-prewarm'"
$readyAnchor = @(
  '      this._logger.info(`BrowserWindow ready-to-show (${this._namespace})`)',
  '',
  '      if (this._forceReadyTimerId) {',
  '        clearTimeout(this._forceReadyTimerId)',
  '        this._forceReadyTimerId = null',
  '      }',
  '      runInAction(() => (this.state.ready = true))'
) -join $nl
Replace-TextExactlyOnce $baseWindow $readyAnchor ($readyAnchor + $nl + "      if (this._namespaceSuffix === 'main-window') {" + $nl + '        reportReady()' + $nl + '      }')
$fallbackReadyAnchor = @(
  '        this._logger.warn(`WebContents force-ready (${this._namespace})`)',
  '        runInAction(() => (this.state.ready = true))'
) -join $nl
Replace-TextExactlyOnce $baseWindow $fallbackReadyAnchor ($fallbackReadyAnchor + $nl + "        if (this._namespaceSuffix === 'main-window') reportReady()")
Replace-TextExactlyOnce $baseWindow ("  showOrRestore(inactive = false) {" + $nl + "    if (this._window) {") ("  showOrRestore(inactive = false) {" + $nl + "    if (isPrewarming()) return" + $nl + "    if (this._window) {")
Replace-TextExactlyOnce $baseWindow ("  show(inactive = false) {" + $nl + "    if (this._window && !this.state.show) {") ("  show(inactive = false) {" + $nl + "    if (isPrewarming()) return" + $nl + "    if (this._window && !this.state.show) {")
Replace-TextExactlyOnce $baseWindow "    this._context.ipc.onCall(this._namespace, 'restore', async () => {" ("    this._context.ipc.onCall(this._namespace, 'restore', async () => {" + $nl + "      if (isPrewarming()) return")
Replace-TextExactlyOnce $baseWindow "  toggleMinimizedAndFocused() {" ("  toggleMinimizedAndFocused() {" + $nl + "    if (isPrewarming()) return")
Replace-TextExactlyOnce $baseWindow '      ...rest' '      ...rest,'
Replace-TextExactlyOnce $baseWindow '      ...rest,' ("      ...rest," + $nl + "      show: isPrewarming() ? false : rest.show")
Replace-TextExactlyOnce $baseWindow "      this.state.show = this._config.browserWindowOptions?.show ?? true" "      this.state.show = this._window?.isVisible() ?? (this._config.browserWindowOptions?.show ?? true)"

$mainWindow = Join-Path $SourceRoot 'src\main\shards\window-manager\main-window\window.ts'
Add-PrewarmImport $mainWindow "import { onActivated, reportShown } from '@main/mgs-prewarm'"
Replace-TextExactlyOnce $mainWindow ("        if (ready) {" + $nl + "          this.showOrRestore()" + $nl + "        }") ("        if (ready) {" + $nl + "          onActivated(() => {" + $nl + "            this.showOrRestore()" + $nl + "            if (this.window?.isVisible()) {" + $nl + "              reportShown()" + $nl + "            } else {" + $nl + "              this.window?.once('show', reportShown)" + $nl + "            }" + $nl + "          }, 100)" + $nl + "        }")

$patches = @(
  @{ Path = 'src\main\shards\league-client\index.ts'; Import = "import { isPrewarming, onActivated } from '@main/mgs-prewarm'"; Old = '    this._watchConnection()'; New = ("    onActivated(() => {" + $nl + "      void this._watchConnection()" + $nl + "    })") },
  @{ Path = 'src\main\shards\league-client-ux\index.ts'; Import = "import { onActivated } from '@main/mgs-prewarm'"; Old = '    this._watchExistingUx()'; New = '    onActivated(() => this._watchExistingUx())' },
  @{ Path = 'src\main\shards\game-client\index.ts'; Import = "import { onActivated } from '@main/mgs-prewarm'"; Old = '    this._shortcutController.watch()'; New = '    onActivated(() => this._shortcutController.watch())' },
  @{ Path = 'src\main\shards\auto-champ-config\index.ts'; Import = "import { onActivated } from '@main/mgs-prewarm'"; Old = '    this._controller.watch()'; New = '    onActivated(() => this._controller.watch())' },
  @{ Path = 'src\main\shards\respawn-timer\index.ts'; Import = "import { onActivated } from '@main/mgs-prewarm'"; Old = '    this._controller.watch()'; New = '    onActivated(() => this._controller.watch())' },
  @{ Path = 'src\main\shards\sgp\index.ts'; Import = "import { onActivated } from '@main/mgs-prewarm'"; Old = '    this._tokenStateWatcher.watch()'; New = '    onActivated(() => this._tokenStateWatcher.watch())' }
)
foreach ($patch in $patches) {
  $path = Join-Path $SourceRoot $patch.Path
  Add-PrewarmImport $path $patch.Import
  Replace-TextExactlyOnce $path $patch.Old $patch.New
}

$leagueMain = Join-Path $SourceRoot 'src\main\shards\league-client\index.ts'
$protocolAnchor = "    this._protocol.registerDomain('league-client', async (uri, req, context) => {"
Replace-TextExactlyOnce $leagueMain $protocolAnchor ($protocolAnchor + $nl + "      if (isPrewarming()) {" + $nl + "        throw new Error('League client activation is pending')" + $nl + "      }")

$riot = Join-Path $SourceRoot 'src\main\shards\riot-client\index.ts'
Add-PrewarmImport $riot "import { onActivated } from '@main/mgs-prewarm'"
$riotOld = @('    this._mobxUtils.reaction(', '      () => this._leagueClient.state.auth,', '      async (auth) => {', '        if (auth) {', '          this._initHttpInstance(auth)', '        } else {', '          this._httpClient = null', '          this._riotClientApi = null', '        }', '      },', '      { fireImmediately: true }', '    )') -join $nl
$riotNew = @('    onActivated(() => {', '      this._mobxUtils.reaction(', '        () => this._leagueClient.state.auth,', '        async (auth) => {', '          if (auth) {', '            this._initHttpInstance(auth)', '          } else {', '            this._httpClient = null', '            this._riotClientApi = null', '          }', '        },', '        { fireImmediately: true }', '      )', '    })') -join $nl
Replace-TextExactlyOnce $riot $riotOld $riotNew

$riotProtocol = Join-Path $SourceRoot 'src\main\shards\riot-client\protocol-controller.ts'
Add-PrewarmImport $riotProtocol "import { isPrewarming } from '@main/mgs-prewarm'"
$riotProtocolAnchor = "    protocol.registerDomain('riot-client', async (uri, req, context) => {"
Replace-TextExactlyOnce $riotProtocol $riotProtocolAnchor ($riotProtocolAnchor + $nl + "      if (isPrewarming()) {" + $nl + "        throw new Error('Riot client activation is pending')" + $nl + "      }")

$autoGameflow = Join-Path $SourceRoot 'src\main\shards\auto-gameflow\index.ts'
Add-PrewarmImport $autoGameflow "import { onActivated } from '@main/mgs-prewarm'"
$autoGameflowOld = @('    this._honorController.watch()', '    this._lobbyFlow.watch()', '    this._invitations.watch()', '    this._matchmaking.watch()', '    this._aramTeamSide.watch()') -join $nl
$autoGameflowNew = @('    onActivated(() => {', '      this._honorController.watch()', '      this._lobbyFlow.watch()', '      this._invitations.watch()', '      this._matchmaking.watch()', '      this._aramTeamSide.watch()', '    })') -join $nl
Replace-TextExactlyOnce $autoGameflow $autoGameflowOld $autoGameflowNew

$autoMisc = Join-Path $SourceRoot 'src\main\shards\auto-misc\index.ts'
Add-PrewarmImport $autoMisc "import { onActivated } from '@main/mgs-prewarm'"
$autoMiscOld = @('    this._autoReplyController.watch()', '    this._loginAutomationController.watch()') -join $nl
$autoMiscNew = @('    onActivated(() => {', '      this._autoReplyController.watch()', '      this._loginAutomationController.watch()', '    })') -join $nl
Replace-TextExactlyOnce $autoMisc $autoMiscOld $autoMiscNew

$autoSelect = Join-Path $SourceRoot 'src\main\shards\auto-select\index.ts'
Add-PrewarmImport $autoSelect "import { onActivated } from '@main/mgs-prewarm'"
$autoSelectOld = @('    this._localMessage.watch()', '    this._banPick.watch()', '    this._benchController.watch()', '    this._tradeController.watch()') -join $nl
$autoSelectNew = @('    onActivated(() => {', '      this._localMessage.watch()', '      this._banPick.watch()', '      this._benchController.watch()', '      this._tradeController.watch()', '    })') -join $nl
Replace-TextExactlyOnce $autoSelect $autoSelectOld $autoSelectNew

$ongoing = Join-Path $SourceRoot 'src\main\shards\ongoing-game\index.ts'
Add-PrewarmImport $ongoing "import { onActivated } from '@main/mgs-prewarm'"
$ongoingOld = @('    this._champSelectHandoff.watch()', '    this._analysis.watch()', '    this._sideEffects.watch()', '    this._playerData.watch()', '    this._matchHistory.watch()', '    this._watchUnavailableState()', '    this._additionalInfo.watch()') -join $nl
$ongoingNew = @('    onActivated(() => {', '      this._champSelectHandoff.watch()', '      this._analysis.watch()', '      this._sideEffects.watch()', '      this._playerData.watch()', '      this._matchHistory.watch()', '      this._watchUnavailableState()', '      this._additionalInfo.watch()', '    })') -join $nl
Replace-TextExactlyOnce $ongoing $ongoingOld $ongoingNew

$leagueIpc = Join-Path $SourceRoot 'src\main\shards\league-client\ipc-handlers.ts'
Add-PrewarmImport $leagueIpc "import { isPrewarming } from '@main/mgs-prewarm'"
Replace-TextExactlyOnce $leagueIpc "    ipc.onCall(namespace, 'connect', async (_, auth: UxCommandLine & { force?: boolean }) => {" ("    ipc.onCall(namespace, 'connect', async (_, auth: UxCommandLine & { force?: boolean }) => {" + $nl + "      if (isPrewarming()) {" + $nl + "        throw new Error('League client activation is pending')" + $nl + "      }")
$leagueHttpOld = "    ipc.onCall(namespace, 'http-request', async (_, config: AxiosRequestConfig) => {"
Replace-TextExactlyOnce $leagueIpc $leagueHttpOld ($leagueHttpOld + $nl + "      if (isPrewarming()) {" + $nl + "        throw new Error('League client activation is pending')" + $nl + "      }")
$leaguePeek = "    ipc.onCall(namespace, 'peekClient', async (_, auth: UxCommandLine) => {"
Replace-TextExactlyOnce $leagueIpc $leaguePeek ($leaguePeek + $nl + "      if (isPrewarming()) {" + $nl + "        throw new Error('League client activation is pending')" + $nl + "      }")
$leagueSubscribe = "    ipc.onCall(namespace, 'subscribeLcuEndpoint', async (_, uri: string) => {"
Replace-TextExactlyOnce $leagueIpc $leagueSubscribe ($leagueSubscribe + $nl + "      if (isPrewarming()) {" + $nl + "        throw new Error('League client activation is pending')" + $nl + "      }")

$uxIpc = Join-Path $SourceRoot 'src\main\shards\league-client-ux\ipc-handlers.ts'
Add-PrewarmImport $uxIpc "import { isPrewarming } from '@main/mgs-prewarm'"
Replace-TextExactlyOnce $uxIpc "    ipc.onCall(namespace, 'update', () => this.leagueClientUx.update())" "    ipc.onCall(namespace, 'update', () => {`n      if (isPrewarming()) throw new Error('League client activation is pending')`n      return this.leagueClientUx.update()`n    })".Replace('`n', $nl)

$keyboard = Join-Path $SourceRoot 'src\main\shards\keyboard-shortcuts\index.ts'
Add-PrewarmImport $keyboard "import { isPrewarming } from '@main/mgs-prewarm'"
$keyboardOld = @(
  '      this._nativeKeyEventHandler = (key) => {',
  '        this._processNativeKeyEvent(key)',
  '      }'
) -join $nl
$keyboardNew = @(
  '      this._nativeKeyEventHandler = (key) => {',
  '        if (isPrewarming()) return',
  '        this._processNativeKeyEvent(key)',
  '      }'
) -join $nl
Replace-TextExactlyOnce $keyboard $keyboardOld $keyboardNew

# Refuse a partial patch: every connection/automation entry point must carry
# the gate before this staged source can be built.
$required = @(
  'src\main\mgs-prewarm.ts',
  'src\main\shards\league-client\index.ts',
  'src\main\shards\league-client-ux\index.ts',
  'src\main\shards\riot-client\index.ts',
  'src\main\shards\riot-client\protocol-controller.ts',
  'src\main\shards\game-client\index.ts',
  'src\main\shards\auto-champ-config\index.ts',
  'src\main\shards\auto-gameflow\index.ts',
  'src\main\shards\auto-misc\index.ts',
  'src\main\shards\auto-select\index.ts',
  'src\main\shards\ongoing-game\index.ts',
  'src\main\shards\respawn-timer\index.ts',
  'src\main\shards\sgp\index.ts',
  'src\main\shards\keyboard-shortcuts\index.ts'
)
foreach ($relative in $required) {
  if (-not (Test-Path -LiteralPath (Join-Path $SourceRoot $relative) -PathType Leaf)) {
    throw "Prewarm patch verification target is missing: $relative"
  }
}
Write-Host "League prewarm gate staged in $SourceRoot"
