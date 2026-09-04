from __future__ import annotations

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release-windows.yml"


def _step_block(workflow: str, step_name: str) -> str:
    marker = f"      - name: {step_name}\n"
    start = workflow.index(marker)
    next_step = workflow.find("\n      - name:", start + len(marker))
    return workflow[start:] if next_step < 0 else workflow[start:next_step]


def test_formal_release_steps_require_push_and_strict_stable_version():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    expected_gate = (
        "if: github.event_name == 'push' && github.ref_type == 'tag' "
        "&& steps.release_version.outputs.stable == 'true'"
    )

    for step_name in (
        "Resolve GitHub Release notes",
        "Generate signed updater manifest",
        "Create GitHub Release",
        "Publish updater channel",
    ):
        assert expected_gate in _step_block(workflow, step_name)

    assert "$stable = $version -match '^\\d+\\.\\d+\\.\\d+$'" in workflow
    assert '"stable=$($stable.ToString().ToLowerInvariant())"' in workflow


def test_manual_dispatch_only_uploads_private_artifact():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    private_artifact = _step_block(workflow, "Upload private release candidate")
    assert "if: github.event_name == 'workflow_dispatch'" in private_artifact
    assert "actions/upload-artifact@v4" in private_artifact


def test_release_title_is_only_the_version_tag():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    release = _step_block(workflow, "Create GitHub Release")
    assert "name: ${{ steps.release_version.outputs.tag }}" in release
    assert "name: MaxGameStudio ${{ steps.release_version.outputs.tag }}" not in release
    assert "body_path: ${{ steps.release_notes.outputs.path }}" in release
    assert "generate_release_notes: true" not in release


def test_stable_release_requires_a_versioned_release_notes_file():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    notes = _step_block(workflow, "Resolve GitHub Release notes")
    assert 'docs/releases/v$env:BUILD_VERSION.zh-CN.md' in notes
    assert "Stable Release notes are required" in notes
    assert 'docs/update-notes/v$env:BUILD_VERSION.json' in notes
    assert "Plain-language update notes are required" in notes
    assert 'releaseStatus -ne "stable"' in notes
    assert "Stable tags require releaseStatus=stable" in notes
    manifest = _step_block(workflow, "Generate signed updater manifest")
    assert "USER_RELEASE_NOTES_PATH:" in manifest


def test_windows_signing_covers_the_elevated_league_runtime_before_tauri_build():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    certificate_step = _step_block(workflow, "Import Windows code-signing certificate")
    artifact_step = _step_block(workflow, "Write SHA256SUMS")

    sign_runtime = 'Set-AuthenticodeSignature `\n              -FilePath $runtimeExe'
    refresh_hashes = '$runtimeHashes[$relative] = (Get-FileHash'
    write_manifest = 'maxgamestudio-runtime-hashes.json'
    assert certificate_step.index(sign_runtime) < certificate_step.index(refresh_hashes)
    assert certificate_step.index(refresh_hashes) < certificate_step.rindex(write_manifest)
    assert workflow.index("Import Windows code-signing certificate") < workflow.index(
        "Build Tauri desktop installer"
    )
    assert "CS2_INSIGHT_WINDOWS_CERTIFICATE_THUMBPRINT" in certificate_step
    assert "MaxGameStudioLeague.exe" in artifact_step
    assert "foreach ($artifact in @($appExe, $installer, $leagueRuntimeExe))" in artifact_step


def test_versioned_desktop_build_requires_an_embedded_league_runtime_manifest():
    wrapper = (REPO_ROOT / "frontend" / "scripts" / "tauri-build-version.mjs").read_text(
        encoding="utf-8"
    )
    build_script = (REPO_ROOT / "frontend" / "src-tauri" / "build.rs").read_text(
        encoding="utf-8"
    )

    assert 'MAXGAMESTUDIO_REQUIRE_LEAGUE_RUNTIME_MANIFEST: "1"' in wrapper
    assert "versioned desktop builds require the staged League runtime hash manifest" in build_script
    assert "versioned desktop builds require a non-empty League runtime hash manifest" in build_script


def test_league_runtime_staging_rejects_unsafe_manifest_paths_and_links():
    staging = (REPO_ROOT / "frontend" / "scripts" / "stage-tauri-resources.mjs").read_text(
        encoding="utf-8"
    )

    assert "contains an unsafe path" in staging
    assert 'part === ".."' in staging
    assert "lstatSync(currentPath).isSymbolicLink()" in staging
    assert "payload escaped its cache root" in staging


def test_authenticode_is_optional_but_tauri_updater_signing_remains_required():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    certificate_step = _step_block(workflow, "Import Windows code-signing certificate")
    build_step = _step_block(workflow, "Build Tauri desktop installer")

    assert "FORMAL_STABLE_RELEASE" not in certificate_step
    assert "Stable tagged releases require WINDOWS_PFX_BASE64" not in certificate_step
    assert 'if (-not $env:WINDOWS_PFX_BASE64)' in certificate_step
    assert "producing an installer without Authenticode" in certificate_step
    assert "TAURI_SIGNING_PRIVATE_KEY:" in build_step
    assert 'if (-not $env:TAURI_SIGNING_PRIVATE_KEY) { throw' in build_step
    assert "*.exe.sig" in build_step

