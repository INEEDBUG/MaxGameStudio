#Requires -Version 5.1

[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$Root,

  [ValidateSet('Source', 'Built')]
  [string]$Mode = 'Built'
)

$ErrorActionPreference = 'Stop'

function Assert-RendererLogoPng([string]$PathValue) {
  if (-not (Test-Path -LiteralPath $PathValue -PathType Leaf)) {
    throw "Renderer logo asset is missing: $PathValue"
  }

  $bytes = [IO.File]::ReadAllBytes($PathValue)
  $signature = [byte[]](137, 80, 78, 71, 13, 10, 26, 10)
  if ($bytes.Length -lt $signature.Length) {
    throw "Renderer logo asset is too small to be a PNG: $PathValue"
  }
  for ($index = 0; $index -lt $signature.Length; $index++) {
    if ($bytes[$index] -ne $signature[$index]) {
      throw "Renderer logo asset has an invalid PNG signature: $PathValue"
    }
  }
  $nonZeroPayloadBytes = 0
  for ($index = $signature.Length; $index -lt $bytes.Length; $index++) {
    if ($bytes[$index] -ne 0) { $nonZeroPayloadBytes++ }
  }
  if ($nonZeroPayloadBytes -lt 128) {
    throw "Renderer logo asset has no meaningful PNG payload: $PathValue"
  }

  Add-Type -AssemblyName System.Drawing
  $image = $null
  try {
    $image = [Drawing.Image]::FromFile($PathValue)
    if ($image.RawFormat.Guid -ne [Drawing.Imaging.ImageFormat]::Png.Guid) {
      throw "Renderer logo asset is not decoded as PNG: $PathValue"
    }
    if ($image.Width -lt 128 -or $image.Height -lt 128 -or $image.Width -ne $image.Height) {
      throw "Renderer logo asset must be square and at least 128x128 (got $($image.Width)x$($image.Height)): $PathValue"
    }

    # The canonical MaxGameStudio mark has transparent corners. A non-zero
    # corner alpha catches the legacy PNG-in-ICO decoder producing noise.
    $corner = $image.GetPixel(0, 0)
    $center = $image.GetPixel([int]($image.Width / 2), [int]($image.Height / 2))
    if ($corner.A -ne 0 -or $center.A -eq 0) {
      throw "Renderer logo asset has invalid alpha samples (corner=$($corner.A), center=$($center.A)): $PathValue"
    }
  } finally {
    if ($image) { $image.Dispose() }
  }

  return [IO.Path]::GetFileName($PathValue)
}

function Assert-SourceReferences([string]$SourceRoot, [string]$AssetPath) {
  $logoComponentPath = Join-Path $SourceRoot 'src\renderer-shared\assets\icon\AkariLogo.vue'
  $aboutPanePath = Join-Path $SourceRoot 'src\renderer\src-main-window\components\settings-modal\AboutPane.vue'
  foreach ($path in @($logoComponentPath, $aboutPanePath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
      throw "Renderer logo reference target is missing: $path"
    }
  }

  $logoText = Get-Content -LiteralPath $logoComponentPath -Raw -Encoding utf8
  $logoReference = 'href="../logo/maxgamestudio-logo.png"'
  if (([regex]::Matches($logoText, [regex]::Escape($logoReference))).Count -ne 1) {
    throw "AkariLogo.vue must contain exactly one generated PNG reference: $logoComponentPath"
  }
  if ($logoText.Contains('logo-hollow.svg')) {
    throw "AkariLogo.vue still references the upstream hollow logo: $logoComponentPath"
  }

  $aboutText = Get-Content -LiteralPath $aboutPanePath -Raw -Encoding utf8
  $aboutReference = 'src="@renderer-shared/assets/logo/maxgamestudio-logo.png"'
  if (([regex]::Matches($aboutText, [regex]::Escape($aboutReference))).Count -ne 1) {
    throw "AboutPane.vue must contain exactly one generated PNG reference: $aboutPanePath"
  }
  if ($aboutText.Contains('logo-hollow.svg')) {
    throw "AboutPane.vue still references the upstream hollow logo: $aboutPanePath"
  }

  Write-Host "Source logo references: PASS"
}

function Assert-BuiltReferences([string]$BuiltRoot, [string]$AssetName) {
  $rendererRoot = Join-Path $BuiltRoot 'out\renderer'
  if (-not (Test-Path -LiteralPath $rendererRoot -PathType Container)) {
    throw "Built renderer output is missing: $rendererRoot"
  }
  $javascriptFiles = @(Get-ChildItem -LiteralPath $rendererRoot -Recurse -File -Filter '*.js' -ErrorAction Stop)
  if ($javascriptFiles.Count -eq 0) {
    throw "Built renderer output contains no JavaScript bundles: $rendererRoot"
  }
  $javascript = ($javascriptFiles | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw -Encoding utf8 }) -join "`n"
  $assetReferences = @(
    [regex]::Matches($javascript, 'maxgamestudio-logo-[A-Za-z0-9_-]+\.png') |
      ForEach-Object { $_.Value } |
      Select-Object -Unique
  )
  if ($assetReferences.Count -ne 1 -or $assetReferences[0] -ne $AssetName) {
    throw "Built renderer must reference exactly $AssetName (found $($assetReferences -join ', ')): $rendererRoot"
  }
  if ($javascript -notmatch 'new URL\([^)]*maxgamestudio-logo-[A-Za-z0-9_-]+\.png[^)]*import\.meta\.url') {
    throw "Built renderer logo reference is not Vite URL-based: $rendererRoot"
  }
  Write-Host "Built logo reference: PASS ($AssetName)"
}

$rootPath = (Resolve-Path -LiteralPath $Root).Path
if ($Mode -eq 'Source') {
  $assetPath = Join-Path $rootPath 'src\renderer-shared\assets\logo\maxgamestudio-logo.png'
  Assert-RendererLogoPng $assetPath | Out-Null
  Assert-SourceReferences $rootPath $assetPath
  Write-Host "League runtime source assets verified: $rootPath"
  return
}

$rendererAssetsRoot = Join-Path $rootPath 'out\renderer\assets'
if (-not (Test-Path -LiteralPath $rendererAssetsRoot -PathType Container)) {
  throw "Built renderer assets are missing: $rendererAssetsRoot"
}
$assetCandidates = @(
  Get-ChildItem -LiteralPath $rendererAssetsRoot -File -Filter 'maxgamestudio-logo-*.png' -ErrorAction Stop
)
if ($assetCandidates.Count -ne 1) {
  throw "Expected exactly one built MaxGameStudio logo PNG (found $($assetCandidates.Count)) under $rootPath"
}
$assetName = Assert-RendererLogoPng $assetCandidates[0].FullName
Assert-BuiltReferences $rootPath $assetName
Write-Host "League runtime built assets verified: $rootPath"
