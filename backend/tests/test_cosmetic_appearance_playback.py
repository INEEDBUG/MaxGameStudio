from pathlib import Path
from types import SimpleNamespace
import hashlib
import pytest

from app.cosmetic_studio.appearance_playback import launch_verified_appearance


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def artifact(tmp_path):
    original, copy = tmp_path / 'source.dem', tmp_path / 'copy.dem'
    original.write_bytes(b'original'); copy.write_bytes(b'appearance')
    return dict(original_path=str(original), playback_path=str(copy), original_sha256=sha(original),
                playback_sha256=sha(copy), profile_sha256='a' * 64, state='file_verified', visual_verified=None)


class Playback:
    def __init__(self):
        self.calls = []

    def launch(self, path, cfg, *, verified_demo_sha256):
        self.calls.append((path, cfg, verified_demo_sha256))
        return {'session_id': 'session', 'state': 'running'}


def test_only_explicit_offline_action_launches_the_verified_copy(tmp_path):
    proof = artifact(tmp_path)
    player = Playback(); cfg = SimpleNamespace(cs2_path='offline-cs2')
    result = launch_verified_appearance(proof, cfg, player, offline_acknowledged=True)
    assert result['session_id'] == 'session'
    assert player.calls == [(Path(proof['playback_path']), cfg, proof['playback_sha256'])]
    assert result['visual_verified'] is None


@pytest.mark.parametrize('changed', ['original_path', 'playback_path'])
def test_changed_source_or_copy_stops_before_native_launch(tmp_path, changed):
    proof = artifact(tmp_path); Path(proof[changed]).write_bytes(b'changed')
    player = Playback()
    with pytest.raises(ValueError, match='变化'):
        launch_verified_appearance(proof, object(), player, offline_acknowledged=True)
    assert not player.calls


def test_unknown_or_unacknowledged_state_is_not_a_launch_request(tmp_path):
    proof = artifact(tmp_path); player = Playback()
    with pytest.raises(ValueError):
        launch_verified_appearance(proof, object(), player, offline_acknowledged=False)
    proof['state'] = 'pending'
    with pytest.raises(ValueError):
        launch_verified_appearance(proof, object(), player, offline_acknowledged=True)
    assert not player.calls
