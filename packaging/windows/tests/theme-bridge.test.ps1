$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$stageScript = Join-Path $repoRoot 'packaging\windows\stage-league-runtime.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($stageScript, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw "Could not parse theme staging script: $($parseErrors[0].Message)" }
$definition = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Add-MaxGameStudioGlassTheme' }, $true)
if (-not $definition) { throw 'Theme bridge helper is missing.' }
. ([scriptblock]::Create($definition.Extent.Text))

$original = "/* retained upstream rules */`n:root { --upstream-token: #123456; }`n"
$result = Add-MaxGameStudioGlassTheme $original
foreach ($required in @(
  '/* MaxGameStudio glass bridge: static-only */',
  '--mgs-glass-surface',
  "#f59e0b",
  "#2563eb",
  '.app-sidebar',
  '.menu-item__inner',
  '.n-card',
  '.n-tabs-nav',
  '.n-input',
  ':focus-visible',
  'prefers-reduced-transparency',
  'prefers-reduced-motion',
  'forced-colors'
)) {
  if (-not $result.Contains($required)) { throw "Theme bridge output is missing required rule or token: $required" }
}
if (-not $result.StartsWith($original)) { throw 'Theme bridge must preserve existing upstream stylesheet content.' }
if ($result -match '\.n-button--primary-type|\.app-sidebar__expand-line:hover|backdrop-filter|will-change|(?s)\.n-button\s*\{[^}]*\b(?:background(?:-color)?|color)\s*:') {
  throw 'Theme bridge must not override Naive primary buttons or add costly/forbidden effects.'
}
if (([regex]::Matches($result, 'MaxGameStudio glass bridge: static-only')).Count -ne 1) {
  throw 'Theme bridge marker must be unique.'
}
if ((Add-MaxGameStudioGlassTheme $result) -cne $result) {
  throw 'Theme bridge injection must be idempotent.'
}
Write-Host 'THEME_BRIDGE_VERIFIED: static glass rules, theme tokens, accessibility fallbacks, preserved source, and idempotent injection.'
