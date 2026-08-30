from __future__ import annotations

from pathlib import Path


WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "release-windows.yml"


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
    manifest = _step_block(workflow, "Generate signed updater manifest")
    assert "USER_RELEASE_NOTES_PATH:" in manifest

