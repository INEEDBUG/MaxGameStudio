# Remove both overlays from the standalone build, including old shortcuts and startup observers.
function Remove-StandaloneWindowBlock([string]$RelativePath, [string]$Pattern) {
  $source = [IO.File]::ReadAllText((Join-Path $SourceRoot $RelativePath))
  $matches = [regex]::Matches($source, $Pattern)
  if ($matches.Count -ne 1) { throw "Expected one retired window block in $RelativePath ($Pattern)." }
  Update-StandaloneSource $RelativePath $matches[0].Value ''
}

$manager = 'src\main\shards\window-manager\index.ts'
$lifecycle = 'src\main\shards\window-manager\lifecycle-controller.ts'
$trayMenu = 'src\main\shards\tray\tray-menu-controller.ts'
$trayWatcher = 'src\main\shards\tray\tray-state-watcher.ts'
$rendererLinks = 'src\main\shards\app-common\renderer-link-protocol.ts'
$rendererManager = 'src\renderer-shared\shards\window-manager\index.ts'
$windowSettings = Join-Path $renderer 'components\settings-modal\MultiWindowSettings.vue'

foreach ($window in @(
  @{ key = 'ongoingGameWindow'; type = 'AkariOngoingGameWindow'; import = './ongoing-game-window/window'; view = 'ongoing-game-window'; watch = '_watchOngoingGameWindow'; store = 'useOngoingGameWindowStore'; settings = 'ogws' },
  @{ key = 'cdTimerWindow'; type = 'AkariCdTimerWindow'; import = './cd-timer-window/windows'; view = 'cd-timer-window'; watch = '_watchCdTimerWindow'; store = 'useCdTimerWindowStore'; settings = 'ctws' }
)) {
  $key = $window.key
  $type = $window.type
  $view = $window.view
  $watch = $window.watch
  $item = "$($key)DevTrayItem"
  Update-StandaloneSource $manager "import { $type } from '$($window.import)'" ''
  Update-StandaloneSource $manager "  public readonly ${key}: $type" ''
  Update-StandaloneSource $manager "    this.$key = new $type(this._context)" ''
  Remove-StandaloneWindowBlock $manager "(?m)^      ${key}: this\.$key,?\r?\n"

  Update-StandaloneSource $lifecycle "import type { $type } from '$($window.import)'" ''
  Update-StandaloneSource $lifecycle "  ${key}: $type" ''
  Update-StandaloneSource $lifecycle "    await this._windows.$key.onInit()" ''
  Update-StandaloneSource $trayWatcher "    this.$watch()" ''
  Remove-StandaloneWindowBlock $trayWatcher "(?ms)^  private $watch\(\) \{.*?^  \}\r?\n"
  Update-StandaloneSource $trayMenu "  $item!: MenuItem" ''
  Update-StandaloneSource $trayMenu "          this.$item," ''
  Update-StandaloneSource $trayMenu "        windowManager.$key.repositionWindowIfInvisible()" ''
  Remove-StandaloneWindowBlock $trayMenu "(?ms)^    this\.$item = new MenuItem\(\{.*?^    \}\)\r?\n"
  Remove-StandaloneWindowBlock $rendererLinks "(?m)^      case '$view':\r?\n.*?windowManager\.$key\.window\?.*\r?\n        break\r?\n"

  Update-StandaloneSource $rendererManager "  public ${key}: $type" ''
  Update-StandaloneSource $rendererManager "    this.$key = new $type(this.context)" ''
  Update-StandaloneSource $rendererManager "    await this.$key.onInit()" ''

  Remove-StandaloneWindowBlock $windowSettings "(?ms)^      <SettingsSection\r?\n        :title=\x22\r?\n          as\.isElevated\r?\n            \? t\('settings\.multiWindow\.$key\.title'\).*?^      </SettingsSection>\r?\n"
  Update-StandaloneSource $windowSettings "  $($window.store)," ''
  Update-StandaloneSource $windowSettings "const $($window.settings) = $($window.store)()" ''

  Remove-StandaloneWindowBlock 'electron.vite.config.ts' "(?m)^          ${key}: resolve\(__dirname, 'src/renderer/$view\.html'\),?\r?\n"
  Update-StandaloneSource 'tsconfig.web.json' ('    "src/renderer/src-{0}/**/*",' -f $view) ''
  Update-StandaloneSource 'tsconfig.web.json' ('    "src/renderer/src-{0}/**/*.vue",' -f $view) ''
}

Update-StandaloneSource $rendererManager "  AkariAuxWindow,`n  AkariCdTimerWindow,`n  AkariMainWindow,`n  AkariOngoingGameWindow,`n  AkariOpggWindow" "  AkariAuxWindow,`n  AkariMainWindow,`n  AkariOpggWindow"
Update-StandaloneSource $rendererManager "export {`n  AkariCdTimerWindow,`n  AkariOngoingGameWindow,`n" "export {`n"
Update-StandaloneSource $windowSettings '  AkariCdTimerWindow,' ''
Update-StandaloneSource $windowSettings "import { AkariOngoingGameWindow } from '@renderer-shared/shards/window-manager'" ''
Update-StandaloneSource $windowSettings 'NIcon, NRadio, NRadioGroup, NScrollbar' 'NIcon, NScrollbar'
