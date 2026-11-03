"""Read-only binding to existing recording/AI-director plan JSON.

No model call, arbitrary command, game launch, or automatic visual approval.
Only independently inspected same-weapon candidates contained in a planned
player interval are offered. A sampled inspection is not a full director plan.
"""
from pathlib import PureWindowsPath
from .models import Inspection, JobRequest, InspectRequest


def select_director_candidates(inspection: Inspection, plan: dict, demo_path: str,
                               expected_sha256: str) -> Inspection:
    if inspection.demo_sha256 != expected_sha256:
        raise ValueError('Demo 已变化，请重新检查再导入导演计划')
    if (not isinstance(plan, dict) or not isinstance(plan.get('demo_path'), str)
            or isinstance(plan.get('tick_rate'), bool) or plan.get('tick_rate') != 64):
        raise ValueError('只接受现有 64 tick 制片计划')
    planned_path = InspectRequest.demo_local(plan['demo_path'])
    if PureWindowsPath(planned_path) != PureWindowsPath(demo_path):
        raise ValueError('导演计划不是当前 Demo')
    segments = plan.get('active_segments', plan.get('segments'))
    if not isinstance(segments, list) or not 1 <= len(segments) <= 128:
        raise ValueError('导演计划应包含 1–128 个片段')
    intervals = []
    for segment in segments:
        if not isinstance(segment, dict):
            raise ValueError('导演片段格式无效')
        if segment.get('disabled', False) is not False:
            continue
        xuid = JobRequest.exact_xuid(segment.get('target_steamid64'))
        start, end = segment.get('start_tick'), segment.get('end_tick')
        if type(start) is not int or type(end) is not int or not 0 <= start < end <= 100_000_000:
            raise ValueError('导演片段 tick 范围无效')
        intervals.append((xuid, start, end))
    candidates = [candidate for candidate in inspection.candidates
        if any(candidate.xuid == xuid and candidate.start_tick >= start and candidate.end_tick <= end
               for xuid, start, end in intervals)]
    if not candidates:
        raise ValueError('导演计划中没有通过独立武器检查的完整片段')
    return inspection.model_copy(update={'candidates': candidates})
