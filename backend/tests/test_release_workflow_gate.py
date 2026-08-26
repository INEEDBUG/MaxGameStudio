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
    expected_gate = "if: github.event_name == 'push' && steps.release_version.outputs.stable == 'true'"

    for step_name in (
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

