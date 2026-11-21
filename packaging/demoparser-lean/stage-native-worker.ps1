#Requires -Version 5.1

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceRoot
)

$ErrorActionPreference = "Stop"
$source = & (Join-Path $PSScriptRoot "prepare-native-source.ps1") -SourceRoot $SourceRoot
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../..")).ProviderPath
$worker = "rust/maxgamestudio-parser-worker"
$inputRoot = Join-Path $repo $worker
$stageRoot = Join-Path $source $worker

$files = @(
    "Cargo.toml", "Cargo.lock", "src/main.rs", "src/demo_inspection.rs", "src/replay.rs",
    "src/replay_effect_rows.rs", "src/replay_io.rs", "src/round_windows.rs", "tests/ipc.rs",
    "src/analysis_events.rs", "src/match_timeline.rs", "src/match_timeline/match_facts.rs",
    "src/match_timeline/player_stats.rs", "src/match_timeline/damage_stats.rs",
    "src/match_timeline/round_score.rs", "src/replay_effect_rows/inferno_tracks.rs",
    "src/replay_effect_rows/smoke_tracks.rs", "src/replay_effect_rows/smoke_formation.rs",
    "src/replay_effect_rows/trajectory_paths.rs"
)
foreach ($relative in $files) {
    $destination = Join-Path $stageRoot $relative
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination)) | Out-Null
    Copy-Item -LiteralPath (Join-Path $inputRoot $relative) -Destination $destination -Force
}
foreach ($shared in @("rust/shared/case_fold.rs", "rust/shared/demo_boundary.rs")) {
    $destination = Join-Path $source $shared
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination)) | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo $shared) -Destination $destination -Force
}

# The verified release patch already contains the Rust smoke geometry decoder.
# Stage that implementation without the Python binding or a second decoder.
$wheelPatch = [IO.File]::ReadAllLines((Join-Path $PSScriptRoot "demoparser2-v0.41.4.patch"))
$projectionStart = [Array]::IndexOf($wheelPatch, '+const SMOKE_GRID_DIM: usize = 32;')
$projectionEnd = [Array]::IndexOf($wheelPatch, '+fn native_columns(')
if ($projectionStart -lt 0 -or $projectionEnd -le $projectionStart) {
    throw "The pinned wheel patch does not contain the native smoke projection."
}
$projection = foreach ($line in $wheelPatch[$projectionStart..($projectionEnd - 1)]) {
    if (-not $line.StartsWith('+')) { throw "Unexpected smoke projection patch line." }
    $line.Substring(1)
}
# Native replay consumes sequence numbers and projected cells. The wheel also reports diagnostics.
$projection = $projection | Where-Object { $_ -cnotin @(
    '    phase: u8,', '    voxel_count: usize,', '        let phase = blob[0];',
    '                phase,', '                voxel_count,'
) }
$projectionCode = ($projection -join [Environment]::NewLine).
    Replace('struct SmokeProjectionSample {', 'pub(super) struct SmokeProjectionSample {').
    Replace('    seq: u16,', '    pub(super) seq: u16,').
    Replace('    cells: Vec<[f64; 4]>,', '    pub(super) cells: Vec<[f64; 4]>,').
    Replace('fn decode_smoke_projection_samples(', 'pub(super) fn decode_smoke_projection_samples(').
    Replace('let (cells, voxel_count) = smoke_project(', 'let (cells, _) = smoke_project(').
    Replace('0 * SMOKE_GRID_DIM + y', 'y')
$projectionFile = Join-Path $stageRoot "src/replay_effect_rows/smoke_projection.generated.rs"
$projectionHeader = "// Staged from the pinned demoparser v0.41.4 wheel patch. See LICENSE.upstream.txt." + [Environment]::NewLine + "use std::collections::{HashMap, VecDeque};" + [Environment]::NewLine
[IO.File]::WriteAllText($projectionFile, $projectionHeader + $projectionCode, [Text.UTF8Encoding]::new($false))

# The original upstream notice travels beside any distributed worker.
$notice = Join-Path $stageRoot "LICENSE.upstream.txt"
Copy-Item -LiteralPath (Join-Path $source "LICENSE") -Destination $notice -Force

[pscustomobject]@{
    Manifest = Join-Path $stageRoot "Cargo.toml"
    Notice = $notice
}
