param([Parameter(Mandatory = $true)][string]$PinnedSource)

$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
function Get-ScriptAst([string]$Path) {
  $errors = $null
  $ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Resolve-Path -LiteralPath $Path).Path, [ref]$null, [ref]$errors
  )
  if ($errors) { throw $errors[0].Message }
  $ast
}

$shared = Get-ScriptAst (Join-Path $PSScriptRoot '../stage-league-runtime.ps1')
$standalone = Get-ScriptAst (Join-Path $PSScriptRoot 'stage.ps1')
$sharedHelper = $shared.Find({
  param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Replace-TextExactlyOnce'
}, $true)
$standaloneHelper = $standalone.Find({
  param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Update-StandaloneSource'
}, $true)
$sharedEdit = $shared.Find({
  param($node)
  $node -is [System.Management.Automation.Language.CommandAst] -and
    $node.GetCommandName() -eq 'Replace-TextExactlyOnce' -and $node.CommandElements[1].Extent.Text -eq '$loggerPath'
}, $true)
$standaloneEdits = $standalone.FindAll({
  param($node)
  $node -is [System.Management.Automation.Language.CommandAst] -and
    $node.GetCommandName() -eq 'Update-StandaloneSource' -and $node.CommandElements[1].Extent.Text -eq '$logger'
}, $true)
$anchor = $standalone.Find({
  param($node)
  $node -is [System.Management.Automation.Language.AssignmentStatementAst] -and $node.Left.Extent.Text -eq '$oldLoggerPath'
}, $true)
if (-not ($sharedHelper -and $standaloneHelper -and $sharedEdit -and $standaloneEdits.Count -and $anchor)) {
  throw 'Logger patch operations were not found.'
}

. ([scriptblock]::Create($sharedHelper.Extent.Text))
. ([scriptblock]::Create($standaloneHelper.Extent.Text))
. ([scriptblock]::Create($anchor.Extent.Text))
$logger = 'src/main/logger/index.ts'
$pinned = [IO.File]::ReadAllText((Join-Path (Resolve-Path -LiteralPath $PinnedSource).Path $logger))
$lf = $pinned -replace '\r?\n', "`n"
$crlf = $lf.Replace("`n", "`r`n")
$mixed = $crlf -replace "(?m)(  const logsDir =)`r`n", "`$1`n"
$SourceRoot = Join-Path ([IO.Path]::GetTempPath()) ('mgs-league-stage-logger-' + [Guid]::NewGuid())
$loggerPath = Join-Path $SourceRoot $logger
$variants = @(
  @{ Name = 'LF'; Source = $lf },
  @{ Name = 'CRLF'; Source = $crlf },
  @{ Name = 'Mixed'; Source = $mixed }
)
$failures = @()
try {
  [void][IO.Directory]::CreateDirectory((Split-Path -Parent $loggerPath))
  foreach ($variant in $variants) {
    try {
      [IO.File]::WriteAllText($loggerPath, $variant.Source, $utf8)
      & ([scriptblock]::Create($sharedEdit.Extent.Text))
      $afterShared = [IO.File]::ReadAllText($loggerPath)
      if (-not $afterShared.Contains("process.env.MAXGAMESTUDIO_EMBEDDED === '1'")) {
        throw 'The shared embedded logger predicate was not applied.'
      }
      foreach ($edit in $standaloneEdits) {
        & ([scriptblock]::Create($edit.Extent.Text))
      }
      $output = [IO.File]::ReadAllText($loggerPath)
      if ($output -match '(?m)^import\s+path\s+from\s+') {
        throw 'The standalone logger carries an unused path import.'
      }
      if (-not $output.Contains("const logsDir = app.getPath('logs')") -or
          $output.Contains("app.getPath('exe')") -or $output.Contains('MAXGAMESTUDIO_EMBEDDED')) {
        throw 'The standalone logger did not use the selected-profile logs path.'
      }
      Write-Output "$($variant.Name): the staged logger uses app.getPath('logs') without an unused path import."
    } catch {
      $failures += "$($variant.Name): $($_.Exception.Message.Split([char]10)[0])"
    }
  }
  if ($failures) { throw ($failures -join [Environment]::NewLine) }
} finally {
  if ([IO.File]::Exists($loggerPath)) {
    [IO.File]::Delete($loggerPath)
  }
  foreach ($relative in @('src/main/logger', 'src/main', 'src', '.')) {
    $directory = Join-Path $SourceRoot $relative
    if ([IO.Directory]::Exists($directory)) { [IO.Directory]::Delete($directory) }
  }
}
