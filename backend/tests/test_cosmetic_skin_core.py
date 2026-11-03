import hashlib
import json
import os
import threading
from pathlib import Path

import pytest
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

from app.cosmetic_studio import skin_core


def test_materialized_usp_has_a_separate_exact_zero_identity_contract():
    item = skin_core.CoreMaterializedUspItem(paint_kit=504, wear=.01, pattern_seed=0)
    profile = skin_core.CoreProfile(steam_id64="76561199849752808", items=(item,))
    assert profile.payload()["items"] == [{"item_id64": "0", "definition_index": 61,
        "paint_kit": 504, "pattern_seed": 0, "wear": .01, "team": "CT"}]
    with pytest.raises(ValueError):
        skin_core.CoreItem(item_id64="0", definition_index=61, paint_kit=504)


@pytest.mark.parametrize("changes", [{"item_id64": "1"}, {"team": "T"},
    {"definition_index": 9}, {"definition_index": 61.0}, {"paint_kit": 137},
    {"replacement_definition_index": 515}, {"wear": float("nan")}, {"pattern_seed": True}])
def test_materialized_usp_rejects_unqualified_identity_side_model_or_finish(changes):
    with pytest.raises(ValueError):
        skin_core.CoreMaterializedUspItem(**{"paint_kit": 504, "wear": .01, "pattern_seed": 0, **changes})


def test_materialized_reply_requires_matching_ct_scope(tmp_path):
    output = tmp_path / "copy.dem"
    output.write_bytes(b"PBDEMS2\x00" + b"fixture bytes")
    profile = skin_core.CoreProfile(steam_id64="76561199849752808",
        items=(skin_core.CoreMaterializedUspItem(paint_kit=504),))
    reply = {"schema_version": 1, "ok": True, "items_rewritten": 1,
        "succeeded": [{"item_id64": "0", "definition_index": 61, "team": "CT"}],
        "sha256": hashlib.sha256(output.read_bytes()).hexdigest()}
    assert skin_core.validate_reply(reply, profile, output) == reply["sha256"]
    reply["succeeded"][0]["team"] = "T"
    with pytest.raises(CosmeticError, match="TEAM_MISMATCH"):
        skin_core.validate_reply(reply, profile, output)
from app.cosmetic_studio.core_wire import RESPONSE_MAGIC
from app.cosmetic_studio.provider import CosmeticError


TARGET = "76561199059074958"


def profile():
    return skin_core.CoreProfile.model_validate({
        "steam_id64": TARGET,
        "items": ({"item_id64": "52256425842", "definition_index": 9, "paint_kit": 344,
                   "pattern_seed": 0, "wear": 0.01},),
    })


def setup_provider(tmp_path, monkeypatch, *, reply=None, verifier=lambda *_: True, mutate=None,
                   preview_check=None, stop_playback=None, transport_code=0, write_response=True):
    tmp_path.mkdir(parents=True, exist_ok=True)
    # Keep the fake-only fixture independent of whichever drive pytest uses.
    tmp_drive = Path(tmp_path).drive.upper()
    monkeypatch.setenv("SystemDrive", "Z:" if tmp_drive != "Z:" else "Y:")
    original = tmp_path / "source.dem"
    original.write_bytes(b"PBDEMS2\x00" + b"source")
    executable = tmp_path / "skin-core.exe"
    executable.write_bytes(b"fake core")
    python = tmp_path / "python.exe"
    python.write_bytes(b"fake python")
    monkeypatch.setattr(skin_core, "QUALIFIED_CORE_SHA256", hashlib.sha256(executable.read_bytes()).hexdigest())
    root = tmp_path / "job"

    def transport(_exe, args, packet, work_root, _env, cancel):
        if mutate:
            mutate(work_root, args)
        if transport_code:
            return transport_code, "CORE_PARENT_NOT_SUPPORTED"
        output = Path(args[args.index("--output") + 1])
        response_path = Path(args[args.index("--response") + 1])
        output.write_bytes(b"PBDEMS2\x00rewritten")
        result = reply or {"schema_version": 1, "ok": True, "items_rewritten": 1,
                           "succeeded": [{"item_id64": "52256425842", "definition_index": 9}],
                           "failed": [], "sha256": hashlib.sha256(output.read_bytes()).hexdigest()}
        key = packet[-32:]
        plain = json.dumps(result, separators=(",", ":")).encode()
        nonce = b"Z" * 12
        aad = RESPONSE_MAGIC + b"\x01" + str(Path(args[args.index("--input") + 1])).encode() + str(output).encode()
        if write_response:
            response_path.write_bytes(RESPONSE_MAGIC + b"\x01" + nonce + AESGCM(key).encrypt(nonce, plain, aad))
        return 0, "ok"

    p = skin_core.SkinCoreProvider(original, root, profile(), executable, python,
                                    threading.Event(), verifier=verifier, transport=transport,
                                    preview_check=preview_check, stop_playback=stop_playback)
    return p, original, root


def test_core_profile_rejects_duplicate_and_non_uint64_item_ids():
    with pytest.raises(ValueError):
        skin_core.CoreProfile.model_validate({"steam_id64": TARGET, "items": (
            {"item_id64": "1", "paint_kit": 344}, {"item_id64": "1", "paint_kit": 344})})
    for value in ("1.0", "-1", "18446744073709551616", True):
        with pytest.raises(ValueError):
            skin_core.CoreProfile.model_validate({"steam_id64": TARGET, "items": ({"item_id64": value, "paint_kit": 344},)})


def test_cross_model_core_reply_reports_the_source_definition(tmp_path):
    output = tmp_path / 'butterfly.dem'
    output.write_bytes(b'PBDEMS2\x00' + b'butterfly')
    item = skin_core.CoreItem(item_id64='48742431476', definition_index=507,
        replacement_definition_index=515, paint_kit=38, pattern_seed=0, wear=.01)
    cross_profile = skin_core.CoreProfile(steam_id64='76561199369875913', items=(item,))
    output_hash = hashlib.sha256(output.read_bytes()).hexdigest()
    reply = {'schema_version': 1, 'ok': True, 'items_rewritten': 1,
        'succeeded': [{'item_id64': '48742431476', 'definition_index': 507}],
        'failed': [], 'sha256': output_hash}

    assert skin_core.validate_reply(reply, cross_profile, output) == output_hash
    reply['succeeded'][0]['definition_index'] = 515
    with pytest.raises(CosmeticError, match='CORE_TARGET_RESULT_MISMATCH'):
        skin_core.validate_reply(reply, cross_profile, output)


def test_authenticated_file_success_is_not_visual_ready_until_preview(tmp_path, monkeypatch):
    p, _, _ = setup_provider(tmp_path, monkeypatch, preview_check=lambda _artifact: True)
    p.load()
    p.apply_profile(p.profile.sha256())
    assert p.state == "pending_visual_review"
    assert p.check_ready() is False
    p.confirm_preview()
    assert p.state == "human_confirmed"
    assert p.check_ready() is True


def test_bad_reply_partial_and_target_mismatch_fail_closed(tmp_path, monkeypatch):
    for reply in (
        {"schema_version": 1, "ok": True, "items_rewritten": 0, "succeeded": [], "failed": [], "sha256": "0" * 64},
        {"schema_version": 1, "ok": True, "items_rewritten": 1, "succeeded": [], "failed": [{"error": "bad"}], "sha256": "0" * 64},
        {"schema_version": 1, "ok": True, "items_rewritten": 1, "succeeded": [{"item_id64": "1", "definition_index": 9}], "failed": [], "sha256": "0" * 64},
    ):
        case_dir = tmp_path / f"case-{len(reply.get('succeeded', []))}-{reply.get('items_rewritten')}"
        p, _, _ = setup_provider(case_dir, monkeypatch, reply=reply)
        p.load()
        with pytest.raises(CosmeticError):
            p.apply_profile(p.profile.sha256())
        assert p.state == "failed"


def test_output_hash_and_demo_validation_are_required(tmp_path, monkeypatch):
    reply = {"schema_version": 1, "ok": True, "items_rewritten": 1,
             "succeeded": [{"item_id64": "52256425842", "definition_index": 9}], "failed": [], "sha256": "a" * 64}
    p, _, _ = setup_provider(tmp_path, monkeypatch, reply=reply)
    p.load()
    with pytest.raises(CosmeticError, match="CORE_OUTPUT_DIGEST_MISMATCH"):
        p.apply_profile(p.profile.sha256())


def test_missing_response_and_parent_rejection_fail_closed(tmp_path, monkeypatch):
    p, _, _ = setup_provider(tmp_path / "missing", monkeypatch, write_response=False)
    p.load()
    with pytest.raises(CosmeticError, match="CORE_RESPONSE_MISSING"):
        p.apply_profile(p.profile.sha256())
    p2, _, _ = setup_provider(tmp_path / "parent", monkeypatch, transport_code=2)
    p2.load()
    with pytest.raises(CosmeticError, match="CORE_PARENT_NOT_SUPPORTED"):
        p2.apply_profile(p2.profile.sha256())


def test_unqualified_core_and_reused_output_directory_are_rejected(tmp_path, monkeypatch):
    p, _, root = setup_provider(tmp_path / "version", monkeypatch)
    monkeypatch.setattr(skin_core, "QUALIFIED_CORE_SHA256", "0" * 64)
    with pytest.raises(CosmeticError, match="CORE_VERSION_UNQUALIFIED"):
        p.load()
    p2, _, root2 = setup_provider(tmp_path / "existing", monkeypatch)
    root2.mkdir()
    with pytest.raises(FileExistsError):
        p2.load()


def test_stale_preview_cannot_be_confirmed(tmp_path, monkeypatch):
    p, _, _ = setup_provider(tmp_path, monkeypatch, preview_check=lambda _artifact: False)
    p.load()
    p.apply_profile(p.profile.sha256())
    with pytest.raises(CosmeticError, match="CORE_VISUAL_REVIEW_STALE"):
        p.confirm_preview()


def test_source_or_input_mutation_fails_before_verifier(tmp_path, monkeypatch):
    def mutate(work_root, _args):
        (work_root / "input.dem").write_bytes(b"PBDEMS2\x00tampered")
    p, _, _ = setup_provider(tmp_path, monkeypatch, mutate=mutate)
    p.load()
    with pytest.raises(CosmeticError, match="CORE_SOURCE_OR_INPUT_CHANGED"):
        p.apply_profile(p.profile.sha256())


def test_cancel_and_profile_binding_are_hard_failures(tmp_path, monkeypatch):
    p, _, _ = setup_provider(tmp_path, monkeypatch)
    p.cancel.set()
    with pytest.raises(CosmeticError, match="CORE_CANCELLED"):
        p.load()
    p2, _, _ = setup_provider(tmp_path / "second", monkeypatch)
    p2.load()
    with pytest.raises(CosmeticError, match="CORE_PROFILE_NOT_BOUND"):
        p2.apply_profile("0" * 64)


def test_environment_isolated_from_development_flags(tmp_path, monkeypatch):
    monkeypatch.setenv("CS2_SKIN_CORE_DEV", "1")
    monkeypatch.setenv("OPENAI_API_KEY", "secret-must-not-pass")
    env = skin_core.core_environment(tmp_path, tmp_path / "python.exe", tmp_path / "skin-core.exe")
    assert "CS2_SKIN_CORE_DEV" not in env
    assert "OPENAI_API_KEY" not in env
    assert env["APPDATA"] == str(tmp_path / "profile")


def test_unload_does_not_accept_false_cleanup_ack(tmp_path, monkeypatch):
    p, _, _ = setup_provider(tmp_path, monkeypatch, stop_playback=lambda: False)
    p.load()
    with pytest.raises(CosmeticError, match="CORE_PLAYBACK_STOP_UNCONFIRMED"):
        p.unload()
    assert p.state != "stopped", "false cleanup acknowledgement must not be reported stopped"


def test_system_drive_output_is_rejected_independently(tmp_path, monkeypatch):
    p, _, _ = setup_provider(tmp_path, monkeypatch)
    monkeypatch.setenv("SystemDrive", Path(tmp_path).drive)
    with pytest.raises(CosmeticError, match="CORE_OUTPUT_LOCATION_INVALID"):
        p.load()
