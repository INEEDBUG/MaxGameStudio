#Requires -Version 5.1
<##
.SYNOPSIS
  Prepare the approved League client toolkit source as an isolated Tauri resource.

.DESCRIPTION
  This script is intentionally a staging step, not an installer and not an
  updater. It pins the upstream tree to one immutable commit, copies only the
  source/build inputs needed by the integration, applies the MaxGameStudio
  display name in user-visible metadata for source builds, preserves the
  upstream updater files for license/audit completeness, and writes the
  required MIT notice and machine-readable hash manifest.

  The default operation does not install dependencies or run a package manager.
  Use -Build only when the source directory already has its locked dependencies.
  The staged directory is generated under frontend/.runtime-cache and is
  ignored by Git. The resource staging script copies it into the Tauri bundle
  only after the ordinary resources have been rebuilt.
##>
param(
  [string]$SourceDir = $env:MAXGAMESTUDIO_LEAGUE_SOURCE_DIR,
  [string]$RuntimeDir = $env:MAXGAMESTUDIO_LEAGUE_RUNTIME_DIR,
  [string]$OutputDir = "",
  [switch]$FetchPinnedSource,
  [switch]$Build
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repository = "https://github.com/LeagueAkari/LeagueAkari.git"
$PinnedCommit = "14557723706ccc0e0a9d62c470141d4cb7190fcd"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$buildTempRoot = if ($env:MAXGAMESTUDIO_BUILD_TEMP) {
  [IO.Path]::GetFullPath($env:MAXGAMESTUDIO_BUILD_TEMP)
} else {
  [IO.Path]::GetTempPath()
}
New-Item -ItemType Directory -Path $buildTempRoot -Force | Out-Null
if (-not $OutputDir) {
  $OutputDir = Join-Path $repoRoot "frontend\.runtime-cache\league-runtime"
}

function Invoke-Git([string[]]$Arguments, [string]$WorkingDirectory) {
  $result = & git -C $WorkingDirectory @Arguments 2>&1
  if ($LASTEXITCODE -ne 0) {
    throw "git $($Arguments -join ' ') failed in $WorkingDirectory`n$result"
  }
  return ($result | Out-String).Trim()
}

function Get-RelativePathCompat([string]$BasePath, [string]$ChildPath) {
  # [IO.Path]::GetRelativePath was added after the .NET Framework shipped
  # with Windows PowerShell 5.1. Use file:// URIs so this script remains
  # runnable under the declared PS5.1 contract.
  $base = [IO.Path]::GetFullPath($BasePath)
  if (-not $base.EndsWith([IO.Path]::DirectorySeparatorChar.ToString())) {
    $base += [IO.Path]::DirectorySeparatorChar
  }
  $child = [IO.Path]::GetFullPath($ChildPath)
  $baseUri = [Uri]::new($base)
  $childUri = [Uri]::new($child)
  if ($baseUri.Scheme -ne $childUri.Scheme) {
    throw "Cannot compute a relative path across URI schemes: $BasePath -> $ChildPath"
  }
  [Uri]::UnescapeDataString($baseUri.MakeRelativeUri($childUri).ToString()).Replace('/', [IO.Path]::DirectorySeparatorChar)
}

function Replace-TextExactlyOnce([string]$PathValue, [string]$OldValue, [string]$NewValue) {
  if (-not (Test-Path -LiteralPath $PathValue -PathType Leaf)) {
    throw "Pinned source patch target is missing: $PathValue"
  }
  $text = Get-Content -LiteralPath $PathValue -Raw -Encoding utf8
  $hits = ([regex]::Matches($text, [regex]::Escape($OldValue))).Count
  if ($hits -ne 1) {
    throw "Expected exactly one pinned source patch match in $PathValue (found $hits)."
  }
  $text = $text.Replace($OldValue, $NewValue)
  [IO.File]::WriteAllText($PathValue, $text, [Text.UTF8Encoding]::new($false))
}

function Apply-MaxGameStudioTheme([string]$SourceRoot, [string]$LeagueWorkbenchIconPath) {
  # The live pinned renderer routes Naive UI through naive-ui.ts. The split
  # naive-ui-overrides directory is an inactive experiment, so patch only the
  # active light/dark common overrides and leave status/alternate theme tokens
  # unchanged.
  $classicTheme = Join-Path $SourceRoot "src\renderer-shared\theme\naive-ui.ts"
  if (-not (Test-Path -LiteralPath $classicTheme -PathType Leaf)) { throw "Active Naive UI theme module is missing: $classicTheme" }
  $lineBreak = [Environment]::NewLine
  $lightThemeTokens = "      modalColor: '#f5f5f6'," + $lineBreak +
    "      primaryColor: '#2563eb'," + $lineBreak +
    "      primaryColorHover: '#1d4ed8'," + $lineBreak +
    "      primaryColorPressed: '#1e40af'," + $lineBreak +
    "      primaryColorSuppl: 'rgba(37, 99, 235, 0.14)'"
  $darkThemeTokens = "      modalColor: '#1e1e22'," + $lineBreak +
    "      primaryColor: '#f59e0b'," + $lineBreak +
    "      primaryColorHover: '#fbbf24'," + $lineBreak +
    "      primaryColorPressed: '#d97706'," + $lineBreak +
    "      primaryColorSuppl: 'rgba(245, 158, 11, 0.2)'"
  Replace-TextExactlyOnce $classicTheme "      modalColor: '#f5f5f6'" $lightThemeTokens
  Replace-TextExactlyOnce $classicTheme "      modalColor: '#1e1e22'" $darkThemeTokens

  # The embedded workspace follows MaxGameStudio's two product themes. Keep
  # upstream theme IDs readable for config compatibility, but do not expose
  # alternate palettes that would make this bundled surface look like a
  # separate product.
  $appSettings = Join-Path $SourceRoot "src\renderer\src-main-window\components\settings-modal\AppSettings.vue"
  $titlebarButtons = Join-Path $SourceRoot "src\renderer\src-main-window\components\titlebar\CommonButtons.vue"
  foreach ($themePicker in @($appSettings, $titlebarButtons)) {
    if (-not (Test-Path -LiteralPath $themePicker -PathType Leaf)) { throw "Theme picker is missing: $themePicker" }
    Replace-TextExactlyOnce $themePicker "BUILTIN_LIGHT_THEME_IDS.map" "BUILTIN_LIGHT_THEME_IDS.filter((id) => id === 'light').map"
    Replace-TextExactlyOnce $themePicker "BUILTIN_DARK_THEME_IDS.map" "BUILTIN_DARK_THEME_IDS.filter((id) => id === 'dark').map"
  }

  $themeCss = Join-Path $SourceRoot "src\renderer-shared\assets\css\theme-system.css"
  if (-not (Test-Path -LiteralPath $themeCss -PathType Leaf)) { throw "Theme stylesheet is missing: $themeCss" }
  $css = Get-Content -LiteralPath $themeCss -Raw -Encoding utf8
  $marker = "/* MaxGameStudio theme bridge: do not remove */"
  if (-not $css.Contains($marker)) {
    $css += @"

$marker
:root[data-theme='dark'] {
  --la-color-bg-primary: #111111;
  --la-color-text-primary: rgba(255, 255, 255, 0.92);
  --la-color-text-themed: rgba(255, 255, 255, 0.92);
  --la-color-link: #fbbf24;
  --la-color-scrollbar-thumb: #4b5563;
  --la-color-scrollbar-thumb-hover: #6b7280;
  --la-color-select-menu-bg: #1f1f23;
  --la-color-message-bg: #242424;
  --la-color-popconfirm-bg: #1c1c1f;
  --la-color-popover-border: rgba(245, 158, 11, 0.38);
  --la-sidebar-bg: rgba(0, 0, 0, 0.38);
  --la-sidebar-border: rgba(245, 158, 11, 0.18);
}
:root[data-theme='light'] {
  --la-color-bg-primary: #f8fafc;
  --la-color-text-primary: #172033;
  --la-color-text-themed: #172033;
  --la-color-link: #2563eb;
  --la-color-scrollbar-thumb: #94a3b8;
  --la-color-scrollbar-thumb-hover: #64748b;
  --la-color-select-menu-bg: #ffffff;
  --la-color-message-bg: #f1f5f9;
  --la-color-popconfirm-bg: #ffffff;
  --la-color-popover-border: rgba(37, 99, 235, 0.3);
  --la-sidebar-bg: rgba(226, 232, 240, 0.7);
  --la-sidebar-border: rgba(37, 99, 235, 0.18);
}
:root[data-theme='dark'] {
  --mgs-accent: #f59e0b;
  --mgs-accent-hover: #fbbf24;
  --mgs-accent-pressed: #d97706;
  --mgs-accent-shadow: rgba(245, 158, 11, 0.3);
}
 :root[data-theme='light'] {
   --mgs-accent: #2563eb;
   --mgs-accent-hover: #1d4ed8;
   --mgs-accent-pressed: #1e40af;
   --mgs-accent-shadow: rgba(37, 99, 235, 0.24);
 }
 :root[data-theme='dark'] .beautiful-akari {
   background-image: linear-gradient(90deg, #fbbf24, #f59e0b 55%, #fff7ed 100%);
 }
 :root[data-theme='light'] .beautiful-akari {
   background-image: linear-gradient(90deg, #1d4ed8, #2563eb 55%, #dbeafe 100%);
 }
 :root[data-theme='dark'] .app-sidebar__logo-icon,
 :root[data-theme='light'] .app-sidebar__logo-icon,
 :root[data-theme='dark'] .startup-logo,
 :root[data-theme='light'] .startup-logo {
   color: var(--mgs-accent);
   filter: drop-shadow(0 0 8px var(--mgs-accent-shadow));
 }
:root[data-theme='dark'] .sidebar-menu:not(.rabi-test) .indicator-rail::before,
:root[data-theme='dark'] .sidebar-menu:not(.rabi-test) .indicator-rail::after {
   background-color: var(--mgs-accent);
 }
 :root[data-theme='light'] .sidebar-menu:not(.rabi-test) .indicator-rail::before,
 :root[data-theme='light'] .sidebar-menu:not(.rabi-test) .indicator-rail::after {
   background-color: var(--mgs-accent);
 }
 /* Keep interaction affordances on the product accent; semantic success,
    warning, and error classes are intentionally not overridden. */
:root[data-theme='dark'] .n-menu-item-content--selected::before,
:root[data-theme='light'] .n-menu-item-content--selected::before,
:root[data-theme='dark'] .n-tabs-bar,
:root[data-theme='light'] .n-tabs-bar { background-color: var(--mgs-accent); }
:root[data-theme='dark'] .n-switch.n-switch--active .n-switch__rail,
:root[data-theme='light'] .n-switch.n-switch--active .n-switch__rail,
:root[data-theme='dark'] .n-radio-button.n-radio-button--checked,
:root[data-theme='light'] .n-radio-button.n-radio-button--checked { background-color: var(--mgs-accent); border-color: var(--mgs-accent); }
:root[data-theme='dark'] .n-menu-item-content--selected,
:root[data-theme='light'] .n-menu-item-content--selected,
:root[data-theme='dark'] .n-tabs-tab--active,
:root[data-theme='light'] .n-tabs-tab--active { color: var(--mgs-accent); }
"@
    [IO.File]::WriteAllText($themeCss, $css, [Text.UTF8Encoding]::new($false))
  }
  $patchedThemeCss = Get-Content -LiteralPath $themeCss -Raw -Encoding utf8
  if (-not $patchedThemeCss.Contains($marker)) {
    throw "MaxGameStudio theme bridge was not written: $themeCss"
  }
  if ($patchedThemeCss.Contains('.n-button--primary-type')) {
    throw "Theme bridge must not force primary button backgrounds; Naive UI owns secondary button contrast."
  }
  if ($patchedThemeCss.Contains('.app-sidebar__expand-line:hover')) {
    throw "Theme bridge must not create a full-height sidebar resize hover line."
  }
  foreach ($requiredThemeToken in @('#f59e0b', '#2563eb', 'var(--mgs-accent)')) {
    if (-not $patchedThemeCss.Contains($requiredThemeToken)) {
      throw "MaxGameStudio theme bridge is missing required accent token $requiredThemeToken."
    }
  }
  foreach ($semanticSelector in @('.n-button--success-type', '.n-button--warning-type', '.n-button--error-type')) {
    if ($patchedThemeCss.Contains($semanticSelector)) {
      throw "Theme bridge must not override semantic button selector $semanticSelector."
    }
  }

  $runtimeIcon = Join-Path $SourceRoot "resources\LA_ICON.ico"
  if (-not (Test-Path -LiteralPath $LeagueWorkbenchIconPath -PathType Leaf)) {
    throw "Dedicated League workbench icon is missing: $LeagueWorkbenchIconPath"
  }
  Copy-Item -LiteralPath $LeagueWorkbenchIconPath -Destination $runtimeIcon -Force
  $sourceIconHash = (Get-FileHash -LiteralPath $LeagueWorkbenchIconPath -Algorithm SHA256).Hash
  $runtimeIconHash = (Get-FileHash -LiteralPath $runtimeIcon -Algorithm SHA256).Hash
  if ($sourceIconHash -ne $runtimeIconHash) {
    throw "Dedicated League workbench icon was not copied byte-for-byte."
  }

  # Renderer logo imports use PNG/SVG assets, while Windows packaging uses ICO.
  # Do not derive the renderer asset through Icon.ToBitmap(): Windows
  # PowerShell 5.1/.NET Framework can treat PNG-backed ICO frames as legacy
  # DIB data and emit a noisy bitmap. Reuse the canonical host PNG directly;
  # the verifier below guards its format, dimensions, alpha samples, content,
  # and renderer references.
  $rendererLogoDir = Join-Path $SourceRoot "src\renderer-shared\assets\logo"
  $rendererLogoPng = Join-Path $rendererLogoDir "maxgamestudio-logo.png"
  New-Item -ItemType Directory -Path $rendererLogoDir -Force | Out-Null
  $hostLogoPath = Join-Path $repoRoot "frontend\public\cs2-ultimate-insight-logo.png"
  if (-not (Test-Path -LiteralPath $hostLogoPath -PathType Leaf)) {
    throw "Canonical MaxGameStudio renderer logo is missing: $hostLogoPath"
  }
  Copy-Item -LiteralPath $hostLogoPath -Destination $rendererLogoPng -Force
  $logoComponentPath = Join-Path $SourceRoot "src\renderer-shared\assets\icon\AkariLogo.vue"
  if (-not (Test-Path -LiteralPath $logoComponentPath -PathType Leaf)) {
    throw "Pinned renderer logo component AkariLogo.vue is missing: $logoComponentPath"
  }
  $logoText = Get-Content -LiteralPath $logoComponentPath -Raw -Encoding utf8
  $templateMatches = [regex]::Matches($logoText, '(?s)<template>.*</template>')
  if ($templateMatches.Count -ne 1 -or $logoText.Trim() -notmatch '(?s)^<template>.*</template>$') {
    throw "Expected the pinned AkariLogo.vue to contain one standalone template block."
  }
  # v1.5.1 embeds the upstream logo as inline SVG; it has no logo.svg import.
  # Keep one deterministic reference to the generated MaxGameStudio asset while
  # retaining the SVG contract expected by Naive UI's NIcon wrapper.
  $logoTemplate = @(
    '<template>'
    '  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" role="img" aria-label="MaxGameStudio">'
    '    <image href="../logo/maxgamestudio-logo.png" width="64" height="64" preserveAspectRatio="xMidYMid meet" />'
    '  </svg>'
    '</template>'
    ''
  ) -join [Environment]::NewLine
  [IO.File]::WriteAllText($logoComponentPath, $logoTemplate, [Text.UTF8Encoding]::new($false))

  $aboutLogoPath = Join-Path $SourceRoot "src\renderer\src-main-window\components\settings-modal\AboutPane.vue"
  Replace-TextExactlyOnce $aboutLogoPath 'src="@renderer-shared/assets/logo/logo-hollow.svg"' 'src="@renderer-shared/assets/logo/maxgamestudio-logo.png"'

  $assetVerifier = Join-Path $repoRoot "packaging\windows\verify-league-runtime-assets.ps1"
  if (-not (Test-Path -LiteralPath $assetVerifier -PathType Leaf)) {
    throw "League runtime asset verifier is missing: $assetVerifier"
  }
  & $assetVerifier -Root $SourceRoot -Mode Source
}

function Apply-EmbeddedUiPruning([string]$SourceRoot) {
  # Remove only the user-facing Game Send entry point and its two runtime
  # registrations. Keep schemas and migrations intact so old settings remain
  # readable and can still be migrated by the host/runtime compatibility path.
  $toolkitPath = Join-Path $SourceRoot "src\renderer\src-main-window\views\toolkit\Toolkit.vue"
  $rendererShardsPath = Join-Path $SourceRoot "src\renderer\src-main-window\shards\index.ts"
  $mainBootstrapPath = Join-Path $SourceRoot "src\main\bootstrap\index.ts"
  foreach ($path in @($toolkitPath, $rendererShardsPath, $mainBootstrapPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
      throw "Game Send pruning target is missing: $path"
    }
  }

  Replace-TextExactlyOnce $toolkitPath "import InGameSend from './in-game-send/InGameSend.vue'" ""
  Replace-TextExactlyOnce $toolkitPath "import { Box, Chat, UserMultiple } from '@vicons/carbon'" "import { Box, UserMultiple } from '@vicons/carbon'"
  $toolkitTabBlock = @(
    "  {",
    "    key: 'in-game-send',",
    "    name: t('toolkit.home.in-game-send'),",
    "    icon: Chat,",
    "    component: InGameSend",
    "  },"
  ) -join [Environment]::NewLine
  Replace-TextExactlyOnce $toolkitPath $toolkitTabBlock ""
  $toolkitText = Get-Content -LiteralPath $toolkitPath -Raw -Encoding utf8
  if ($toolkitText.Contains('InGameSend') -or $toolkitText.Contains('in-game-send') -or $toolkitText.Contains('Chat')) {
    throw "Game Send Toolkit import/tab was not removed completely: $toolkitPath"
  }

  Replace-TextExactlyOnce $rendererShardsPath "import { InGameSendRenderer } from '@renderer-shared/shards/in-game-send'" ""
  Replace-TextExactlyOnce $rendererShardsPath "manager.use(InGameSendRenderer)" ""
  $rendererShardsText = Get-Content -LiteralPath $rendererShardsPath -Raw -Encoding utf8
  if ($rendererShardsText.Contains('InGameSendRenderer')) {
    throw "Game Send renderer shard registration was not removed completely: $rendererShardsPath"
  }

  Replace-TextExactlyOnce $mainBootstrapPath "import { InGameSendMain } from '@main/shards/in-game-send'" ""
  Replace-TextExactlyOnce $mainBootstrapPath "    manager.use(InGameSendMain)" ""
  $mainBootstrapText = Get-Content -LiteralPath $mainBootstrapPath -Raw -Encoding utf8
  if ($mainBootstrapText.Contains('InGameSendMain')) {
    throw "Game Send main shard registration was not removed completely: $mainBootstrapPath"
  }

  $compatibilityPaths = @(
    'src\shared\shards\in-game-send\index.ts',
    'src\main\shards\in-game-send\setting-schemas.ts',
    'src\main\shards\config-migrate\migrations\from-1-3-5.ts',
    'src\main\shards\config-migrate\migrations\from-1-4-3.ts'
  )
  foreach ($relativePath in $compatibilityPaths) {
    $path = Join-Path $SourceRoot $relativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
      throw "Game Send compatibility/migration file was removed unexpectedly: $path"
    }
  }
  $migrationText = Get-Content -LiteralPath (Join-Path $SourceRoot 'src\main\shards\config-migrate\migrations\from-1-4-3.ts') -Raw -Encoding utf8
  if (-not $migrationText.Contains('in-game-send-main') -or -not $migrationText.Contains('resetInGameSendSettings')) {
    throw "Game Send migration contract is missing from the staged source."
  }

  # Keep the resize hit area and pointer affordance, but remove only the
  # upstream full-height red hover line from the sidebar resize handle.
  $sidebarPath = Join-Path $SourceRoot "src\renderer\src-main-window\components\sidebar\Sidebar.vue"
  if (-not (Test-Path -LiteralPath $sidebarPath -PathType Leaf)) {
    throw "Sidebar resize target is missing: $sidebarPath"
  }
  $resizeHoverBlock = @(
    "    &:hover .app-sidebar__expand-line-inner {",
    "      background-color: #f83f6f;",
    "    }"
  ) -join [Environment]::NewLine
  Replace-TextExactlyOnce $sidebarPath $resizeHoverBlock ""
  $sidebarText = Get-Content -LiteralPath $sidebarPath -Raw -Encoding utf8
  foreach ($resizeContract in @('cursor: ew-resize;', 'padding: 0 4px;', '@mousedown="toggleCollapse"', '.app-sidebar__expand-line-inner')) {
    if (-not $sidebarText.Contains($resizeContract)) {
      throw "Sidebar resize contract was damaged while removing hover highlight: $resizeContract"
    }
  }
  if ($sidebarText.Contains('#f83f6f')) {
    throw "Sidebar resize handle still contains the upstream red hover highlight: $sidebarPath"
  }

  # The host passes these switches only for the embedded runtime. This timer
  # is intentionally independent of IPC and is never installed for ordinary
  # upstream launches; the existing MAXGAMESTUDIO_EMBEDDED env guards remain
  # the compatibility mechanism for other embedded-only behavior.
  $watchdogAnchor = "  const events = new EventEmitter<AkariAppEventMap>()"
  $watchdogBlock = @(
    "  const embeddedLaunchRequested = process.argv.includes('--maxgamestudio-embedded')",
    "  if (embeddedLaunchRequested) {",
    "    process.env.MAXGAMESTUDIO_EMBEDDED = '1'",
    "  }",
    "  const embeddedHostPidArgument = process.argv.find(",
    "    (argument) => typeof argument === 'string' && argument.startsWith('--maxgamestudio-host-pid=')",
    "  )",
    "  const embeddedHostPid = embeddedHostPidArgument",
    "    ? Number(embeddedHostPidArgument.slice('--maxgamestudio-host-pid='.length))",
    "    : Number.NaN",
    "  const embeddedShutdownSignalArgument = process.argv.find(",
    "    (argument) => typeof argument === 'string' && argument.startsWith('--maxgamestudio-shutdown-signal=')",
    "  )",
    "  const embeddedShutdownSignalPath = embeddedShutdownSignalArgument",
    "    ? embeddedShutdownSignalArgument.slice('--maxgamestudio-shutdown-signal='.length)",
    "    : ''",
    "  const embeddedShutdownSignal = path.isAbsolute(embeddedShutdownSignalPath)",
    "    ? embeddedShutdownSignalPath",
    "    : null",
    "  const embeddedHostWatchdogEnabled =",
    "    process.platform === 'win32' &&",
    "    embeddedLaunchRequested &&",
    "    Number.isInteger(embeddedHostPid) &&",
    "    embeddedHostPid > 0",
    "  if (embeddedHostWatchdogEnabled) {",
    "    const embeddedHostWatchdog = setInterval(() => {",
    "      if (embeddedShutdownSignal && fs.existsSync(embeddedShutdownSignal)) {",
    "        try {",
    "          fs.unlinkSync(embeddedShutdownSignal)",
    "        } catch {}",
    "        clearInterval(embeddedHostWatchdog)",
    "        app.quit()",
    "        return",
    "      }",
    "      try {",
    "        process.kill(embeddedHostPid, 0)",
    "      } catch (error) {",
    "        if ((error as NodeJS.ErrnoException).code === 'ESRCH') {",
    "          clearInterval(embeddedHostWatchdog)",
    "          app.quit()",
    "        }",
    "      }",
    "    }, 1000)",
    "    embeddedHostWatchdog.unref()",
    "  }"
  ) -join [Environment]::NewLine
  Replace-TextExactlyOnce $mainBootstrapPath $watchdogAnchor ($watchdogAnchor + [Environment]::NewLine + $watchdogBlock)
  $watchdogText = Get-Content -LiteralPath $mainBootstrapPath -Raw -Encoding utf8
  foreach ($watchdogContract in @(
    "--maxgamestudio-embedded",
    "process.env.MAXGAMESTUDIO_EMBEDDED = '1'",
    "--maxgamestudio-host-pid=",
    "--maxgamestudio-shutdown-signal=",
    "process.platform === 'win32'",
    "path.isAbsolute(embeddedShutdownSignalPath)",
    "fs.existsSync(embeddedShutdownSignal)",
    "fs.unlinkSync(embeddedShutdownSignal)",
    "process.kill(embeddedHostPid, 0)",
    "setInterval",
    "}, 1000)",
    "embeddedHostWatchdog.unref()",
    "app.quit()"
  )) {
    if (-not $watchdogText.Contains($watchdogContract)) {
      throw "Embedded host-PID watchdog patch is incomplete: $watchdogContract"
    }
  }
  if ($watchdogText.IndexOf("process.env.MAXGAMESTUDIO_EMBEDDED = '1'") -gt $watchdogText.IndexOf('const embeddedHostWatchdogEnabled')) {
    throw "Embedded env compatibility guard is not initialized before the watchdog condition."
  }
  if (-not $watchdogText.Contains('embeddedLaunchRequested &&')) {
    throw "Embedded host-PID watchdog argument guard is not present in bootstrap."
  }

  Write-Host "Embedded UI pruning: PASS (Game Send registrations removed; compatibility retained; resize hit area retained; host watchdog guarded)"
}

function Restore-UnpackedNativeHeaders([string]$SourceRoot, [string]$OutputRoot) {
  # electron-builder's pinned afterPack trim removes the native package's
  # source directory after asar metadata has recorded these two files as
  # unpacked. Restore only the exact metadata entries so archive extraction
  # and downstream statFile(...).unpacked checks remain consistent.
  $sourcePackage = Join-Path $SourceRoot "node_modules\league-akari-native-win32"
  $outputPackage = Join-Path $OutputRoot "resources\app.asar.unpacked\node_modules\league-akari-native-win32"
  foreach ($relative in @("src\input\input.h", "src\tools\tools.h")) {
    $sourcePath = Join-Path $sourcePackage $relative
    $outputPath = Join-Path $outputPackage $relative
    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
      throw "Pinned native unpacked source is missing: $sourcePath"
    }
    New-Item -ItemType Directory -Path (Split-Path -Parent $outputPath) -Force | Out-Null
    Copy-Item -LiteralPath $sourcePath -Destination $outputPath -Force
    $sourceHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $outputHash = (Get-FileHash -LiteralPath $outputPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($sourceHash -ne $outputHash) {
      throw "Failed to restore native unpacked file byte-for-byte: $relative"
    }
  }
}

$tempSource = $null
$tempBuildSource = $null
try {
  if ($FetchPinnedSource) {
    $tempSource = Join-Path $buildTempRoot ("maxgamestudio-league-" + [Guid]::NewGuid().ToString("n"))
    New-Item -ItemType Directory -Path $tempSource -Force | Out-Null
    Invoke-Git @("init", "--quiet") $tempSource | Out-Null
    Invoke-Git @("remote", "add", "origin", $Repository) $tempSource | Out-Null
    Invoke-Git @("fetch", "--quiet", "--depth", "1", "origin", $PinnedCommit) $tempSource | Out-Null
    Invoke-Git @("checkout", "--quiet", "--detach", "FETCH_HEAD") $tempSource | Out-Null
    $SourceDir = $tempSource
  }

  if (-not $RuntimeDir -and -not $SourceDir) {
    throw "A runnable RuntimeDir or pinned SourceDir is required. Pass -RuntimeDir, -FetchPinnedSource, or MAXGAMESTUDIO_LEAGUE_RUNTIME_DIR."
  }

  if (Test-Path -LiteralPath $OutputDir) {
    Remove-Item -LiteralPath $OutputDir -Recurse -Force
  }
  $source = $null
  $runtimeSource = $null
  $sourceMode = $null
  if ($RuntimeDir) {
    $runtimeSource = (Resolve-Path -LiteralPath $RuntimeDir).Path
    $runtimeExe = Get-ChildItem -LiteralPath $runtimeSource -File -Filter "LeagueAkari.exe" | Select-Object -First 1
    if (-not $runtimeExe) { throw "RuntimeDir must contain LeagueAkari.exe at its root." }
    if (-not (Test-Path -LiteralPath (Join-Path $runtimeSource "resources\app.asar"))) {
      throw "RuntimeDir is not an unpacked Electron directory: resources\app.asar is missing."
    }
    if (-not (Test-Path -LiteralPath (Join-Path $runtimeSource "resources\app.asar.unpacked"))) {
      throw "RuntimeDir is incomplete: resources\app.asar.unpacked is missing."
    }
    $runtimeVersion = (Get-Item -LiteralPath $runtimeExe.FullName).VersionInfo.FileVersion
    if ($runtimeVersion -ne "1.5.1") {
      throw "RuntimeDir must be the approved v1.5.1 runtime (got $runtimeVersion)."
    }
    Copy-Item -LiteralPath $runtimeSource -Destination $OutputDir -Recurse -Force
    $sourceMode = "runnable-runtime-input"
  } else {
    $source = (Resolve-Path -LiteralPath $SourceDir).Path
    $head = Invoke-Git @("rev-parse", "HEAD") $source
    if ($head -ne $PinnedCommit) {
      throw "League source is not pinned to approved commit $PinnedCommit (got $head)."
    }
    if (-not $Build) { throw "A source tree is not runnable. Pass -Build with preinstalled locked dependencies." }
    # Never patch a user checkout in place. Build from an isolated copy; this
    # also makes the generated unpacked runtime independent of the worktree.
    $tempBuildSource = Join-Path $buildTempRoot ("maxgamestudio-league-build-" + [Guid]::NewGuid().ToString("n"))
    New-Item -ItemType Directory -Path $tempBuildSource -Force | Out-Null
    Get-ChildItem -LiteralPath $source -Force | Where-Object {
      $_.Name -notin @('.git', 'node_modules', 'out', 'dist', 'coverage')
    } | ForEach-Object {
      Copy-Item -LiteralPath $_.FullName -Destination $tempBuildSource -Recurse -Force
    }
    $sourceNodeModules = Join-Path $source "node_modules"
    if (-not (Test-Path -LiteralPath $sourceNodeModules)) {
      throw "-Build requires preinstalled dependencies in SourceDir; refusing implicit install."
    }
    $tempBuildNodeModules = Join-Path $tempBuildSource "node_modules"
    $robocopy = Get-Command robocopy.exe -ErrorAction SilentlyContinue
    if (-not $robocopy) { throw "-Build requires the Windows robocopy.exe system tool." }
    & $robocopy.Source $sourceNodeModules $tempBuildNodeModules /E /XJ /MT:16 /R:0 /W:0 /NFL /NDL /NJH /NJS /NC /NS | Out-Null
    if ($LASTEXITCODE -gt 7) {
      throw "Failed to copy locked dependencies into the isolated build directory (robocopy exit $LASTEXITCODE)."
    }
    $workspacePackage = Join-Path $source "native\win32-x64"
    if (-not (Test-Path -LiteralPath $workspacePackage)) {
      throw "Pinned source is missing native\win32-x64."
    }
    Copy-Item -LiteralPath $workspacePackage -Destination (Join-Path $tempBuildNodeModules "league-akari-native-win32") -Recurse -Force
    $source = $tempBuildSource
    $hostPackagePath = Join-Path $repoRoot "frontend\package.json"
    if (-not (Test-Path -LiteralPath $hostPackagePath -PathType Leaf)) {
      throw "Host package.json is missing: $hostPackagePath"
    }
    $hostPackage = Get-Content -LiteralPath $hostPackagePath -Raw -Encoding utf8 | ConvertFrom-Json
    $hostVersion = [string]$hostPackage.version
    if ($hostVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
      throw "Host package version is not valid semver: $hostVersion"
    }
    $runtimePackagePath = Join-Path $source "package.json"
    if (-not (Test-Path -LiteralPath $runtimePackagePath -PathType Leaf)) {
      throw "Pinned runtime package.json is missing: $runtimePackagePath"
    }
    $runtimePackageText = Get-Content -LiteralPath $runtimePackagePath -Raw -Encoding utf8
    $runtimeVersionPattern = '(?m)^(\s*"version"\s*:\s*")([^"]+)(".*)$'
    $runtimeVersionMatches = [regex]::Matches($runtimePackageText, $runtimeVersionPattern)
    if ($runtimeVersionMatches.Count -ne 1) {
      throw "Expected exactly one root runtime package version field (found $($runtimeVersionMatches.Count))."
    }
    $runtimeVersionMatch = $runtimeVersionMatches[0]
    $runtimeVersionLinePatched = $runtimeVersionMatch.Groups[1].Value + $hostVersion + $runtimeVersionMatch.Groups[3].Value
    $runtimePackageText = $runtimePackageText.Replace($runtimeVersionMatch.Value, $runtimeVersionLinePatched)
    [IO.File]::WriteAllText($runtimePackagePath, $runtimePackageText, [Text.UTF8Encoding]::new($false))
    $patchedRuntimePackage = Get-Content -LiteralPath $runtimePackagePath -Raw -Encoding utf8 | ConvertFrom-Json
    if ([string]$patchedRuntimePackage.version -ne $hostVersion) {
      throw "Failed to synchronize runtime package version with host version $hostVersion."
    }
    $yarn = Get-Command yarn.cmd -ErrorAction SilentlyContinue
    $yarnArgs = @()
    if (-not $yarn) {
      $corepack = Get-Command corepack.cmd -ErrorAction SilentlyContinue
      if (-not $corepack) { throw "-Build requires yarn.cmd or corepack.cmd; no package manager was installed." }
      $yarn = $corepack
      $yarnArgs = @("yarn")
    }
    # Enumerate only source roots that are known not to contain dependencies.
    # Filtering a whole-tree Get-ChildItem after traversal is unsafe on this
    # pinned checkout because nested node_modules contains broken/generated
    # junction paths on some Windows workstations.
    $brandingFiles = @()
    foreach ($brandingRoot in @(
      (Join-Path $source "src"),
      (Join-Path $source "scripts")
    )) {
      if (Test-Path -LiteralPath $brandingRoot -PathType Container) {
        $brandingFiles += Get-ChildItem -LiteralPath $brandingRoot -Recurse -File -Force -ErrorAction Stop
      }
    }
    $brandingFiles += Get-ChildItem -LiteralPath $source -File -Force -ErrorAction Stop
    $brandingFiles | Where-Object {
      $_.Extension -in @('.ts', '.tsx', '.vue', '.yaml', '.yml', '.json', '.md', '.html') -and
      $_.Name -notin @('LICENSE', 'LeagueAkari-MIT.txt') -and
      $_.FullName -notmatch '[\\/](node_modules|\.git|\.yarn|out|dist|coverage)[\\/]'
    } | ForEach-Object {
      $text = Get-Content -LiteralPath $_.FullName -Raw -Encoding utf8
      $text = $text.Replace('League Akari', 'MaxGameStudio')
      [IO.File]::WriteAllText($_.FullName, $text, [Text.UTF8Encoding]::new($false))
    }
    # Keep internal compatibility identifiers intact, but update the small set
    # of human-facing i18n labels and examples that use the upstream name.
    $i18nPatchFiles = @()
    foreach ($locale in @('en', 'zh-CN')) {
      $localeRoot = Join-Path $source "src\shared\i18n\$locale"
      if (-not (Test-Path -LiteralPath $localeRoot -PathType Container)) {
        throw "Pinned i18n locale directory is missing: $localeRoot"
      }
      $i18nPatchFiles += Get-ChildItem -LiteralPath $localeRoot -Recurse -File -Force -ErrorAction Stop |
        Where-Object { $_.Extension -eq '.yaml' }
    }
    # Keep this staging script BOM-free for Windows PowerShell 5.1. Build the
    # Chinese labels from code points so the script itself remains ASCII-only.
    $makeUnicodeText = {
      param([int[]]$CodePoints)
      -join ($CodePoints | ForEach-Object { [char]$_ })
    }
    $zhUse = & $makeUnicodeText @(0x4F7F, 0x7528)
    $zhStyle = & $makeUnicodeText @(0x98CE, 0x683C, 0x7684)
    $zhName = & $makeUnicodeText @(0x540D, 0x5B57)
    $zhPlaceholder = & $makeUnicodeText @(0x5360, 0x4F4D) + $zhName
    $zhStyledNameOld = $zhUse + ' Akari ' + $zhStyle + $zhName
    $zhStyledNameNew = $zhUse + ' MaxGameStudio ' + $zhStyle + $zhName
    $zhStyledPlaceholderOld = $zhUse + ' Akari ' + $zhStyle + $zhPlaceholder
    $zhStyledPlaceholderNew = $zhUse + ' MaxGameStudio ' + $zhStyle + $zhPlaceholder
    $zhBecause = & $makeUnicodeText @(0x56E0, 0x4E3A)
    $zhIs = & $makeUnicodeText @(0x662F)
    $zhUnique = & $makeUnicodeText @(0x72EC, 0x4E00, 0x65E0, 0x4E8C)
    $zhComma = & $makeUnicodeText @(0x7684, 0xFF0C)
    $zhTherefore = & $makeUnicodeText @(0x6240, 0x4EE5)
    $zhSameTime = & $makeUnicodeText @(0x540C, 0x4E00, 0x65F6, 0x95F4)
    $zhOnlyCanHave = & $makeUnicodeText @(0x53EA, 0x80FD, 0x6709)
    $zhOne = & $makeUnicodeText @(0x4E00, 0x4E2A)
    $zhSingletonOld = $zhBecause + ' Akari ' + $zhIs + $zhUnique + $zhComma + $zhTherefore + $zhSameTime + $zhOnlyCanHave + $zhOne + ' Akari'
    $zhSingletonNew = 'MaxGameStudio ' + (& $makeUnicodeText @(0x4EE5, 0x5355, 0x5B9E, 0x4F8B)) + (& $makeUnicodeText @(0x8FD0, 0x884C)) + (& $makeUnicodeText @(0xFF0C)) + $zhTherefore + $zhSameTime + $zhOnlyCanHave + $zhOne + ' MaxGameStudio'
    $i18nPatches = @(
      @{ Old = 'Akari Score'; New = 'MGS Score'; Expected = 12 }
      @{ Old = "Akari's Electron"; New = 'MaxGameStudio Electron'; Expected = 2 }
      @{ Old = 'Use Akari-Styled Name'; New = 'Use MaxGameStudio-Styled Name'; Expected = 1 }
      @{ Old = 'Use the Akari-styled names in summoner placeholder.'; New = 'Use MaxGameStudio-styled names in summoner placeholder.'; Expected = 1 }
      @{ Old = $zhStyledNameOld; New = $zhStyledNameNew; Expected = 1 }
      @{ Old = $zhStyledPlaceholderOld; New = $zhStyledPlaceholderNew; Expected = 1 }
      @{ Old = 'Because Akari is unique, there can only be ONE Akari at a time'; New = 'Because MaxGameStudio runs as a single instance, only ONE MaxGameStudio can run at a time'; Expected = 1 }
      @{ Old = $zhSingletonOld; New = $zhSingletonNew; Expected = 1 }
      @{ Old = "'Akari #Akaza'"; New = "'MGS #Example'"; Expected = 1 }
    )
    foreach ($i18nPatch in $i18nPatches) {
      $totalHits = 0
      foreach ($i18nFile in $i18nPatchFiles) {
        $i18nText = Get-Content -LiteralPath $i18nFile.FullName -Raw -Encoding utf8
        $hits = ([regex]::Matches($i18nText, [regex]::Escape($i18nPatch.Old))).Count
        if ($hits -gt 0) {
          $i18nText = $i18nText.Replace($i18nPatch.Old, $i18nPatch.New)
          [IO.File]::WriteAllText($i18nFile.FullName, $i18nText, [Text.UTF8Encoding]::new($false))
          $totalHits += $hits
        }
      }
      if ($totalHits -ne [int]$i18nPatch.Expected) {
        throw "Expected $($i18nPatch.Expected) i18n matches for '$($i18nPatch.Old)' (found $totalHits)."
      }
      $remainingHits = 0
      $newHits = 0
      foreach ($i18nFile in $i18nPatchFiles) {
        $patchedI18nText = Get-Content -LiteralPath $i18nFile.FullName -Raw -Encoding utf8
        $remainingHits += ([regex]::Matches($patchedI18nText, [regex]::Escape($i18nPatch.Old))).Count
        $newHits += ([regex]::Matches($patchedI18nText, [regex]::Escape($i18nPatch.New))).Count
      }
      if ($remainingHits -ne 0 -or $newHits -lt [int]$i18nPatch.Expected) {
        throw "I18n brand patch verification failed for '$($i18nPatch.Old)'."
      }
    }
    $visibleTitlePatches = @(
      @{ Path = (Join-Path $source 'src\main\shards\window-manager\opgg-window\window.ts'); Old = "static readonly TITLE = 'OP.GG Akari'"; New = "static readonly TITLE = 'OP.GG MaxGameStudio'" }
      @{ Path = (Join-Path $source 'src\main\shards\window-manager\ongoing-game-window\window.ts'); Old = "static readonly TITLE = 'Akari Ongoing Game Inspector'"; New = "static readonly TITLE = 'MaxGameStudio Ongoing Game Inspector'" }
      @{ Path = (Join-Path $source 'src\main\shards\window-manager\aux-window\window.ts'); Old = "static readonly TITLE = 'Mini Akari'"; New = "static readonly TITLE = 'Mini MaxGameStudio'" }
      @{ Path = (Join-Path $source 'src\renderer\src-main-window\components\settings-modal\DebugSettings.vue'); Old = 'title="Akari Zone"'; New = 'title="MaxGameStudio Zone"' }
      @{ Path = (Join-Path $source 'src\renderer-shared\components\ongoing-game-panel\widgets\player-info-card\player-card-tags\tags\akari-score.tsx'); Old = '>Akari {analysis.akariScore.total.toFixed(2)}</div>'; New = '>MGS {analysis.akariScore.total.toFixed(2)}</div>' }
      @{ Path = (Join-Path $source 'src\shared\utils\yuriyuri-names.ts'); Old = "    'Akari',"; New = "    'MaxGameStudio'," }
      @{ Path = (Join-Path $source 'src\renderer-shared\components\stories\basic\BasicComponentsDemo.vue'); Old = '<CopyableText text="EUW1 / Akari#2026">EUW1 / Akari#2026</CopyableText>'; New = '<CopyableText text="EUW1 / MGS#2026">EUW1 / MGS#2026</CopyableText>' }
      @{ Path = (Join-Path $source 'src\renderer-shared\components\stories\layout\LayoutComponentsDemo.vue'); Old = 'Akari#2026'; New = 'MGS#2026' }
    )
    foreach ($visibleTitlePatch in $visibleTitlePatches) {
      Replace-TextExactlyOnce $visibleTitlePatch.Path $visibleTitlePatch.Old $visibleTitlePatch.New
    }
    $autoChampConfigPath = Join-Path $source 'src\main\shards\auto-champ-config\auto-config-controller.ts'
    $autoChampConfigText = Get-Content -LiteralPath $autoChampConfigPath -Raw -Encoding utf8
    $autoChampOld = '[Akari] '
    $autoChampNew = '[MGS] '
    $autoChampHits = ([regex]::Matches($autoChampConfigText, [regex]::Escape($autoChampOld))).Count
    if ($autoChampHits -ne 2) {
      throw "Expected two visible auto-champ page-name prefixes (found $autoChampHits)."
    }
    $autoChampConfigText = $autoChampConfigText.Replace($autoChampOld, $autoChampNew)
    [IO.File]::WriteAllText($autoChampConfigPath, $autoChampConfigText, [Text.UTF8Encoding]::new($false))
    $patchedAutoChampConfigText = Get-Content -LiteralPath $autoChampConfigPath -Raw -Encoding utf8
    if (([regex]::Matches($patchedAutoChampConfigText, [regex]::Escape($autoChampOld))).Count -ne 0 -or
        ([regex]::Matches($patchedAutoChampConfigText, [regex]::Escape($autoChampNew))).Count -ne 2) {
      throw "Visible auto-champ page-name prefix patch did not apply exactly."
    }
    # The embedded runtime is supervised by MaxGameStudio. It must not display
    # the upstream first-run declaration, fetch upstream notices, report
    # version-usage statistics, or link product controls to the upstream repo.
    # Required MIT attribution remains in THIRD_PARTY_NOTICES instead.
    $notificationsIndex = Join-Path $source "src\renderer\src-main-window\shards\simple-notifications\index.ts"
    Replace-TextExactlyOnce $notificationsIndex "import { registerDeclarationModal } from './declaration-modal'" ""
    Replace-TextExactlyOnce $notificationsIndex "    registerDeclarationModal(this.context)" ""
    $apiMainPath = Join-Path $source "src\main\shards\akari-api\index.ts"
    Replace-TextExactlyOnce $apiMainPath "    this._noticeLoader.watch()" "    // MaxGameStudio owns user notices for the embedded runtime."
    $mainWindowPath = Join-Path $source "src\main\shards\window-manager\main-window\window.ts"
    $windowLineBreak = [Environment]::NewLine
    $closeStrategyAnchor = "    const s = this._nextCloseAction || this.settings.closeAction"
    $embeddedCloseMinimize = @(
      "    if (process.env.MAXGAMESTUDIO_EMBEDDED === '1' && s === 'minimize-to-tray') {"
      "      event.preventDefault()"
      "      this._window?.minimize()"
      "      this._nextCloseAction = null"
      "      return"
      "    }"
    ) -join $windowLineBreak
    $patchedCloseStrategyAnchor = $closeStrategyAnchor + $windowLineBreak + $embeddedCloseMinimize
    Replace-TextExactlyOnce $mainWindowPath $closeStrategyAnchor $patchedCloseStrategyAnchor
    $patchedWindowText = Get-Content -LiteralPath $mainWindowPath -Raw -Encoding utf8
    if (-not $patchedWindowText.Contains($embeddedCloseMinimize)) {
      throw "Embedded close minimization was not applied to main window handleClose."
    }
    $trueCloseIndex = $patchedWindowText.IndexOf('    if (this._trueClose || this._context.shared.global.isReadyToQuit) {')
    $closeStrategyIndex = $patchedWindowText.IndexOf($closeStrategyAnchor)
    $embeddedMinimizeIndex = $patchedWindowText.IndexOf("    if (process.env.MAXGAMESTUDIO_EMBEDDED === '1' && s === 'minimize-to-tray') {")
    if ($trueCloseIndex -lt 0 -or $closeStrategyIndex -le $trueCloseIndex -or $embeddedMinimizeIndex -le $closeStrategyIndex) {
      throw "Embedded close minimization order is unsafe: close strategy must be resolved first."
    }
    if ($patchedWindowText.Contains("if (process.env.MAXGAMESTUDIO_EMBEDDED === '1') {")) {
      throw "Embedded close path must not bypass the close strategy."
    }
    # Keep the runtime's existing `ask` and `quit` branches intact. In
    # embedded mode the `minimize-to-tray` strategy is deliberately mapped to
    # BrowserWindow.minimize(), because the embedded tray is disabled and the
    # taskbar must remain a reliable restore path.
    $shellEnglishPath = Join-Path $source "src\shared\i18n\en\renderer\shell.yaml"
    $shellChinesePath = Join-Path $source "src\shared\i18n\zh-CN\renderer\shell.yaml"
    $settingsEnglishPath = Join-Path $source "src\shared\i18n\en\renderer\settings.yaml"
    $settingsChinesePath = Join-Path $source "src\shared\i18n\zh-CN\renderer\settings.yaml"
    foreach ($i18nPath in @($shellEnglishPath, $shellChinesePath, $settingsEnglishPath, $settingsChinesePath)) {
      if (-not (Test-Path -LiteralPath $i18nPath -PathType Leaf)) {
        throw "Embedded close copy i18n target is missing: $i18nPath"
      }
    }
    # Keep this staging script ASCII-only for Windows PowerShell 5.1. Build
    # the Chinese labels from code points instead of embedding them literally.
    $zhCloseLeagueWorkspace = & $makeUnicodeText @(0x5173, 0x95ED, 0x82F1, 0x96C4, 0x8054, 0x76DF, 0x5DE5, 0x4F5C, 0x53F0)
    $zhMinimizeLeagueWorkspace = & $makeUnicodeText @(0x6700, 0x5C0F, 0x5316, 0x82F1, 0x96C4, 0x8054, 0x76DF, 0x5DE5, 0x4F5C, 0x53F0)
    $zhReturnToMaxGameStudio = (& $makeUnicodeText @(0x8FD4, 0x56DE)) + ' MaxGameStudio'
    $zhLeagueWorkspaceCloseAction = & $makeUnicodeText @(0x82F1, 0x96C4, 0x8054, 0x76DF, 0x5DE5, 0x4F5C, 0x53F0, 0x5173, 0x95ED, 0x884C, 0x4E3A)
    $zhCloseLeagueWorkspaceDescription = & $makeUnicodeText @(0x5173, 0x95ED, 0x82F1, 0x96C4, 0x8054, 0x76DF, 0x5DE5, 0x4F5C, 0x53F0, 0x65F6, 0x6267, 0x884C, 0x7684, 0x64CD, 0x4F5C)
    $zhQuitCommonAppName = (& $makeUnicodeText @(0x9000, 0x51FA)) + ' $t(common:appName)'
    $zhMinimizeToTray = & $makeUnicodeText @(0x6700, 0x5C0F, 0x5316, 0x5230, 0x6258, 0x76D8, 0x533A)
    $zhMinimizeToTraySettings = & $makeUnicodeText @(0x6700, 0x5C0F, 0x5316, 0x5230, 0x6258, 0x76D8)
    $zhQuitApplication = & $makeUnicodeText @(0x9000, 0x51FA, 0x5E94, 0x7528)
    $zhMainWindowCloseAction = & $makeUnicodeText @(0x4E3B, 0x7A97, 0x53E3, 0x5173, 0x95ED, 0x7B56, 0x7565)
    $zhMainWindowCloseDescription = & $makeUnicodeText @(0x5F53, 0x5173, 0x95ED, 0x4E3B, 0x7A97, 0x53E3, 0x65F6, 0x6240, 0x6267, 0x884C, 0x7684, 0x884C, 0x4E3A)
    $zhAskEveryTime = & $makeUnicodeText @(0x6BCF, 0x6B21, 0x8BE2, 0x95EE)
    Replace-TextExactlyOnce $shellEnglishPath "    title: Quit `$t(common:appName)" "    title: Close League Workspace"
    Replace-TextExactlyOnce $shellEnglishPath "      minimize-to-tray: Minimize to tray" "      minimize-to-tray: Minimize League Workspace"
    Replace-TextExactlyOnce $shellEnglishPath "      quit: Quit" "      quit: Return to MaxGameStudio"
    Replace-TextExactlyOnce $shellChinesePath ("    title: " + $zhQuitCommonAppName) ("    title: " + $zhCloseLeagueWorkspace)
    Replace-TextExactlyOnce $shellChinesePath ("      minimize-to-tray: " + $zhMinimizeToTray) ("      minimize-to-tray: " + $zhMinimizeLeagueWorkspace)
    Replace-TextExactlyOnce $shellChinesePath ("      quit: " + $zhQuitApplication) ("      quit: " + $zhReturnToMaxGameStudio)
    Replace-TextExactlyOnce $settingsEnglishPath "        label: Main Window Close Action" "        label: League Workspace Close Action"
    Replace-TextExactlyOnce $settingsEnglishPath "        description: The action to be executed when the main window is closed" "        description: The action to take when closing the League workspace"
    Replace-TextExactlyOnce $settingsEnglishPath "          minimize-to-tray: Minimize to tray" "          minimize-to-tray: Minimize League Workspace"
    Replace-TextExactlyOnce $settingsEnglishPath "          quit: Quit" "          quit: Return to MaxGameStudio"
    Replace-TextExactlyOnce $settingsChinesePath ("        label: " + $zhMainWindowCloseAction) ("        label: " + $zhLeagueWorkspaceCloseAction)
    Replace-TextExactlyOnce $settingsChinesePath ("        description: " + $zhMainWindowCloseDescription) ("        description: " + $zhCloseLeagueWorkspaceDescription)
    Replace-TextExactlyOnce $settingsChinesePath ("          minimize-to-tray: " + $zhMinimizeToTraySettings) ("          minimize-to-tray: " + $zhMinimizeLeagueWorkspace)
    Replace-TextExactlyOnce $settingsChinesePath ("          quit: " + $zhQuitApplication) ("          quit: " + $zhReturnToMaxGameStudio)
    $closeCopyContracts = @(
      @{ Path = $shellEnglishPath; Values = @('Close League Workspace', 'Minimize League Workspace', 'Return to MaxGameStudio') }
      @{ Path = $settingsEnglishPath; Values = @('League Workspace Close Action', 'Minimize League Workspace', 'Return to MaxGameStudio', 'Ask every time') }
      @{ Path = $shellChinesePath; Values = @($zhCloseLeagueWorkspace, $zhMinimizeLeagueWorkspace, $zhReturnToMaxGameStudio) }
      @{ Path = $settingsChinesePath; Values = @($zhLeagueWorkspaceCloseAction, $zhCloseLeagueWorkspaceDescription, $zhMinimizeLeagueWorkspace, $zhReturnToMaxGameStudio, $zhAskEveryTime) }
    )
    foreach ($contract in $closeCopyContracts) {
      $closeCopyText = Get-Content -LiteralPath $contract.Path -Raw -Encoding utf8
      foreach ($value in $contract.Values) {
        if (-not $closeCopyText.Contains($value)) {
          throw "Embedded close copy i18n contract is missing '$value': $($contract.Path)"
        }
      }
    }
    $settingsIpcPath = Join-Path $source "src\main\shards\setting-factory\ipc-handlers.ts"
    $settingsOldFileName = 'league-akari-settings.json'
    $settingsNewFileName = 'maxgamestudio-league-settings.json'
    $settingsText = Get-Content -LiteralPath $settingsIpcPath -Raw -Encoding utf8
    $settingsHits = ([regex]::Matches($settingsText, [regex]::Escape($settingsOldFileName))).Count
    if ($settingsHits -ne 2) {
      throw "Expected two settings default filenames in pinned source (found $settingsHits)."
    }
    $settingsText = $settingsText.Replace($settingsOldFileName, $settingsNewFileName)
    [IO.File]::WriteAllText($settingsIpcPath, $settingsText, [Text.UTF8Encoding]::new($false))
    $patchedSettingsText = Get-Content -LiteralPath $settingsIpcPath -Raw -Encoding utf8
    if (([regex]::Matches($patchedSettingsText, [regex]::Escape($settingsOldFileName))).Count -ne 0 -or
        ([regex]::Matches($patchedSettingsText, [regex]::Escape($settingsNewFileName))).Count -ne 2) {
      throw "Settings default filename patch did not apply exactly."
    }
    $savedPlayerIpcPath = Join-Path $source "src\main\shards\saved-player\ipc-handlers.ts"
    $savedPlayerOldFileName = 'league-akari-tagged-players.json'
    $savedPlayerNewFileName = 'maxgamestudio-league-tagged-players.json'
    $savedPlayerText = Get-Content -LiteralPath $savedPlayerIpcPath -Raw -Encoding utf8
    $savedPlayerHits = ([regex]::Matches($savedPlayerText, [regex]::Escape($savedPlayerOldFileName))).Count
    if ($savedPlayerHits -ne 2) {
      throw "Expected two tagged-player default filenames in pinned source (found $savedPlayerHits)."
    }
    $savedPlayerText = $savedPlayerText.Replace($savedPlayerOldFileName, $savedPlayerNewFileName)
    [IO.File]::WriteAllText($savedPlayerIpcPath, $savedPlayerText, [Text.UTF8Encoding]::new($false))
    $patchedSavedPlayerText = Get-Content -LiteralPath $savedPlayerIpcPath -Raw -Encoding utf8
    if (([regex]::Matches($patchedSavedPlayerText, [regex]::Escape($savedPlayerOldFileName))).Count -ne 0 -or
        ([regex]::Matches($patchedSavedPlayerText, [regex]::Escape($savedPlayerNewFileName))).Count -ne 2) {
      throw "Tagged-player default filename patch did not apply exactly."
    }
    $clientInstallationPath = Join-Path $source "src\main\shards\client-installation\index.ts"
    Replace-TextExactlyOnce $clientInstallationPath `
      "  private readonly _jumpListController: ClientInstallationJumpListController" `
      "  private readonly _jumpListController: ClientInstallationJumpListController | null"
    $jumpListConstructionOld = @(
      "    this._jumpListController = new ClientInstallationJumpListController("
      "      this._context,"
      "      this._launcher"
      "    )"
    ) -join [Environment]::NewLine
    $jumpListConstructionNew = @(
      "    this._jumpListController ="
      "      process.env.MAXGAMESTUDIO_EMBEDDED === '1'"
      "        ? null"
      "        : new ClientInstallationJumpListController(this._context, this._launcher)"
    ) -join [Environment]::NewLine
    Replace-TextExactlyOnce $clientInstallationPath $jumpListConstructionOld $jumpListConstructionNew
    $jumpListRegistrationOld = "    this._jumpListController.register()"
    $jumpListRegistrationNew = @(
      "    if (this._jumpListController) {"
      "      this._jumpListController.register()"
      "    }"
    ) -join [Environment]::NewLine
    Replace-TextExactlyOnce $clientInstallationPath $jumpListRegistrationOld $jumpListRegistrationNew
    $patchedClientInstallationText = Get-Content -LiteralPath $clientInstallationPath -Raw -Encoding utf8
    if (-not $patchedClientInstallationText.Contains("process.env.MAXGAMESTUDIO_EMBEDDED === '1'") -or
        -not $patchedClientInstallationText.Contains($jumpListRegistrationNew)) {
      throw "Embedded Jump List guard was not applied to client-installation shard."
    }
    $bootstrapPath = Join-Path $source "src\main\bootstrap\index.ts"
    Replace-TextExactlyOnce $bootstrapPath "import { StatisticsMain } from '@main/shards/statistics'" ""
    Replace-TextExactlyOnce $bootstrapPath "    manager.use(StatisticsMain)" ""
    Replace-TextExactlyOnce $bootstrapPath "    manager.use(TrayMain)" (@(
      "    if (process.env.MAXGAMESTUDIO_EMBEDDED !== '1') {"
      "      manager.use(TrayMain)"
      "    }"
    ) -join [Environment]::NewLine)
    $bootstrapProtocolOld = @(
      "    if (process.defaultApp) {"
      "      const appPath = path.resolve(process.argv[1])"
      "      app.setAsDefaultProtocolClient(DEEP_LINK_PROTOCOL, process.execPath, [appPath])"
      "    } else {"
      "      app.setAsDefaultProtocolClient(DEEP_LINK_PROTOCOL)"
      "    }"
    ) -join [Environment]::NewLine
    $bootstrapProtocolNew = @(
      "    if (process.env.MAXGAMESTUDIO_EMBEDDED !== '1') {"
      "      if (process.defaultApp) {"
      "        const appPath = path.resolve(process.argv[1])"
      "        app.setAsDefaultProtocolClient(DEEP_LINK_PROTOCOL, process.execPath, [appPath])"
      "      } else {"
      "        app.setAsDefaultProtocolClient(DEEP_LINK_PROTOCOL)"
      "      }"
      "    }"
    ) -join [Environment]::NewLine
    Replace-TextExactlyOnce $bootstrapPath $bootstrapProtocolOld $bootstrapProtocolNew
    $patchedBootstrapText = Get-Content -LiteralPath $bootstrapPath -Raw -Encoding utf8
    if (-not $patchedBootstrapText.Contains("process.env.MAXGAMESTUDIO_EMBEDDED !== '1'") -or
        -not $patchedBootstrapText.Contains($bootstrapProtocolNew) -or
        -not $patchedBootstrapText.Contains("manager.use(TrayMain)")) {
      throw "Embedded protocol and tray guards were not applied to bootstrap."
    }
    $commonConstantsPath = Join-Path $source "src\shared\constants\common.ts"
    Replace-TextExactlyOnce $commonConstantsPath "https://github.com/LeagueAkari/LeagueAkari" "https://github.com/INEEDBUG/MaxGameStudio"
    $shellEnglishPath = Join-Path $source "src\shared\i18n\en\renderer\shell.yaml"
    $shellChinesePath = Join-Path $source "src\shared\i18n\zh-CN\renderer\shell.yaml"
    Replace-TextExactlyOnce $shellEnglishPath "https://github.com/LeagueAkari/LeagueAkari" "https://github.com/INEEDBUG/MaxGameStudio"
    Replace-TextExactlyOnce $shellChinesePath "https://github.com/LeagueAkari/LeagueAkari" "https://github.com/INEEDBUG/MaxGameStudio"
    $aboutPanePath = Join-Path $source "src\renderer\src-main-window\components\settings-modal\AboutPane.vue"
    Replace-TextExactlyOnce $aboutPanePath "https://api.github.com/repos/LeagueAkari/LeagueAkari" "https://api.github.com/repos/INEEDBUG/MaxGameStudio"
    $platformPath = Join-Path $source "src\main\shards\self-update\platform.ts"
    $builderConfig = Join-Path $source "electron-builder.yml"
    if (Test-Path -LiteralPath $builderConfig) {
      $builderText = Get-Content -LiteralPath $builderConfig -Raw -Encoding utf8
      $builderFields = @{
        'productName' = 'MaxGameStudio League'
        'executableName' = 'MaxGameStudioLeague'
        'appId' = 'com.maxgamestudio.league'
      }
      foreach ($entry in $builderFields.GetEnumerator()) {
        $pattern = "(?m)^(\s*)$([regex]::Escape($entry.Key)):\s*.*$"
        $hits = ([regex]::Matches($builderText, $pattern)).Count
        if ($hits -ne 1) {
          throw "Expected exactly one $($entry.Key) field in pinned electron-builder.yml (found $hits)."
        }
        $builderText = [regex]::Replace($builderText, $pattern, "`$1$($entry.Key): $($entry.Value)")
      }
      [IO.File]::WriteAllText($builderConfig, $builderText, [Text.UTF8Encoding]::new($false))
    } else {
      throw "Pinned source is missing electron-builder.yml; refusing to build an unbranded runtime."
    }
    $leagueWorkbenchIcon = Join-Path $repoRoot "packaging\windows\league-workbench-icon.ico"
    if (-not (Test-Path -LiteralPath $leagueWorkbenchIcon -PathType Leaf)) {
      throw "Dedicated League workbench icon is missing: $leagueWorkbenchIcon"
    }
    Apply-MaxGameStudioTheme $source $leagueWorkbenchIcon
    Apply-EmbeddedUiPruning $source
    if (Test-Path -LiteralPath $platformPath) {
      $platformText = Get-Content -LiteralPath $platformPath -Raw -Encoding utf8
      $lifecyclePattern = "return platform === 'win32' && arch === 'x64'"
      $lifecycleHits = ([regex]::Matches($platformText, [regex]::Escape($lifecyclePattern))).Count
      if ($lifecycleHits -ne 1) {
        throw "Expected exactly one self-update platform predicate in pinned platform.ts (found $lifecycleHits)."
      }
      $disabledLifecycle = "void platform`n  void arch`n  return false"
      $platformText = $platformText.Replace($lifecyclePattern, $disabledLifecycle)
      [IO.File]::WriteAllText($platformPath, $platformText, [Text.UTF8Encoding]::new($false))
    } else {
      throw "Pinned source is missing self-update platform.ts; refusing to build without disabling self-update."
    }
    Push-Location $source
    try {
      & $yarn.Source @yarnArgs build:win
      if ($LASTEXITCODE -ne 0) { throw "Upstream build failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }
    $builtAssetVerifier = Join-Path $repoRoot "packaging\windows\verify-league-runtime-assets.ps1"
    & $builtAssetVerifier -Root $source -Mode Built
    $runtimeSource = Join-Path $source "dist\win-unpacked"
    if (-not (Test-Path -LiteralPath (Join-Path $runtimeSource "resources\app.asar"))) {
      throw "Build completed without a runnable dist\win-unpacked/resources/app.asar."
    }
    Copy-Item -LiteralPath $runtimeSource -Destination $OutputDir -Recurse -Force
    $sourceMode = "built-pinned-source"
    Restore-UnpackedNativeHeaders $source $OutputDir
  }

  # Keep the copied source visually neutral when it is later compiled into
  # MaxGameStudio. Identifiers such as `league-akari-native-win32` are left
  # untouched because they are package/API contracts; only human-facing text
  # is rewritten.
  Get-ChildItem -LiteralPath $OutputDir -Recurse -File | Where-Object {
    $_.Extension -in @('.ts', '.tsx', '.vue', '.yaml', '.yml', '.json', '.md', '.html') -and
    $_.Name -notin @('LICENSE', 'LeagueAkari-MIT.txt')
  } | ForEach-Object {
    $text = Get-Content -LiteralPath $_.FullName -Raw -Encoding utf8
    $text = $text.Replace('League Akari', 'MaxGameStudio')
    [IO.File]::WriteAllText($_.FullName, $text, [Text.UTF8Encoding]::new($false))
  }

  # Branding and updater policy are applied to source before a build. A
  # prebuilt runtime is copied byte-for-byte so its asar signature/content is
  # not silently invalidated; the host must disable its updater integration.

  $runtimeExeName = if ($sourceMode -eq "built-pinned-source") { "MaxGameStudioLeague.exe" } else { "LeagueAkari.exe" }
  $requiredRuntimePaths = @(
    (Join-Path $OutputDir $runtimeExeName),
    (Join-Path $OutputDir "resources\app.asar"),
    (Join-Path $OutputDir "resources\app.asar.unpacked"),
    (Join-Path $OutputDir "LICENSE.electron.txt"),
    (Join-Path $OutputDir "LICENSES.chromium.html")
  )
  foreach ($path in $requiredRuntimePaths) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Runnable runtime is incomplete; missing $path" }
  }
  $privateData = Get-ChildItem -LiteralPath $OutputDir -Recurse -Force -ErrorAction Stop | Where-Object {
    $_.Name -match '^(?:Cache|GPUCache|Local Storage|Session Storage|Cookies)$' -or
    $_.Extension -in @('.log', '.db', '.sqlite', '.sqlite3')
  }
  if ($privateData) {
    throw "Runtime contains user data/cache files; refusing to stage: $($privateData.FullName -join ', ')"
  }
  # Keep updater/elevation binaries intact: the pinned upstream source proves
  # they are consumed by self-update/uninstall only. The platform patch (for
  # source builds) disables those call paths while preserving the files for
  # auditability and reproducible packaging.

  $noticeDir = Join-Path $OutputDir "THIRD_PARTY_NOTICES"
  New-Item -ItemType Directory -Path $noticeDir -Force | Out-Null
  @"
LeagueAkari source component
Copyright (c) 2026 Hanxven

This component is distributed under the MIT License. The original license
text is preserved below and applies to the copied LeagueAkari source tree.
Source: $Repository
Pinned commit: $PinnedCommit

MIT License

Copyright (c) 2026 Hanxven

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"@ | Set-Content -LiteralPath (Join-Path $noticeDir "LeagueAkari-MIT.txt") -Encoding utf8

$manifest = [ordered]@{
    component = "league-runtime"
    sourceRepository = $Repository
    sourceCommit = $PinnedCommit
    sourceVersion = "1.5.1"
    productBrand = "MaxGameStudio League"
    upstreamUpdaterDisabledByHost = $true
     expectedRuntimeMemoryMb = 800
     memoryEstimateSource = "local v1.5.1 isolated-profile smoke test measured about 774 MiB; actual usage varies by system/open view"
    buildRequested = [bool]$Build
    sourceMode = if ($FetchPinnedSource) { "fetched-pinned" } else { $sourceMode }
    runnable = $true
    updaterOwnership = "MaxGameStudio host"
    upstreamUpdaterFilesRetained = $true
    upstreamNoticeFetchDisabled = $true
    upstreamUsageStatisticsDisabled = $true
    upstreamFirstRunDeclarationDisabled = $true
  }
  $manifestJson = $manifest | ConvertTo-Json -Depth 10
  [IO.File]::WriteAllText((Join-Path $OutputDir "maxgamestudio-runtime-manifest.json"), $manifestJson, [Text.UTF8Encoding]::new($false))
  $hashes = [ordered]@{}
  Get-ChildItem -LiteralPath $OutputDir -Recurse -File | Where-Object {
    $_.Name -ne "maxgamestudio-runtime-hashes.json"
  } | Sort-Object FullName | ForEach-Object {
    $relative = (Get-RelativePathCompat $OutputDir $_.FullName).Replace('\', '/')
    $hashes[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  }
  $hashesJson = $hashes | ConvertTo-Json -Depth 10
  [IO.File]::WriteAllText((Join-Path $OutputDir "maxgamestudio-runtime-hashes.json"), $hashesJson, [Text.UTF8Encoding]::new($false))
  if (-not (Test-Path -LiteralPath (Join-Path $OutputDir "THIRD_PARTY_NOTICES\LeagueAkari-MIT.txt"))) {
    throw "MIT notice was not staged."
  }
  Write-Host "League runtime staged at $OutputDir"
  Write-Host "Pinned source: $PinnedCommit"
} finally {
  if ($tempSource -and (Test-Path -LiteralPath $tempSource)) {
    Remove-Item -LiteralPath $tempSource -Recurse -Force -ErrorAction SilentlyContinue
  }
  if ($tempBuildSource -and (Test-Path -LiteralPath $tempBuildSource)) {
    Remove-Item -LiteralPath $tempBuildSource -Recurse -Force -ErrorAction SilentlyContinue
  }
}
