param(
  [Parameter(Mandatory)][ValidateSet('Main', 'League')][string]$Product,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+-beta\.\d+$')][string]$Version,
  [string]$PredecessorCatalog
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:GITHUB_ACTIONS -ne 'true') {
  throw 'Candidate packaging is restricted to the hosted verification job.'
}
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
if ((git -C $root rev-parse HEAD).Trim() -cne $env:GITHUB_SHA) { throw 'Unexpected candidate checkout.' }
$temp = (Resolve-Path -LiteralPath $env:RUNNER_TEMP).Path
$out = Join-Path $temp 'mgs-native-candidates'
$null = [IO.Directory]::CreateDirectory($out)
$priorHash = if ($PredecessorCatalog) {
  (Get-FileHash -LiteralPath $PredecessorCatalog -Algorithm SHA256).Hash.ToLowerInvariant()
}

Push-Location $root
try {
  if ($Product -eq 'Main') {
    $name = "MaxGameStudio_${Version}_x64-setup.exe"
    if (Test-Path -LiteralPath (Join-Path $out $name)) { throw 'The candidate already exists.' }
    $env:MGS_MAIN_NATIVE_PACKAGE = '1'
    $env:MGS_MAIN_NATIVE_CATALOG = (Resolve-Path -LiteralPath 'packaging/windows/native-legacy/main-3.1.4-repair.json').Path
    $env:MGS_MAIN_NATIVE_CATALOG_SHA256 = 'c33c1af740dcb75dc97f630c21bd709a426c65ea0f23029ed9ddfef7ad8896fd'
    $env:MGS_MAIN_NATIVE_PRIOR_CATALOG = $PredecessorCatalog
    $env:MGS_MAIN_NATIVE_PRIOR_CATALOG_SHA256 = $priorHash
    $env:CARGO_TARGET_DIR = Join-Path $root 'frontend/src-tauri/target'
    $parserSource = Join-Path $temp 'mgs-parser-source'
    $parserStage = Join-Path $parserSource 'rust/maxgamestudio-parser-worker'
    $parser = Join-Path $env:CARGO_TARGET_DIR 'release/maxgamestudio-parser-worker.exe'
    $parserProof = Join-Path $temp 'mgs-native-parser-sha256.txt'
    if (!(Test-Path -LiteralPath $parserProof -PathType Leaf)) {
      $worker = ./packaging/demoparser-lean/stage-native-worker.ps1 -SourceRoot $parserSource
      cargo build --release --locked --manifest-path $worker.Manifest
      (Get-FileHash -LiteralPath $parser).Hash.ToLowerInvariant() |
        Set-Content -LiteralPath $parserProof -Encoding ascii
    }
    if ((Get-FileHash -LiteralPath $parser).Hash -ine (Get-Content -LiteralPath $parserProof -Raw).Trim()) {
      throw 'The staged parser changed between candidate builds.'
    }
    $env:MGS_MAIN_NATIVE_PARSER_EXE = $parser
    $env:MGS_MAIN_NATIVE_PARSER_NOTICE = Join-Path $parserStage 'LICENSE.upstream.txt'
    Push-Location frontend
    try {
      if (!(Test-Path -LiteralPath 'node_modules/.pnpm' -PathType Container)) {
        pnpm.cmd install --frozen-lockfile
      }
      pnpm.cmd run desktop:build:ver -- $Version
    } finally { Pop-Location }
    $setup = Join-Path $env:CARGO_TARGET_DIR "release/bundle/nsis/$name"
    Copy-Item -LiteralPath $setup -Destination $out
    if (Test-Path -LiteralPath "$setup.sig") { Copy-Item -LiteralPath "$setup.sig" -Destination $out }
    return
  }

  $source = Join-Path $temp 'akari-source'
  if (!(Test-Path -LiteralPath $source)) {
    git init --quiet $source
    git -c core.longpaths=true -C $source fetch --quiet --depth 1 https://github.com/LeagueAkari/LeagueAkari.git $env:LEAGUE_PIN
    git -c core.longpaths=true -C $source checkout --quiet --detach FETCH_HEAD
    Push-Location $source
    try { node .yarn/releases/yarn-4.9.1.cjs install --immutable } finally { Pop-Location }
  }
  if ((git -C $source rev-parse HEAD).Trim() -cne $env:LEAGUE_PIN) { throw 'Unexpected League renderer checkout.' }
  # Tauri must resolve the asset directory relative to its own config.
  $rendererPath = "dist-$Version"
  $renderer = @{
    SourceDir=$source; Standalone=$true; Build=$true; NativeRendererOnly=$true
    StandaloneVersion=$Version; OutputDir=(Join-Path $root "rust/league-desktop/$rendererPath")
  }
  ./packaging/windows/stage-league-runtime.ps1 @renderer
  $env:TAURI_CONFIG = @{
    version=$Version; build=@{frontendDist=$rendererPath}; bundle=@{icon=@("$rendererPath/league.ico")}
  } | ConvertTo-Json -Depth 4 -Compress
  cargo build --release --locked --manifest-path rust/league-desktop/Cargo.toml --target x86_64-pc-windows-msvc
  $cargoRelease = 'rust/league-desktop/target/x86_64-pc-windows-msvc/release'
  $payload = Join-Path $temp "league-native-payload-$Version"
  $null = New-Item -ItemType Directory -Path $payload
  Copy-Item -LiteralPath (Join-Path $cargoRelease 'maxgamestudio-league-desktop.exe') -Destination (Join-Path $payload 'MaxGameStudioLeagueToolbox.exe')
  $loader = Join-Path $cargoRelease 'WebView2Loader.dll'
  if (Test-Path -LiteralPath $loader) { Copy-Item -LiteralPath $loader -Destination $payload }
  $bootstrapper = Join-Path $temp 'MicrosoftEdgeWebview2Setup.exe'
  if (!(Test-Path -LiteralPath $bootstrapper)) {
    Invoke-WebRequest -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $bootstrapper -TimeoutSec 60
  }
  $catalog = 'packaging/windows/native-legacy'
  $options = @{
    SourceDir=$source; UnpackedDir=$payload; Version=$Version; Native=$true
    InstallerOutputDir=$out; WebView2Bootstrapper=$bootstrapper
    LegacyInventory=@((Join-Path $catalog 'trusted-1.0.0-beta.5.json'), (Join-Path $catalog 'trusted-1.0.0-beta.8.json'))
    LegacyInventorySHA256=@(
      '85c33b257b4bcb4faa77ea8bbe947cd69deadbffdda0618711e0cc5177d50bc9',
      '358d78e5417cadeff0153539e2f99cbe8b0be5a6123c0b58e1182005334aac7b'
    )
  }
  if ($PredecessorCatalog) {
    $options.LegacyInventory += $PredecessorCatalog
    $options.LegacyInventorySHA256 += $priorHash
  }
  ./packaging/windows/build-league-installer.ps1 @options
} finally { Pop-Location }
