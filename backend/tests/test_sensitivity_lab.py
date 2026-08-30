import asyncio
import sys
from pathlib import Path

import pytest
from pydantic import ValidationError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.sensitivity_lab import (
    SensitivityRecommendationRequest,
    recommend_sensitivity,
    sensitivity_to_cm360,
)
from app.training_db import TrainingDB


def _trial(kind: str, multiplier: float, quality: float) -> dict:
    if kind == "flick":
        return {
            "kind": kind,
            "multiplier": multiplier,
            "duration_ms": 15_000,
            "hits": round(20 * quality),
            "targets": 20,
            "average_reaction_ms": 900 - 500 * quality,
            "path_efficiency": quality,
            "overshoots": round(8 * (1 - quality)),
        }
    return {
        "kind": kind,
        "multiplier": multiplier,
        "duration_ms": 15_000,
        "targets": 60,
        "on_target_ratio": quality,
        "path_efficiency": quality,
        "overshoots": round(12 * (1 - quality)),
    }


def _request() -> SensitivityRecommendationRequest:
    return SensitivityRecommendationRequest(
        dpi=800,
        current_sensitivity=1.0,
        m_yaw=0.022,
        game_width=1024,
        game_height=1080,
        display_aspect="16:9",
        scaling_mode="stretched",
        trials=[
            _trial("flick", 0.8, 0.45),
            _trial("tracking", 0.8, 0.50),
            _trial("flick", 1.0, 0.62),
            _trial("tracking", 1.0, 0.60),
            _trial("flick", 1.2, 0.94),
            _trial("tracking", 1.2, 0.92),
        ],
    )


def test_cm360_uses_cs2_yaw_formula():
    assert sensitivity_to_cm360(800, 1.0) == pytest.approx(51.9545, rel=1e-4)
    assert sensitivity_to_cm360(800, 1.0, 0.044) == pytest.approx(25.9773, rel=1e-4)


def test_recommendation_follows_strongest_measured_multiplier():
    result = recommend_sensitivity(_request())

    assert result.recommended_sensitivity > 1.1
    assert result.edpi == pytest.approx(800 * result.recommended_sensitivity)
    assert result.m_yaw == pytest.approx(0.022)
    assert result.current_cm_per_360 == pytest.approx(51.95)
    assert result.console_command.startswith('sensitivity "')
    assert "1024×1080" in result.resolution_context
    assert "m_yaw 0.022" in result.resolution_context
    assert "不会套用虚假的固定倍率" in result.resolution_context
    assert result.diagnosis == "too_slow"
    assert result.adjustment_percent > 0
    assert result.suggested_min < result.recommended_sensitivity < result.suggested_max
    assert result.insights
    assert len(result.action_plan) >= 3
    assert result.click_tendency == "insufficient"
    assert result.click_tendency_label == "点击样本不足"
    assert result.click_evidence


def _with_click_metrics(*, underflicks: int, overflicks: int, off_axis_misses: int = 0):
    payload = _request().model_dump()
    for trial in payload["trials"]:
        if trial["kind"] != "flick":
            continue
        trial.update({
            "clicks": trial["hits"] + 10,
            "misses": 10,
            "underflicks": underflicks,
            "overflicks": overflicks,
            "off_axis_misses": off_axis_misses,
            "average_click_error_ratio": 0.18,
        })
    return SensitivityRecommendationRequest.model_validate(payload)


def test_click_tendency_requires_eight_valid_clicks_and_ignores_off_axis_for_direction():
    insufficient_payload = _request().model_dump()
    for trial in insufficient_payload["trials"]:
        if trial["kind"] == "flick":
            trial.update({
                "hits": 0,
                "clicks": 3,
                "misses": 3,
                "underflicks": 2,
                "overflicks": 0,
                "off_axis_misses": 1,
            })
    insufficient = SensitivityRecommendationRequest.model_validate(insufficient_payload)
    insufficient_result = recommend_sensitivity(insufficient)
    assert insufficient_result.click_tendency == "insufficient"

    balanced = _with_click_metrics(underflicks=1, overflicks=0, off_axis_misses=9)
    balanced_result = recommend_sensitivity(balanced)
    assert balanced_result.click_tendency == "balanced"


def test_click_tendency_is_evidence_and_conflicting_late_clicks_lower_confidence():
    baseline = recommend_sensitivity(_request())
    early_result = recommend_sensitivity(_with_click_metrics(underflicks=8, overflicks=0))
    assert early_result.click_tendency == "early"
    assert early_result.diagnosis == "too_slow"
    assert early_result.recommended_sensitivity == baseline.recommended_sensitivity

    late_result = recommend_sensitivity(_with_click_metrics(underflicks=0, overflicks=8))
    assert late_result.click_tendency == "late"
    assert late_result.diagnosis == "mixed"
    assert late_result.recommended_sensitivity == baseline.recommended_sensitivity
    assert late_result.confidence == pytest.approx(round(baseline.confidence * 0.75, 3))


def test_late_clicks_reinforce_a_lower_sensitivity_recommendation():
    request = SensitivityRecommendationRequest(
        dpi=800,
        current_sensitivity=1.0,
        game_width=1920,
        game_height=1080,
        trials=[
            _trial("flick", 0.8, 0.96),
            _trial("tracking", 0.8, 0.95),
            _trial("flick", 1.0, 0.55),
            _trial("tracking", 1.0, 0.56),
            _trial("flick", 1.2, 0.35),
            _trial("tracking", 1.2, 0.36),
        ],
    )
    payload = request.model_dump()
    for trial in payload["trials"]:
        if trial["kind"] == "flick":
            trial.update({
                "clicks": trial["hits"] + 10,
                "misses": 10,
                "underflicks": 0,
                "overflicks": 8,
            })
    result = recommend_sensitivity(SensitivityRecommendationRequest.model_validate(payload))

    assert result.click_tendency == "late"
    assert result.diagnosis == "too_fast"
    assert result.adjustment_percent < 0
    assert any("降低游戏内灵敏度" in step for step in result.action_plan)


def test_click_metric_counts_must_be_internally_consistent():
    payload = _request().model_dump()
    payload["trials"][0].update({"clicks": 5, "hits": 3, "misses": 1})
    with pytest.raises(ValidationError, match=r"clicks 必须等于 hits \+ misses"):
        SensitivityRecommendationRequest.model_validate(payload)


def test_recommendation_requires_flick_and_tracking_trials():
    payload = _request().model_dump()
    payload["trials"] = [_trial("flick", 0.8, 0.8)] * 4

    with pytest.raises(ValidationError, match="甩枪测试和一轮追踪测试"):
        SensitivityRecommendationRequest.model_validate(payload)


def test_manual_unlimited_round_accepts_duration_over_three_minutes():
    payload = _request().model_dump()
    for trial in payload["trials"]:
        trial["duration_ms"] = 190_000

    request = SensitivityRecommendationRequest.model_validate(payload)

    assert all(trial.duration_ms == 190_000 for trial in request.trials)


def test_training_db_persists_and_lists_session(tmp_path: Path):
    async def scenario():
        database = TrainingDB(tmp_path / "training.db")
        await database.init_tables()
        request = _request()
        result = recommend_sensitivity(request)
        saved = await database.save_sensitivity_session(
            request.model_dump(mode="json"),
            result.model_dump(mode="json"),
        )
        rows = await database.list_sensitivity_sessions()
        return saved, rows

    saved, rows = asyncio.run(scenario())

    assert saved["id"] == 1
    assert len(rows) == 1
    assert rows[0]["recommended_sensitivity"] == saved["recommended_sensitivity"]
    assert rows[0]["game_width"] == 1024
    assert rows[0]["game_height"] == 1080
    assert rows[0]["m_yaw"] == pytest.approx(0.022)
