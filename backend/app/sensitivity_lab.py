"""Deterministic CS2 sensitivity recommendations from embedded aim trials."""

from __future__ import annotations

import math
from collections import defaultdict
from typing import Literal

from pydantic import BaseModel, Field, field_validator, model_validator


CS2_YAW = 0.022
CLICK_VALID_MIN = 8
CLICK_DIRECTION_MARGIN = 0.20


class SensitivityTrial(BaseModel):
    kind: Literal["flick", "tracking"]
    multiplier: float = Field(ge=0.5, le=1.5)
    duration_ms: int = Field(ge=3_000, le=86_400_000)
    hits: int = Field(default=0, ge=0, le=100_000)
    targets: int = Field(default=0, ge=0, le=100_000)
    average_reaction_ms: float = Field(default=0, ge=0, le=30_000)
    path_efficiency: float = Field(default=0, ge=0, le=1)
    overshoots: int = Field(default=0, ge=0, le=100_000)
    on_target_ratio: float = Field(default=0, ge=0, le=1)
    # These fields were added after the original no-click flick protocol. Keep
    # zero defaults so existing clients and persisted sessions remain valid.
    clicks: int = Field(default=0, ge=0, le=100_000)
    misses: int = Field(default=0, ge=0, le=100_000)
    underflicks: int = Field(default=0, ge=0, le=100_000)
    overflicks: int = Field(default=0, ge=0, le=100_000)
    off_axis_misses: int = Field(default=0, ge=0, le=100_000)
    average_click_error_ratio: float = Field(default=0, ge=0, le=1)

    @model_validator(mode="after")
    def validate_click_metrics(self) -> "SensitivityTrial":
        if self.clicks == 0:
            if any((self.misses, self.underflicks, self.overflicks, self.off_axis_misses)):
                raise ValueError("点击分类不允许在 clicks 为 0 时出现")
            return self
        if self.hits + self.misses != self.clicks:
            raise ValueError("clicks 必须等于 hits + misses")
        if self.off_axis_misses > self.misses:
            raise ValueError("偏轴未命中不能超过 misses")
        if self.underflicks + self.off_axis_misses > self.misses:
            raise ValueError("欠甩与偏轴分类不能超过 misses")
        if self.underflicks + self.overflicks + self.off_axis_misses > self.clicks:
            raise ValueError("点击方向分类总数不能超过 clicks")
        return self


class SensitivityRecommendationRequest(BaseModel):
    dpi: int = Field(ge=100, le=32_000)
    current_sensitivity: float = Field(gt=0, le=25)
    m_yaw: float = Field(default=CS2_YAW, gt=0, le=1)
    game_width: int = Field(ge=320, le=16_384)
    game_height: int = Field(ge=240, le=16_384)
    display_aspect: Literal["16:9", "16:10", "4:3", "5:4", "other"] = "16:9"
    scaling_mode: Literal["stretched", "black_bars", "native"] = "stretched"
    trials: list[SensitivityTrial] = Field(min_length=4, max_length=40)

    @field_validator("trials")
    @classmethod
    def require_both_test_kinds(cls, trials: list[SensitivityTrial]) -> list[SensitivityTrial]:
        kinds = {trial.kind for trial in trials}
        if kinds != {"flick", "tracking"}:
            raise ValueError("至少需要一轮甩枪测试和一轮追踪测试")
        if len({round(trial.multiplier, 3) for trial in trials}) < 2:
            raise ValueError("至少需要测试两个不同的灵敏度倍率")
        return trials


class SensitivityRecommendation(BaseModel):
    recommended_sensitivity: float
    current_sensitivity: float
    multiplier: float
    edpi: float
    cm_per_360: float
    current_cm_per_360: float
    m_yaw: float
    confidence: float
    score: float
    tested_scores: dict[str, float]
    resolution_context: str
    console_command: str
    diagnosis: Literal["too_fast", "too_slow", "balanced", "mixed"]
    diagnosis_label: str
    click_tendency: Literal["early", "late", "balanced", "mixed", "insufficient"]
    click_tendency_label: str
    click_evidence: list[str]
    adjustment_percent: float
    suggested_min: float
    suggested_max: float
    insights: list[str]
    action_plan: list[str]
    methodology_note: str


def sensitivity_to_cm360(dpi: int, sensitivity: float, m_yaw: float = CS2_YAW) -> float:
    return 360.0 * 2.54 / (float(dpi) * float(sensitivity) * float(m_yaw))


def _clamp(value: float, low: float, high: float) -> float:
    return max(low, min(high, value))


def _trial_score(trial: SensitivityTrial) -> float:
    path = _clamp(trial.path_efficiency, 0.0, 1.0)
    if trial.kind == "flick":
        hit_ratio = trial.hits / max(1, trial.targets)
        reaction = 0.0 if trial.average_reaction_ms <= 0 else _clamp(
            (950.0 - trial.average_reaction_ms) / 700.0,
            0.0,
            1.0,
        )
        overshoot = 1.0 - _clamp(trial.overshoots / max(1, trial.targets), 0.0, 1.0)
        return 0.45 * hit_ratio + 0.30 * reaction + 0.15 * path + 0.10 * overshoot
    stability = 1.0 - _clamp(trial.overshoots / max(1, trial.targets), 0.0, 1.0)
    return 0.65 * trial.on_target_ratio + 0.20 * path + 0.15 * stability


def _resolution_context(request: SensitivityRecommendationRequest) -> str:
    game_ratio = request.game_width / request.game_height
    ratio_label = f"{request.game_width}×{request.game_height} ({game_ratio:.3f}:1)"
    yaw_label = f"m_yaw {request.m_yaw:g}"
    if request.scaling_mode == "stretched":
        return (
            f"测试已记录游戏分辨率 {ratio_label}，并按 {request.display_aspect} 拉伸显示理解视觉速度；"
            f"角度与 cm/360 按 {yaw_label} 计算。分辨率不直接改变 CS2 转角，因此不会套用虚假的固定倍率修正。"
        )
    return (
        f"测试已记录游戏分辨率 {ratio_label}、{request.scaling_mode} 显示，并按 {yaw_label} 计算；"
        "结果由实际鼠标测试表现决定，分辨率只作为视觉与准星移动背景。"
    )


def _click_tendency(trials: list[SensitivityTrial]) -> tuple[str, str, list[str]]:
    """Summarize click timing without letting it override sensitivity scores.

    ``underflicks`` and ``overflicks`` are directional evidence. Off-axis
    misses are intentionally excluded from the valid-click denominator: they
    describe lateral placement, not early/late timing. Eight valid clicks and
    a 20% under/over difference are required before returning a direction.
    """
    flick_trials = [trial for trial in trials if trial.kind == "flick"]
    clicks = sum(trial.clicks for trial in flick_trials)
    hits = sum(trial.hits for trial in flick_trials)
    misses = sum(trial.misses for trial in flick_trials)
    underflicks = sum(trial.underflicks for trial in flick_trials)
    overflicks = sum(trial.overflicks for trial in flick_trials)
    off_axis = min(clicks, sum(trial.off_axis_misses for trial in flick_trials))
    valid_clicks = max(0, clicks - off_axis)
    directional_clicks = underflicks + overflicks
    average_error_weight = sum(
        trial.average_click_error_ratio * trial.clicks
        for trial in flick_trials
    )
    average_error = average_error_weight / clicks if clicks else 0.0

    evidence = [
        f"甩枪记录 {clicks} 次点击；排除偏轴落点 {off_axis} 次后，有效点击 {valid_clicks} 次。",
        f"方向事件：点早（欠甩）{underflicks} 次，点晚（过甩或修正）{overflicks} 次，偏轴 {off_axis} 次；命中 {hits} 次。",
        f"平均点击落点误差约 {average_error * 100:.1f}%；未命中 {misses} 次。",
    ]

    if valid_clicks < CLICK_VALID_MIN:
        return "insufficient", "点击样本不足", evidence + [
            f"有效点击少于 {CLICK_VALID_MIN} 次，本次不据此判断点早或点晚。",
        ]

    difference = abs(underflicks - overflicks)
    if difference >= valid_clicks * CLICK_DIRECTION_MARGIN:
        if underflicks > overflicks:
            return "early", "点早（欠甩偏多）", evidence + [
                "点早（欠甩）相对点晚（过甩）至少多出有效点击的 20%，可作为辅助证据。",
            ]
        if overflicks > underflicks:
            return "late", "点晚（过甩偏多）", evidence + [
                "点晚（过甩）相对点早（欠甩）至少多出有效点击的 20%，可作为辅助证据。",
            ]

    # A global near-tie is normally balanced. ``mixed`` is reserved for a
    # real split between candidate multipliers, so the enum remains useful
    # without weakening the 20% global direction threshold.
    by_multiplier: dict[float, list[int]] = defaultdict(lambda: [0, 0, 0])
    for trial in flick_trials:
        multiplier = round(float(trial.multiplier), 3)
        by_multiplier[multiplier][0] += max(0, trial.clicks - min(trial.clicks, trial.off_axis_misses))
        by_multiplier[multiplier][1] += trial.underflicks
        by_multiplier[multiplier][2] += trial.overflicks
    candidate_directions: set[str] = set()
    for candidate_valid, candidate_under, candidate_over in by_multiplier.values():
        candidate_difference = abs(candidate_under - candidate_over)
        if candidate_valid >= 4 and candidate_difference >= candidate_valid * CLICK_DIRECTION_MARGIN:
            candidate_directions.add("early" if candidate_under > candidate_over else "late")
    if len(candidate_directions) > 1 and directional_clicks > 0:
        return "mixed", "点击落点方向不一致", evidence + [
            "不同灵敏度倍率下的点早/点晚证据互相冲突，不把点击倾向单独用于改值。",
        ]
    return "balanced", "点击落点基本均衡", evidence + [
        "点早与点晚差异未达到 20% 门槛，暂按点击落点基本均衡处理。",
    ]


def recommend_sensitivity(request: SensitivityRecommendationRequest) -> SensitivityRecommendation:
    grouped: dict[float, list[float]] = defaultdict(list)
    kinds_by_multiplier: dict[float, set[str]] = defaultdict(set)
    scores_by_kind: dict[float, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    for trial in request.trials:
        multiplier = round(float(trial.multiplier), 3)
        score = _trial_score(trial)
        grouped[multiplier].append(score)
        scores_by_kind[multiplier][trial.kind].append(score)
        kinds_by_multiplier[multiplier].add(trial.kind)

    aggregate: dict[float, float] = {}
    for multiplier, scores in grouped.items():
        kind_coverage = len(kinds_by_multiplier[multiplier]) / 2.0
        aggregate[multiplier] = (sum(scores) / len(scores)) * (0.9 + 0.1 * kind_coverage)

    best_multiplier = max(aggregate, key=lambda value: (aggregate[value], -abs(1.0 - value)))
    best_score = aggregate[best_multiplier]

    # Blend strong neighbouring candidates to avoid snapping to one noisy round.
    eligible = {
        multiplier: score
        for multiplier, score in aggregate.items()
        if score >= best_score - 0.12
    }
    weights = {
        multiplier: math.exp((score - best_score) * 8.0)
        for multiplier, score in eligible.items()
    }
    recommended_multiplier = sum(multiplier * weights[multiplier] for multiplier in eligible) / sum(weights.values())
    recommended_multiplier = _clamp(recommended_multiplier, 0.65, 1.45)
    recommended = round(request.current_sensitivity * recommended_multiplier, 4)
    recommended = _clamp(recommended, 0.01, 25.0)

    sorted_scores = sorted(aggregate.values(), reverse=True)
    separation = sorted_scores[0] - sorted_scores[1] if len(sorted_scores) > 1 else 0.0
    complete_candidates = sum(1 for kinds in kinds_by_multiplier.values() if len(kinds) == 2)
    coverage = min(1.0, len(request.trials) / 10.0) * min(1.0, complete_candidates / 3.0)
    confidence = _clamp(0.35 + 0.45 * coverage + 0.20 * min(1.0, separation / 0.15), 0.0, 0.98)

    kind_scores = {
        multiplier: {
            kind: sum(values) / len(values)
            for kind, values in grouped_kinds.items()
            if values
        }
        for multiplier, grouped_kinds in scores_by_kind.items()
    }
    best_flick = max(
        (multiplier for multiplier in kind_scores if "flick" in kind_scores[multiplier]),
        key=lambda multiplier: (kind_scores[multiplier]["flick"], -abs(1.0 - multiplier)),
    )
    best_tracking = max(
        (multiplier for multiplier in kind_scores if "tracking" in kind_scores[multiplier]),
        key=lambda multiplier: (kind_scores[multiplier]["tracking"], -abs(1.0 - multiplier)),
    )
    click_tendency, click_tendency_label, click_evidence = _click_tendency(request.trials)
    recommended_ratio = recommended / request.current_sensitivity
    split_preference = abs(best_flick - best_tracking) >= 0.25
    if split_preference:
        diagnosis = "mixed"
        diagnosis_label = "甩枪与追踪偏好不一致"
    elif recommended_ratio < 0.94:
        diagnosis = "too_fast"
        diagnosis_label = "当前灵敏度偏快"
    elif recommended_ratio > 1.06:
        diagnosis = "too_slow"
        diagnosis_label = "当前灵敏度偏慢"
    else:
        diagnosis = "balanced"
        diagnosis_label = "当前灵敏度接近平衡区"

    click_conflict = (
        (click_tendency == "early" and diagnosis == "too_fast")
        or (click_tendency == "late" and diagnosis == "too_slow")
    )
    if click_conflict:
        diagnosis = "mixed"
        diagnosis_label = "综合成绩与点击落点方向不一致"
        confidence = _clamp(confidence * 0.75, 0.0, 0.98)

    current_multiplier = min(aggregate, key=lambda value: abs(value - 1.0))
    current_trials = [trial for trial in request.trials if round(float(trial.multiplier), 3) == current_multiplier]
    current_targets = sum(trial.targets for trial in current_trials)
    current_overshoots = sum(trial.overshoots for trial in current_trials)
    overshoot_rate = current_overshoots / max(1, current_targets)
    adjustment_percent = (recommended_ratio - 1.0) * 100.0
    insights = [
        f"当前倍率附近记录到 {overshoot_rate * 100:.1f}% 的过冲事件。",
        f"甩枪最佳测试倍率为 ×{best_flick:g}，追踪最佳测试倍率为 ×{best_tracking:g}。",
    ]
    if diagnosis == "too_fast":
        insights.append("较低倍率在命中、路径控制与过冲惩罚后的综合表现更好，建议先降速。")
    elif diagnosis == "too_slow":
        insights.append("较高倍率在没有明显牺牲控制的情况下完成目标更快，建议小幅提速。")
    elif diagnosis == "mixed":
        insights.append("不要一次大幅改动；先使用折中值，并用更长测试确认你更重视甩枪还是连续追踪。")
    else:
        insights.append("当前值已经落在实测平衡区，继续追求大幅变化的收益有限。")
    if confidence < 0.65:
        insights.append("候选成绩接近或样本较少，本次结论置信度有限。")
    if click_tendency in {"early", "late"}:
        if click_conflict:
            insights.append("点击落点方向与倍率综合推荐冲突；点击倾向仅作提醒，不单独改变推荐值。")
        else:
            insights.append(f"点击倾向为“{click_tendency_label}”，与倍率综合推荐仅作一致性参考。")
    elif click_tendency == "insufficient":
        insights.append("本次没有足够的真实点击样本，未用点早/点晚判断影响灵敏度推荐。")

    suggested_min = round(_clamp(recommended * 0.96, 0.01, 25.0), 4)
    suggested_max = round(_clamp(recommended * 1.04, 0.01, 25.0), 4)
    action_plan = [
        f"先在 CS2 输入 sensitivity \"{recommended:g}\"，只修改这一项。",
        f"在 {suggested_min:g}–{suggested_max:g} 范围内各复测一轮；每轮建议至少 30 秒。",
        "若仍频繁越过目标，向区间下沿调；若总是停在目标前，向区间上沿调。",
        "保留 DPI 不变，连续使用两到三局后再决定是否固化设置。",
    ]
    if click_tendency == "early" and diagnosis == "too_slow":
        action_plan.insert(1, "真实点击多停在目标前；本轮建议方向是小幅提高游戏内灵敏度。")
    elif click_tendency == "late" and diagnosis == "too_fast":
        action_plan.insert(1, "真实点击多发生在越过目标之后；本轮建议方向是小幅降低游戏内灵敏度。")

    return SensitivityRecommendation(
        recommended_sensitivity=recommended,
        current_sensitivity=request.current_sensitivity,
        multiplier=round(recommended / request.current_sensitivity, 4),
        edpi=round(request.dpi * recommended, 1),
        cm_per_360=round(sensitivity_to_cm360(request.dpi, recommended, request.m_yaw), 2),
        current_cm_per_360=round(
            sensitivity_to_cm360(request.dpi, request.current_sensitivity, request.m_yaw),
            2,
        ),
        m_yaw=round(request.m_yaw, 6),
        confidence=round(confidence, 3),
        score=round(best_score, 3),
        tested_scores={f"{key:.3f}": round(value, 3) for key, value in sorted(aggregate.items())},
        resolution_context=_resolution_context(request),
        console_command=f'sensitivity "{recommended:g}"',
        diagnosis=diagnosis,
        diagnosis_label=diagnosis_label,
        click_tendency=click_tendency,
        click_tendency_label=click_tendency_label,
        click_evidence=click_evidence,
        adjustment_percent=round(adjustment_percent, 1),
        suggested_min=suggested_min,
        suggested_max=suggested_max,
        insights=insights,
        action_plan=action_plan,
        methodology_note=(
            "建议来自本次甩枪与追踪的速度—精度权衡；甩枪点击倾向只作为一致性证据，不会单独改动推荐值。"
            "测试场按当前 sensitivity 与 m_yaw 生成候选增益，DPI、sensitivity 和 m_yaw 用于 eDPI 与 cm/360。"
            "浏览器指针锁定仍是相对模拟，最终请在 CS2 内复测确认。"
        ),
    )
