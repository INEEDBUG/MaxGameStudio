from __future__ import annotations

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release-windows.yml"


def _step_block(workflow: str, step_name: str) -> str:
    marker = f"      - name: {step_name}\n"
    start = workflow.index(marker)
    next_step = workflow.find("\n      - name:", start + len(marker))
    return workflow[start:] if next_step < 0 else workflow[start:next_step]


def test_main_release_publication_requires_a_tag_and_separates_beta_from_stable():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    expected_gate = (
        "if: github.event_name == 'push' && github.ref_type == 'tag'"
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

    release = _step_block(workflow, "Create GitHub Release")
    assert "prerelease: ${{ steps.release_version.outputs.stable != 'true' }}" in release
    assert "make_latest: ${{ steps.release_version.outputs.stable == 'true' && 'true' || 'false' }}" in release
    channel = _step_block(workflow, "Publish updater channel")
    assert '$manifestName = if ($env:BUILD_VERSION.Contains("-")) { "beta.json" } else { "latest.json" }' in channel
    assert 'contents/$manifestName' in channel


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


def test_release_requires_versioned_notes_with_the_matching_channel_status():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    notes = _step_block(workflow, "Resolve GitHub Release notes")
    assert 'docs/releases/v$env:BUILD_VERSION.zh-CN.md' in notes
    assert "Stable Release notes are required" in notes
    assert 'docs/update-notes/v$env:BUILD_VERSION.json' in notes
    assert "Plain-language update notes are required" in notes
    assert '$expectedStatus = if ($env:BUILD_VERSION.Contains("-")) { "beta" } else { "stable" }' in notes
    assert 'releaseStatus -ne $expectedStatus' in notes
    manifest = _step_block(workflow, "Generate signed updater manifest")
    assert "USER_RELEASE_NOTES_PATH:" in manifest


def test_main_signing_and_build_do_not_depend_on_the_separate_league_payload():
    workflow = WORKFLOW.read_text(encoding="utf-8")
    certificate_step = _step_block(workflow, "Import Windows code-signing certificate")
    artifact_step = _step_block(workflow, "Write SHA256SUMS")
    assert "CS2_INSIGHT_WINDOWS_CERTIFICATE_THUMBPRINT" in certificate_step
    assert "foreach ($artifact in @($appExe, $installer))" in artifact_step
    assert "Build pinned MaxGameStudio League runtime" not in workflow
    assert "league-runtime/MaxGameStudioLeague.exe" not in workflow


def test_league_release_cannot_replace_main_latest_or_main_update_channels():
    workflow = (WORKFLOW.parent / "release-league-windows.yml").read_text(encoding="utf-8")
    assert "- 'league-v*'" in workflow
    assert "--latest=false" in workflow
    assert "league-beta.json" in workflow
    assert "league-latest.json" in workflow
    assert "contents/latest.json" not in workflow
    assert "contents/beta.json" not in workflow


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

