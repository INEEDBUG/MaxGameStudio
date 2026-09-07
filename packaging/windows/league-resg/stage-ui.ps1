#Requires -Version 5.1
<##
.SYNOPSIS
  Stage the independent RESG settings and toolbar components into the pinned UI.

.DESCRIPTION
  This helper is intentionally separate from the runtime staging script. It copies
  only the two local Vue components and applies exact, version-pinned anchors to
  the existing multi-window settings, titlebar, and bilingual renderer i18n files.
##>
param(
  [Parameter(Mandatory = $true)]
  [string]$SourceRoot
)

$ErrorActionPreference = 'Stop'
$nl = [Environment]::NewLine

function Replace-TextExactlyOnce([string]$PathValue, [string]$OldValue, [string]$NewValue) {
  if (-not (Test-Path -LiteralPath $PathValue -PathType Leaf)) {
    throw "RESG UI patch target is missing: $PathValue"
  }
  $text = Get-Content -LiteralPath $PathValue -Raw -Encoding utf8
  $hits = ([regex]::Matches($text, [regex]::Escape($OldValue))).Count
  if ($hits -ne 1) {
    throw "Expected exactly one RESG UI patch match in $PathValue (found $hits)."
  }
  [IO.File]::WriteAllText($PathValue, $text.Replace($OldValue, $NewValue), [Text.UTF8Encoding]::new($false))
}

$sourceDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$componentDir = Join-Path $SourceRoot 'src\renderer\src-main-window\components\resg-window'
New-Item -ItemType Directory -Path $componentDir -Force | Out-Null
foreach ($name in @('ResgWindowSettings.vue', 'ResgWindowButton.vue')) {
  $source = Join-Path $sourceDir $name
  if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "RESG component is missing: $source" }
  Copy-Item -LiteralPath $source -Destination (Join-Path $componentDir $name) -Force
}

$multiWindow = Join-Path $SourceRoot 'src\renderer\src-main-window\components\settings-modal\MultiWindowSettings.vue'
$multiAnchor = '      </SettingsSection>' + $nl + '      <SettingsSection' + $nl + '        :title="' + $nl + '          as.isElevated' + $nl + '            ? t(''settings.multiWindow.ongoingGameWindow.title'')'
$multiReplacement = '      </SettingsSection>' + $nl + '      <ResgWindowSettings />' + $nl + '      <SettingsSection' + $nl + '        :title="' + $nl + '          as.isElevated' + $nl + '            ? t(''settings.multiWindow.ongoingGameWindow.title'')'
Replace-TextExactlyOnce $multiWindow $multiAnchor $multiReplacement
Replace-TextExactlyOnce $multiWindow "import SettingsSection from '@renderer-shared/components/SettingsSection.vue'" ("import SettingsSection from '@renderer-shared/components/SettingsSection.vue'" + $nl + "import ResgWindowSettings from '../resg-window/ResgWindowSettings.vue'")

$titlebar = Join-Path $SourceRoot 'src\renderer\src-main-window\components\titlebar\CommonButtons.vue'
$buttonAnchor = "    </HorizontalExpand>" + $nl + $nl + "    <!-- theme selector -->"
$buttonReplacement = "    </HorizontalExpand>" + $nl + $nl + "    <ResgWindowButton />" + $nl + $nl + "    <!-- theme selector -->"
Replace-TextExactlyOnce $titlebar $buttonAnchor $buttonReplacement
Replace-TextExactlyOnce $titlebar "import HorizontalExpand from '@renderer-shared/components/HorizontalExpand.vue'" ("import HorizontalExpand from '@renderer-shared/components/HorizontalExpand.vue'" + $nl + "import ResgWindowButton from '../resg-window/ResgWindowButton.vue'")

$enSettings = Join-Path $SourceRoot 'src\shared\i18n\en\renderer\settings.yaml'
$zhSettings = Join-Path $SourceRoot 'src\shared\i18n\zh-CN\renderer\settings.yaml'
$enResg = @'
    resgWindow:
      title: RESG Window
      enabled:
        label: Enable RESG Window
        description: Open the RESG data window independently from OP.GG.
      autoShow:
        label: Auto Show
        description: Automatically show RESG when Hextech ARAM champion data is available.
      alwaysOnTop:
        label: Keep on top
        description: Keep the RESG window above other windows.
      status:
        label: Status
        description: RESG is limited to Hextech ARAM.
        disabled: Disabled
        waiting: Waiting
        loading: Loading
        ready: Ready
        error: Error
      retry: Retry
      open: Open
      close: Close
      scope: 'Scope: Hextech ARAM only. Other modes wait without opening a window.'
      genericError: RESG action failed. Try again.
'@
$zhResg = @'
    resgWindow:
      title: RESG 窗口
      enabled:
        label: 启用 RESG 窗口
        description: 独立于 OP.GG 打开 RESG 数据窗口。
      autoShow:
        label: 自动弹出
        description: Hextech ARAM 有可用英雄数据时自动显示 RESG。
      alwaysOnTop:
        label: 始终置顶
        description: 让 RESG 窗口保持在其他窗口上方。
      status:
        label: 状态
        description: RESG 仅支持海克斯大乱斗。
        disabled: 已禁用
        waiting: 等待中
        loading: 加载中
        ready: 就绪
        error: 出错
      retry: 重试
      open: 打开
      close: 关闭
      scope: '范围：仅海克斯大乱斗。其他模式保持等待，不弹出窗口。'
      genericError: RESG 操作失败，请重试。
'@
Replace-TextExactlyOnce $enSettings "    ongoingGameWindow:" ($enResg.TrimEnd() + $nl + "    ongoingGameWindow:")
Replace-TextExactlyOnce $zhSettings "    ongoingGameWindow:" ($zhResg.TrimEnd() + $nl + "    ongoingGameWindow:")

$enShell = Join-Path $SourceRoot 'src\shared\i18n\en\renderer\shell.yaml'
$zhShell = Join-Path $SourceRoot 'src\shared\i18n\zh-CN\renderer\shell.yaml'
Replace-TextExactlyOnce $enShell ("    opggWindow: OP.GG" + $nl) ("    opggWindow: OP.GG" + $nl + "    resgWindow: RESG" + $nl + "    resgUnavailable: RESG unavailable for the current game mode" + $nl)
Replace-TextExactlyOnce $zhShell ("    opggWindow: OP.GG" + $nl) ("    opggWindow: OP.GG" + $nl + "    resgWindow: RESG" + $nl + "    resgUnavailable: 当前模式暂不支持 RESG" + $nl)

Write-Host "RESG UI staged in $SourceRoot"
