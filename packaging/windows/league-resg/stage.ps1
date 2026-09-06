#Requires -Version 5.1
param([Parameter(Mandatory = $true)][string]$SourceRoot)
$ErrorActionPreference = 'Stop'
$nl = [Environment]::NewLine
function Replace-ResgAnchor([string]$PathValue, [string]$OldValue, [string]$NewValue) {
  $text = Get-Content -LiteralPath $PathValue -Raw -Encoding utf8
  $hits = ([regex]::Matches($text, [regex]::Escape($OldValue))).Count
  if ($hits -ne 1) { throw "RESG requires exactly one pinned anchor in $PathValue (found $hits)." }
  [IO.File]::WriteAllText($PathValue, $text.Replace($OldValue, $NewValue), [Text.UTF8Encoding]::new($false))
}
$destination = Join-Path $SourceRoot 'src\main\shards\window-manager\resg-window'
New-Item -ItemType Directory -Path $destination -Force | Out-Null
foreach ($name in @('resg-policy.ts', 'resg-display.ts', 'resg-window-controller.ts', 'resg-policy.test.ts', 'resg-window-controller.test.ts')) {
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $destination $name) -Force
}
$manager = Join-Path $SourceRoot 'src\main\shards\window-manager\index.ts'
Replace-ResgAnchor $manager "import { AkariOpggWindow } from './opgg-window/window'" ("import { AkariOpggWindow } from './opgg-window/window'" + $nl + "import { ResgWindowController } from './resg-window/resg-window-controller'")
Replace-ResgAnchor $manager '  public readonly opggWindow: AkariOpggWindow' ('  public readonly opggWindow: AkariOpggWindow' + $nl + '  public readonly resgWindow: ResgWindowController')
Replace-ResgAnchor $manager '    this.opggWindow = new AkariOpggWindow(this._context)' ('    this.opggWindow = new AkariOpggWindow(this._context)' + $nl + '    this.resgWindow = new ResgWindowController(this._context)')
Replace-ResgAnchor $manager '    await this._lifecycleController.init()' ('    await this._lifecycleController.init()' + $nl + '    await this.resgWindow.onInit()')
Replace-ResgAnchor $manager '  async onFinish() {' ('  async onDispose() {' + $nl + '    this.resgWindow.onDispose()' + $nl + '  }' + $nl + $nl + '  async onFinish() {')
& (Join-Path $PSScriptRoot 'stage-ui.ps1') -SourceRoot $SourceRoot
