import threading

import pytest

from app.cosmetic_studio.models import ConfirmRequest, JobRequest, ToolPaths
from app.cosmetic_studio.provider import CosmeticError, NskinzProvider


def tools(**overrides):
    values = {
        "cs2_exe": r"C:\Tools\cs2.exe",
        "hlae_exe": r"C:\Tools\hlae.exe",
        "nskinz_dll": r"C:\Tools\nskinz.dll",
        "obs_exe": r"C:\Tools\obs64.exe",
        "ffmpeg_exe": r"C:\Tools\ffmpeg.exe",
        "ffprobe_exe": r"C:\Tools\ffprobe.exe",
        "output_dir": r"C:\MGS\output",
    }
    values.update(overrides)
    return ToolPaths(**values)


def request(**overrides):
    values = {
        "demo_path": r"C:\Demos\match.dem",
        "xuid": "76561199059074958",
        "start_tick": 256,
        "end_tick": 384,
        "preset_id": "awp-dragon-lore",
        "tools": tools(),
        "acknowledge_offline": True,
    }
    values.update(overrides)
    return JobRequest(**values)


@pytest.mark.parametrize("value", [1, "true", "True", 1.0])
def test_acknowledgements_require_boolean_true(value):
    with pytest.raises(Exception):
        request(acknowledge_offline=value)
    with pytest.raises(Exception):
        ConfirmRequest(preview_id="a" * 32, appearance_confirmed=value)


def test_xuid_is_exact_decimal_unsigned_64_bit_string():
    assert request(xuid="18446744073709551615").xuid == "18446744073709551615"
    for value in ["0", "01", "76561199059074958 ", "18446744073709551616", "+1", "abc"]:
        with pytest.raises(Exception):
            request(xuid=value)


@pytest.mark.parametrize("path", [r"\\server\share\x.exe", r"relative.exe", "C:\\x\\bad\nline.exe", r'C:\x\"quoted.exe'])
def test_tool_paths_reject_unsafe_or_nonlocal_paths(path):
    with pytest.raises(Exception):
        tools(cs2_exe=path)


@pytest.mark.parametrize("start,end", [(256, 383), (256, 2177)])
def test_intervals_outside_initial_experiment_are_rejected(start, end):
    with pytest.raises(Exception):
        request(start_tick=start, end_tick=end)


def test_interval_boundaries_are_accepted():
    assert request(start_tick=256, end_tick=384).end_tick == 384
    assert request(start_tick=256, end_tick=2176).end_tick == 2176


class Transport:
    def __init__(self, stop_error=None):
        self.calls = []
        self.stop_error = stop_error

    def validate_context(self, *, paused):
        self.calls.append(("validate_context", paused))

    def load_extension(self):
        self.calls.append(("load_extension",))

    def request_refresh(self, profile_digest):
        self.calls.append(("request_refresh", profile_digest))

    def stop(self):
        self.calls.append(("stop",))
        if self.stop_error:
            raise self.stop_error


def test_submission_is_not_visual_success():
    transport = Transport()
    provider = NskinzProvider(transport)
    provider.load()
    provider.apply_profile("sha256:fixture")
    assert provider.get_status().visual_verified is None
    assert provider.check_ready() is False
    assert ("request_refresh", "sha256:fixture") in transport.calls


def test_provider_rejects_replay_and_invalid_state_transitions():
    transport = Transport()
    provider = NskinzProvider(transport)
    with pytest.raises(CosmeticError, match="PROVIDER_NOT_LOADED"):
        provider.apply_profile("digest")
    with pytest.raises(CosmeticError, match="VISUAL_REVIEW_NOT_PENDING"):
        provider.confirm_preview()
    provider.load()
    with pytest.raises(CosmeticError, match="PROVIDER_ALREADY_USED"):
        provider.load()
    provider.apply_profile("digest")
    provider.confirm_preview()
    with pytest.raises(CosmeticError, match="PROVIDER_NOT_LOADED"):
        provider.apply_profile("replay")


def test_unload_propagates_owned_stop_failure_and_does_not_claim_success():
    transport = Transport(CosmeticError("PROCESS_EXIT_UNCONFIRMED"))
    provider = NskinzProvider(transport)
    provider.load()
    with pytest.raises(CosmeticError, match="PROCESS_EXIT_UNCONFIRMED"):
        provider.unload()
    assert provider.get_status().state == "stopping"
    assert provider.get_status().visual_verified is None


def test_guarded_handle_close_waits_for_inflight_read():
    from app.cosmetic_studio import native

    entered = threading.Event()
    release = threading.Event()
    closed = []

    def read(handle):
        entered.set()
        assert release.wait(1)
        return handle

    wrapped = native.GuardedHandle("owned", lambda handle: closed.append(handle) or True)
    reader = threading.Thread(target=lambda: wrapped.call(read))
    reader.start()
    assert entered.wait(1)
    closer = threading.Thread(target=wrapped.close)
    closer.start()
    assert not closed
    release.set()
    reader.join(1)
    closer.join(1)
    assert closed == ["owned"]
    with pytest.raises(native.ClosedHandleError):
        wrapped.call(read)
