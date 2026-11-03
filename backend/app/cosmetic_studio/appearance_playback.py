"""Explicit playback of a server-owned, independently verified appearance copy.

No DLL loading, process attachment, online launch or visual-success inference.
"""
from pathlib import Path
import re
from .runtime import digest


def launch_verified_appearance(artifact, config, playback, *, offline_acknowledged):
    if offline_acknowledged is not True or not isinstance(artifact, dict) or artifact.get('state') != 'file_verified':
        raise ValueError('请先完成副本校验并明确确认专用离线回放。')
    paths = {}
    for name, hash_name in (('original_path', 'original_sha256'), ('playback_path', 'playback_sha256')):
        expected = artifact.get(hash_name, '')
        if not isinstance(expected, str) or not re.fullmatch(r'[0-9a-f]{64}', expected):
            raise ValueError('副本校验记录不完整，不能启动。')
        path = Path(artifact.get(name, ''))
        if not path.is_absolute() or path.suffix.lower() != '.dem' or not path.is_file() or digest(path) != expected:
            raise ValueError('原始 Demo 或制片副本已变化，请重新读取并应用外观。')
        paths[name] = path
    result = playback.launch(paths['playback_path'], config, verified_demo_sha256=artifact['playback_sha256'])
    return {**result, 'visual_verified': None, 'appearance_job_state': 'file_verified'}
