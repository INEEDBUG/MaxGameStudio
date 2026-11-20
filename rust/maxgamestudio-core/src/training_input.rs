//! Local training recommendations derived only from recorded aim and key events.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub const CS2_YAW: f64 = 0.022;
const MAX_DURATION_MS: f64 = 86_400_000.0;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct TrainingInputError {
    pub field: String,
    pub kind: &'static str,
    pub message: String,
}

impl fmt::Display for TrainingInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for TrainingInputError {}

fn issue(
    field: impl Into<String>,
    kind: &'static str,
    message: impl Into<String>,
) -> TrainingInputError {
    TrainingInputError {
        field: field.into(),
        kind,
        message: message.into(),
    }
}

fn value_error(field: impl Into<String>, message: &str) -> TrainingInputError {
    issue(field, "value_error", format!("Value error, {message}"))
}

fn bounds(
    field: &str,
    value: f64,
    minimum: f64,
    maximum: f64,
    exclusive_minimum: bool,
) -> Result<(), TrainingInputError> {
    let below = value.is_nan() || value < minimum || (exclusive_minimum && value == minimum);
    if below {
        let (kind, comparator) = if exclusive_minimum {
            ("greater_than", "greater than")
        } else {
            ("greater_than_equal", "greater than or equal to")
        };
        return Err(issue(
            field,
            kind,
            format!("Input should be {comparator} {minimum}"),
        ));
    }
    if value > maximum {
        return Err(issue(
            field,
            "less_than_equal",
            format!("Input should be less than or equal to {maximum}"),
        ));
    }
    Ok(())
}

fn list_bounds(
    field: &str,
    length: usize,
    minimum: usize,
    maximum: usize,
) -> Result<(), TrainingInputError> {
    if length < minimum {
        return Err(issue(
            field,
            "too_short",
            format!("List should have at least {minimum} items after validation, not {length}"),
        ));
    }
    if length > maximum {
        return Err(issue(
            field,
            "too_long",
            format!("List should have at most {maximum} items after validation, not {length}"),
        ));
    }
    Ok(())
}

// CPython rounds to nearest at the decimal place against the binary input.
// Formatting before parsing preserves ties and avoids the error of multiplying
// by a decimal power first (for example, round(2.675, 2) is 2.67).
fn round(value: f64, decimals: usize) -> f64 {
    format!("{value:.decimals$}")
        .parse()
        .expect("formatted number")
}

fn general(value: f64) -> String {
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("finite number");
    let exponent: i32 = exponent.parse().expect("formatted exponent");
    let trim = |text: &str| {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text.to_owned()
        }
    };
    if !(-4..6).contains(&exponent) {
        return format!("{}e{exponent:+03}", trim(mantissa));
    }
    let decimals = (5 - exponent) as usize;
    trim(&format!("{value:.decimals$}"))
}

fn compensated_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut total = 0.0_f64;
    let mut correction = 0.0;
    for value in values {
        let next = total + value;
        correction += if total.abs() >= value.abs() {
            (total - next) + value
        } else {
            (value - next) + total
        };
        total = next;
    }
    total + correction
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialKind {
    Flick,
    Tracking,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SensitivityTrial {
    pub kind: TrialKind,
    pub multiplier: f64,
    pub duration_ms: i64,
    #[serde(default)]
    pub hits: i64,
    #[serde(default)]
    pub targets: i64,
    #[serde(default)]
    pub average_reaction_ms: f64,
    #[serde(default)]
    pub path_efficiency: f64,
    #[serde(default)]
    pub overshoots: i64,
    #[serde(default)]
    pub on_target_ratio: f64,
    #[serde(default)]
    pub clicks: i64,
    #[serde(default)]
    pub misses: i64,
    #[serde(default)]
    pub underflicks: i64,
    #[serde(default)]
    pub overflicks: i64,
    #[serde(default)]
    pub off_axis_misses: i64,
    #[serde(default)]
    pub average_click_error_ratio: f64,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize)]
pub enum DisplayAspect {
    #[default]
    #[serde(rename = "16:9")]
    SixteenNine,
    #[serde(rename = "16:10")]
    SixteenTen,
    #[serde(rename = "4:3")]
    FourThree,
    #[serde(rename = "5:4")]
    FiveFour,
    #[serde(rename = "other")]
    Other,
}

impl DisplayAspect {
    fn name(self) -> &'static str {
        match self {
            Self::SixteenNine => "16:9",
            Self::SixteenTen => "16:10",
            Self::FourThree => "4:3",
            Self::FiveFour => "5:4",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalingMode {
    #[default]
    Stretched,
    BlackBars,
    Native,
}

fn default_yaw() -> f64 {
    CS2_YAW
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SensitivityRecommendationRequest {
    pub dpi: i64,
    pub current_sensitivity: f64,
    #[serde(default = "default_yaw")]
    pub m_yaw: f64,
    pub game_width: i64,
    pub game_height: i64,
    #[serde(default)]
    pub display_aspect: DisplayAspect,
    #[serde(default)]
    pub scaling_mode: ScalingMode,
    pub trials: Vec<SensitivityTrial>,
}

#[derive(Debug, Serialize)]
pub struct SensitivityRecommendation {
    pub recommended_sensitivity: f64,
    pub current_sensitivity: f64,
    pub multiplier: f64,
    pub edpi: f64,
    pub cm_per_360: f64,
    pub current_cm_per_360: f64,
    pub m_yaw: f64,
    pub confidence: f64,
    pub score: f64,
    pub tested_scores: BTreeMap<String, f64>,
    pub resolution_context: String,
    pub console_command: String,
    pub diagnosis: &'static str,
    pub diagnosis_label: &'static str,
    pub click_tendency: &'static str,
    pub click_tendency_label: &'static str,
    pub click_evidence: Vec<String>,
    pub adjustment_percent: f64,
    pub suggested_min: f64,
    pub suggested_max: f64,
    pub insights: Vec<String>,
    pub action_plan: Vec<String>,
    pub methodology_note: &'static str,
}

impl SensitivityTrial {
    fn validate(&self) -> Result<(), TrainingInputError> {
        for (field, value, minimum, maximum) in [
            ("multiplier", self.multiplier, 0.5, 1.5),
            (
                "duration_ms",
                self.duration_ms as f64,
                3_000.0,
                MAX_DURATION_MS,
            ),
            ("hits", self.hits as f64, 0.0, 100_000.0),
            ("targets", self.targets as f64, 0.0, 100_000.0),
            (
                "average_reaction_ms",
                self.average_reaction_ms,
                0.0,
                30_000.0,
            ),
            ("path_efficiency", self.path_efficiency, 0.0, 1.0),
            ("overshoots", self.overshoots as f64, 0.0, 100_000.0),
            ("on_target_ratio", self.on_target_ratio, 0.0, 1.0),
            ("clicks", self.clicks as f64, 0.0, 100_000.0),
            ("misses", self.misses as f64, 0.0, 100_000.0),
            ("underflicks", self.underflicks as f64, 0.0, 100_000.0),
            ("overflicks", self.overflicks as f64, 0.0, 100_000.0),
            (
                "off_axis_misses",
                self.off_axis_misses as f64,
                0.0,
                100_000.0,
            ),
            (
                "average_click_error_ratio",
                self.average_click_error_ratio,
                0.0,
                1.0,
            ),
        ] {
            bounds(field, value, minimum, maximum, false)?;
        }
        let problem = if self.clicks == 0 {
            (self.misses != 0
                || self.underflicks != 0
                || self.overflicks != 0
                || self.off_axis_misses != 0)
                .then_some("点击分类不允许在 clicks 为 0 时出现")
        } else if self.hits + self.misses != self.clicks {
            Some("clicks 必须等于 hits + misses")
        } else if self.off_axis_misses > self.misses {
            Some("偏轴未命中不能超过 misses")
        } else if self.underflicks + self.off_axis_misses > self.misses {
            Some("欠甩与偏轴分类不能超过 misses")
        } else if self.underflicks + self.overflicks + self.off_axis_misses > self.clicks {
            Some("点击方向分类总数不能超过 clicks")
        } else {
            None
        };
        problem.map_or(Ok(()), |message| Err(value_error("", message)))
    }

    fn score(&self) -> f64 {
        let targets = self.targets.max(1) as f64;
        let stability = 1.0 - (self.overshoots as f64 / targets).clamp(0.0, 1.0);
        match self.kind {
            TrialKind::Flick => {
                let hit_ratio = self.hits as f64 / targets;
                let reaction = if self.average_reaction_ms <= 0.0 {
                    0.0
                } else {
                    ((950.0 - self.average_reaction_ms) / 700.0).clamp(0.0, 1.0)
                };
                0.45 * hit_ratio + 0.30 * reaction + 0.15 * self.path_efficiency + 0.10 * stability
            }
            TrialKind::Tracking => {
                0.65 * self.on_target_ratio + 0.20 * self.path_efficiency + 0.15 * stability
            }
        }
    }
}

impl SensitivityRecommendationRequest {
    fn validate(&self) -> Result<(), TrainingInputError> {
        bounds("dpi", self.dpi as f64, 100.0, 32_000.0, false)?;
        bounds(
            "current_sensitivity",
            self.current_sensitivity,
            0.0,
            25.0,
            true,
        )?;
        bounds("m_yaw", self.m_yaw, 0.0, 1.0, true)?;
        bounds("game_width", self.game_width as f64, 320.0, 16_384.0, false)?;
        bounds(
            "game_height",
            self.game_height as f64,
            240.0,
            16_384.0,
            false,
        )?;
        list_bounds("trials", self.trials.len(), 4, 40)?;
        for (index, trial) in self.trials.iter().enumerate() {
            trial.validate().map_err(|mut err| {
                err.field = if err.field.is_empty() {
                    format!("trials.{index}")
                } else {
                    format!("trials.{index}.{}", err.field)
                };
                err
            })?;
        }
        if ![TrialKind::Flick, TrialKind::Tracking]
            .into_iter()
            .all(|kind| self.trials.iter().any(|trial| trial.kind == kind))
        {
            return Err(value_error("trials", "至少需要一轮甩枪测试和一轮追踪测试"));
        }
        let first = round(self.trials[0].multiplier, 3);
        if !self
            .trials
            .iter()
            .any(|trial| round(trial.multiplier, 3) != first)
        {
            return Err(value_error("trials", "至少需要测试两个不同的灵敏度倍率"));
        }
        Ok(())
    }

    fn resolution_context(&self) -> String {
        let ratio = self.game_width as f64 / self.game_height as f64;
        let ratio_label = format!("{}×{} ({ratio:.3}:1)", self.game_width, self.game_height);
        let yaw = general(self.m_yaw);
        match self.scaling_mode {
            ScalingMode::Stretched => format!(
                "测试已记录游戏分辨率 {ratio_label}，并按 {} 拉伸显示理解视觉速度；角度与 cm/360 按 m_yaw {yaw} 计算。分辨率不直接改变 CS2 转角，因此不会套用虚假的固定倍率修正。",
                self.display_aspect.name()
            ),
            mode => {
                let mode = if mode == ScalingMode::BlackBars { "black_bars" } else { "native" };
                format!("测试已记录游戏分辨率 {ratio_label}、{mode} 显示，并按 m_yaw {yaw} 计算；结果由实际鼠标测试表现决定，分辨率只作为视觉与准星移动背景。")
            }
        }
    }
}

pub fn sensitivity_to_cm360(dpi: i64, sensitivity: f64, m_yaw: f64) -> f64 {
    360.0 * 2.54 / (dpi as f64 * sensitivity * m_yaw)
}

struct Candidate<'a> {
    multiplier: f64,
    trials: Vec<&'a SensitivityTrial>,
    score: f64,
    flick_score: Option<f64>,
    tracking_score: Option<f64>,
}

fn candidates(trials: &[SensitivityTrial]) -> Vec<Candidate<'_>> {
    let mut groups: Vec<Candidate<'_>> = Vec::new();
    for trial in trials {
        let multiplier = round(trial.multiplier, 3);
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.multiplier == multiplier)
        {
            group.trials.push(trial);
        } else {
            groups.push(Candidate {
                multiplier,
                trials: vec![trial],
                score: 0.0,
                flick_score: None,
                tracking_score: None,
            });
        }
    }
    for group in &mut groups {
        let mean_kind = |kind| {
            let scores = group.trials.iter().filter(|trial| trial.kind == kind);
            let count = scores.clone().count();
            (count > 0).then(|| compensated_sum(scores.map(|trial| trial.score())) / count as f64)
        };
        group.flick_score = mean_kind(TrialKind::Flick);
        group.tracking_score = mean_kind(TrialKind::Tracking);
        let coverage = if group.flick_score.is_some() && group.tracking_score.is_some() {
            1.0
        } else {
            0.5
        };
        group.score = (compensated_sum(group.trials.iter().map(|trial| trial.score()))
            / group.trials.len() as f64)
            * (0.9 + 0.1 * coverage);
    }
    groups
}

fn best_candidate<'a>(
    groups: &'a [Candidate<'a>],
    score: impl Fn(&Candidate<'a>) -> Option<f64>,
) -> &'a Candidate<'a> {
    groups
        .iter()
        .filter_map(|candidate| score(candidate).map(|score| (candidate, score)))
        .reduce(|best, contender| {
            let (candidate, score) = contender;
            let better_tie = score == best.1
                && (1.0 - candidate.multiplier).abs() < (1.0 - best.0.multiplier).abs();
            if score > best.1 || better_tie {
                contender
            } else {
                best
            }
        })
        .expect("validated trials contain this score")
        .0
}

struct ClickTendency {
    name: &'static str,
    label: &'static str,
    evidence: Vec<String>,
}

fn click_tendency(trials: &[SensitivityTrial], groups: &[Candidate<'_>]) -> ClickTendency {
    let flick = || trials.iter().filter(|trial| trial.kind == TrialKind::Flick);
    let clicks: i64 = flick().map(|trial| trial.clicks).sum();
    let hits: i64 = flick().map(|trial| trial.hits).sum();
    let misses: i64 = flick().map(|trial| trial.misses).sum();
    let under: i64 = flick().map(|trial| trial.underflicks).sum();
    let over: i64 = flick().map(|trial| trial.overflicks).sum();
    let off_axis = clicks.min(flick().map(|trial| trial.off_axis_misses).sum());
    let valid = (clicks - off_axis).max(0);
    let average_error = if clicks > 0 {
        compensated_sum(flick().map(|trial| trial.average_click_error_ratio * trial.clicks as f64))
            / clicks as f64
            * 100.0
    } else {
        0.0
    };
    let mut evidence = vec![
        format!("甩枪记录 {clicks} 次点击；排除偏轴落点 {off_axis} 次后，有效点击 {valid} 次。"),
        format!("方向事件：点早（欠甩）{under} 次，点晚（过甩或修正）{over} 次，偏轴 {off_axis} 次；命中 {hits} 次。"),
        format!("平均点击落点误差约 {average_error:.1}%；未命中 {misses} 次。"),
    ];
    let directional = (under - over).abs() as f64 >= valid as f64 * 0.20;
    let (name, label, explanation) = if valid < 8 {
        (
            "insufficient",
            "点击样本不足",
            "有效点击少于 8 次，本次不据此判断点早或点晚。",
        )
    } else if directional && under > over {
        (
            "early",
            "点早（欠甩偏多）",
            "点早（欠甩）相对点晚（过甩）至少多出有效点击的 20%，可作为辅助证据。",
        )
    } else if directional && over > under {
        (
            "late",
            "点晚（过甩偏多）",
            "点晚（过甩）相对点早（欠甩）至少多出有效点击的 20%，可作为辅助证据。",
        )
    } else {
        let (mut early, mut late) = (false, false);
        for group in groups {
            let trials = group
                .trials
                .iter()
                .filter(|trial| trial.kind == TrialKind::Flick);
            let count: i64 = trials
                .clone()
                .map(|trial| trial.clicks - trial.off_axis_misses)
                .sum();
            let difference: i64 = trials
                .map(|trial| trial.underflicks - trial.overflicks)
                .sum();
            if count >= 4 && difference.abs() as f64 >= count as f64 * 0.20 {
                early |= difference > 0;
                late |= difference < 0;
            }
        }
        if early && late {
            (
                "mixed",
                "点击落点方向不一致",
                "不同灵敏度倍率下的点早/点晚证据互相冲突，不把点击倾向单独用于改值。",
            )
        } else {
            (
                "balanced",
                "点击落点基本均衡",
                "点早与点晚差异未达到 20% 门槛，暂按点击落点基本均衡处理。",
            )
        }
    };
    evidence.push(explanation.to_owned());
    ClickTendency {
        name,
        label,
        evidence,
    }
}

pub fn recommend_sensitivity(
    request: &SensitivityRecommendationRequest,
) -> Result<SensitivityRecommendation, TrainingInputError> {
    request.validate()?;
    let groups = candidates(&request.trials);
    let best = best_candidate(&groups, |candidate| Some(candidate.score));
    let best_flick = best_candidate(&groups, |candidate| candidate.flick_score).multiplier;
    let best_tracking = best_candidate(&groups, |candidate| candidate.tracking_score).multiplier;
    let weight = |score: f64| ((score - best.score) * 8.0).exp();
    let eligible = groups
        .iter()
        .filter(|group| group.score >= best.score - 0.12);
    let weighted_multiplier = compensated_sum(
        eligible
            .clone()
            .map(|group| group.multiplier * weight(group.score)),
    ) / compensated_sum(eligible.map(|group| weight(group.score)));
    let recommended = round(
        request.current_sensitivity * weighted_multiplier.clamp(0.65, 1.45),
        4,
    )
    .clamp(0.01, 25.0);
    let complete = groups
        .iter()
        .filter(|group| group.flick_score.is_some() && group.tracking_score.is_some())
        .count();
    let mut sorted_scores: Vec<_> = groups.iter().map(|candidate| candidate.score).collect();
    sorted_scores.sort_by(|a, b| b.total_cmp(a));
    let separation = sorted_scores[0] - sorted_scores[1];
    let coverage = (request.trials.len() as f64 / 10.0).min(1.0) * (complete as f64 / 3.0).min(1.0);
    let mut confidence =
        (0.35 + 0.45 * coverage + 0.20 * (separation / 0.15).min(1.0)).clamp(0.0, 0.98);
    let click = click_tendency(&request.trials, &groups);
    let ratio = recommended / request.current_sensitivity;
    let (mut diagnosis, mut diagnosis_label) = if (best_flick - best_tracking).abs() >= 0.25 {
        ("mixed", "甩枪与追踪偏好不一致")
    } else if ratio < 0.94 {
        ("too_fast", "当前灵敏度偏快")
    } else if ratio > 1.06 {
        ("too_slow", "当前灵敏度偏慢")
    } else {
        ("balanced", "当前灵敏度接近平衡区")
    };
    let conflict = matches!(
        (click.name, diagnosis),
        ("early", "too_fast") | ("late", "too_slow")
    );
    if conflict {
        diagnosis = "mixed";
        diagnosis_label = "综合成绩与点击落点方向不一致";
        confidence = (confidence * 0.75).clamp(0.0, 0.98);
    }

    let current = groups
        .iter()
        .min_by(|left, right| {
            (left.multiplier - 1.0)
                .abs()
                .total_cmp(&(right.multiplier - 1.0).abs())
        })
        .expect("at least two candidates");
    let current_targets: i64 = current.trials.iter().map(|trial| trial.targets).sum();
    let current_overshoots: i64 = current.trials.iter().map(|trial| trial.overshoots).sum();
    let overshoot = current_overshoots as f64 / current_targets.max(1) as f64 * 100.0;
    let mut insights = vec![
        format!("当前倍率附近记录到 {overshoot:.1}% 的过冲事件。"),
        format!(
            "甩枪最佳测试倍率为 ×{}，追踪最佳测试倍率为 ×{}。",
            general(best_flick),
            general(best_tracking)
        ),
        match diagnosis {
            "too_fast" => "较低倍率在命中、路径控制与过冲惩罚后的综合表现更好，建议先降速。",
            "too_slow" => "较高倍率在没有明显牺牲控制的情况下完成目标更快，建议小幅提速。",
            "mixed" => "不要一次大幅改动；先使用折中值，并用更长测试确认你更重视甩枪还是连续追踪。",
            _ => "当前值已经落在实测平衡区，继续追求大幅变化的收益有限。",
        }
        .to_owned(),
    ];
    if confidence < 0.65 {
        insights.push("候选成绩接近或样本较少，本次结论置信度有限。".to_owned());
    }
    match click.name {
        "early" | "late" if conflict => insights.push(
            "点击落点方向与倍率综合推荐冲突；点击倾向仅作提醒，不单独改变推荐值。".to_owned(),
        ),
        "early" | "late" => insights.push(format!(
            "点击倾向为“{}”，与倍率综合推荐仅作一致性参考。",
            click.label
        )),
        "insufficient" => insights
            .push("本次没有足够的真实点击样本，未用点早/点晚判断影响灵敏度推荐。".to_owned()),
        _ => {}
    }
    let suggested_min = round((recommended * 0.96).clamp(0.01, 25.0), 4);
    let suggested_max = round((recommended * 1.04).clamp(0.01, 25.0), 4);
    let console_command = format!("sensitivity \"{}\"", general(recommended));
    let mut action_plan = vec![
        format!("先在 CS2 输入 {console_command}，只修改这一项。"),
        format!(
            "在 {}–{} 范围内各复测一轮；每轮建议至少 30 秒。",
            general(suggested_min),
            general(suggested_max)
        ),
        "若仍频繁越过目标，向区间下沿调；若总是停在目标前，向区间上沿调。".to_owned(),
        "保留 DPI 不变，连续使用两到三局后再决定是否固化设置。".to_owned(),
    ];
    match (click.name, diagnosis) {
        ("early", "too_slow") => action_plan.insert(
            1,
            "真实点击多停在目标前；本轮建议方向是小幅提高游戏内灵敏度。".to_owned(),
        ),
        ("late", "too_fast") => action_plan.insert(
            1,
            "真实点击多发生在越过目标之后；本轮建议方向是小幅降低游戏内灵敏度。".to_owned(),
        ),
        _ => {}
    }

    if request.dpi as f64 * request.current_sensitivity * request.m_yaw == 0.0
        || request.dpi as f64 * recommended * request.m_yaw == 0.0
    {
        return Err(issue("m_yaw", "division_by_zero", "float division by zero"));
    }
    Ok(SensitivityRecommendation {
        recommended_sensitivity: recommended,
        current_sensitivity: request.current_sensitivity,
        multiplier: round(ratio, 4),
        edpi: round(request.dpi as f64 * recommended, 1),
        cm_per_360: round(sensitivity_to_cm360(request.dpi, recommended, request.m_yaw), 2),
        current_cm_per_360: round(sensitivity_to_cm360(request.dpi, request.current_sensitivity, request.m_yaw), 2),
        m_yaw: round(request.m_yaw, 6),
        confidence: round(confidence, 3),
        score: round(best.score, 3),
        tested_scores: groups.iter().map(|group| (format!("{:.3}", group.multiplier), round(group.score, 3))).collect(),
        resolution_context: request.resolution_context(),
        console_command,
        diagnosis,
        diagnosis_label,
        click_tendency: click.name,
        click_tendency_label: click.label,
        click_evidence: click.evidence,
        adjustment_percent: round((ratio - 1.0) * 100.0, 1),
        suggested_min,
        suggested_max,
        insights,
        action_plan,
        methodology_note: concat!(
            "建议来自本次甩枪与追踪的速度—精度权衡；甩枪点击倾向只作为一致性证据，不会单独改动推荐值。",
            "测试场按当前 sensitivity 与 m_yaw 生成候选增益，DPI、sensitivity 和 m_yaw 用于 eDPI 与 cm/360。",
            "浏览器指针锁定仍是相对模拟，最终请在 CS2 内复测确认。"
        ),
    })
}

const TRACKED_KEYS: [&str; 7] = [
    "ControlLeft",
    "KeyA",
    "KeyD",
    "KeyS",
    "KeyW",
    "ShiftLeft",
    "Space",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    RapidTap,
    CounterStrafe,
    Gameplay,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct InputEvent {
    pub code: String,
    pub event_type: EventType,
    pub timestamp_ms: f64,
}

fn default_keyboard_name() -> String {
    "Magnetic keyboard".to_owned()
}

#[derive(Debug, Deserialize, Serialize)]
pub struct InputLabRequest {
    #[serde(default = "default_keyboard_name")]
    pub keyboard_name: String,
    pub mode: InputMode,
    pub actuation_mm: f64,
    pub rapid_trigger_press_mm: f64,
    pub rapid_trigger_release_mm: f64,
    pub duration_ms: i64,
    pub events: Vec<InputEvent>,
}

#[derive(Debug, Serialize)]
pub struct KeyMetrics {
    pub presses: usize,
    pub mean_hold_ms: f64,
    pub jitter_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct InputLabResult {
    pub total_presses: usize,
    pub presses_per_second: f64,
    pub mean_hold_ms: f64,
    pub hold_jitter_ms: f64,
    pub mean_transition_ms: Option<f64>,
    pub overlap_ratio: f64,
    pub chatter_count: usize,
    pub stability_score: f64,
    pub recommendation: &'static str,
    pub per_key: BTreeMap<&'static str, KeyMetrics>,
    pub diagnosis: &'static str,
    pub diagnosis_label: &'static str,
    pub recommended_actuation_mm: f64,
    pub recommended_rt_press_mm: f64,
    pub recommended_rt_release_mm: f64,
    pub issues: Vec<String>,
    pub action_plan: Vec<String>,
    pub safety_notes: [&'static str; 2],
}

fn key_index(code: &str) -> Option<usize> {
    TRACKED_KEYS.iter().position(|key| *key == code)
}

impl InputLabRequest {
    fn validate(&self) -> Result<(), TrainingInputError> {
        match self.keyboard_name.chars().count() {
            0 => {
                return Err(issue(
                    "keyboard_name",
                    "string_too_short",
                    "String should have at least 1 character",
                ))
            }
            1..=100 => {}
            _ => {
                return Err(issue(
                    "keyboard_name",
                    "string_too_long",
                    "String should have at most 100 characters",
                ))
            }
        }
        bounds("actuation_mm", self.actuation_mm, 0.1, 4.0, false)?;
        bounds(
            "rapid_trigger_press_mm",
            self.rapid_trigger_press_mm,
            0.05,
            4.0,
            false,
        )?;
        bounds(
            "rapid_trigger_release_mm",
            self.rapid_trigger_release_mm,
            0.05,
            4.0,
            false,
        )?;
        bounds(
            "duration_ms",
            self.duration_ms as f64,
            3_000.0,
            MAX_DURATION_MS,
            false,
        )?;
        list_bounds("events", self.events.len(), 2, 20_000)?;
        for (index, event) in self.events.iter().enumerate() {
            if key_index(&event.code).is_none() {
                return Err(value_error(
                    format!("events.{index}.code"),
                    "包含不支持的按键",
                ));
            }
            bounds(
                "timestamp_ms",
                event.timestamp_ms,
                0.0,
                MAX_DURATION_MS,
                false,
            )
            .map_err(|mut err| {
                err.field = format!("events.{index}.{}", err.field);
                err
            })?;
        }
        if self
            .events
            .windows(2)
            .any(|pair| pair[1].timestamp_ms < pair[0].timestamp_ms)
        {
            return Err(value_error("events", "按键事件时间必须递增"));
        }
        Ok(())
    }
}

#[derive(Default)]
struct KeyState {
    active: Option<f64>,
    last_up: Option<f64>,
    presses: usize,
    holds: Vec<f64>,
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        compensated_sum(values.iter().copied()) / values.len() as f64
    }
}

fn population_stddev(values: &[f64], mean: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let squared_distances = values.iter().map(|value| {
        let distance = value - mean;
        distance * distance
    });
    (compensated_sum(squared_distances) / values.len() as f64).sqrt()
}

fn step(value: f64, delta: f64, lower_bound: f64) -> f64 {
    round((value + delta).clamp(lower_bound, 4.0), 2)
}

pub fn analyze_input_session(
    request: &InputLabRequest,
) -> Result<InputLabResult, TrainingInputError> {
    request.validate()?;
    let mut keys: [KeyState; TRACKED_KEYS.len()] = std::array::from_fn(|_| KeyState::default());
    let mut hold_durations = Vec::new();
    let mut transition_times = Vec::new();
    let mut overlap_count = 0;
    let mut chatter = 0;

    for event in &request.events {
        let index = key_index(&event.code).expect("validated event code");
        let timestamp = event.timestamp_ms;
        let key = &mut keys[index];
        match event.event_type {
            EventType::Down => {
                if key.active.is_some() {
                    chatter += 1;
                    continue;
                }
                key.presses += 1;
                key.active = Some(timestamp);
                let opposite = match event.code.as_str() {
                    "KeyA" => key_index("KeyD"),
                    "KeyD" => key_index("KeyA"),
                    _ => None,
                };
                if let Some(opposite) = opposite {
                    let opposite = &keys[opposite];
                    if let Some(started) = opposite.active {
                        overlap_count += 1;
                        transition_times.push(timestamp - started);
                    } else if let Some(released) = opposite.last_up {
                        transition_times.push(timestamp - released);
                    }
                }
            }
            EventType::Up => {
                let Some(started) = key.active.take() else {
                    chatter += 1;
                    continue;
                };
                let hold = (timestamp - started).max(0.0);
                hold_durations.push(hold);
                key.holds.push(hold);
                key.last_up = Some(timestamp);
            }
        }
    }
    for key in &mut keys {
        if let Some(started) = key.active {
            let hold = (request.duration_ms as f64 - started).max(0.0);
            hold_durations.push(hold);
            key.holds.push(hold);
        }
    }

    let total_presses = keys.iter().map(|key| key.presses).sum::<usize>();
    let mean_hold = mean(&hold_durations);
    let jitter = population_stddev(&hold_durations, mean_hold);
    let mean_transition = (!transition_times.is_empty()).then(|| mean(&transition_times));
    let overlap = if transition_times.is_empty() {
        0.0
    } else {
        overlap_count as f64 / transition_times.len() as f64
    };
    let stability = (100.0
        - (jitter / 3.0).min(45.0)
        - (chatter as f64 * 5.0).min(30.0)
        - (overlap * 35.0).min(25.0))
    .max(0.0);

    let mut actuation = request.actuation_mm;
    let mut press = request.rapid_trigger_press_mm;
    let mut release = request.rapid_trigger_release_mm;
    let (diagnosis, label, recommendation, details) =
        if chatter > 0 {
            actuation = step(actuation, 0.10, 0.1);
            press = step(press, 0.05, 0.05);
            release = step(release, 0.05, 0.05);
            ("unstable", "检测到误触或重复边沿",
         "检测到异常重复边沿；先提高触发与 RT 行程来消除误触，再逐步向下寻找稳定下限。",
         format!("记录到 {chatter} 次重复按下或缺失抬起，当前阈值可能过于激进或键盘需要重新校准。"))
        } else if jitter > 35.0 {
            press = step(press, 0.05, 0.05);
            release = step(release, 0.05, 0.05);
            (
                "inconsistent",
                "按键边沿稳定性不足",
                "将 RT 按下/抬起距离各增加 0.05 mm，稳定后再决定是否回调。",
                format!("保持时间标准差为 {jitter:.1} ms，微小手指运动可能被识别为状态变化。"),
            )
        } else if request.mode == InputMode::CounterStrafe && overlap > 0.25 {
            release = step(release, -0.05, 0.05);
            let percentage = overlap * 100.0;
            (
                "overlap",
                "A/D 同时按下偏多",
                "在没有抖动的前提下，将释放 RT 行程降低 0.05 mm，并练习先松开再反向按下。",
                format!("反向切换中有 {percentage:.1}% 出现 A/D 重叠，松键边沿或手法需要更干净。"),
            )
        } else if let Some(transition) = mean_transition.filter(|transition| *transition > 90.0) {
            actuation = step(actuation, -0.10, 0.2);
            press = step(press, -0.05, 0.05);
            (
                "slow",
                "方向切换偏慢",
                "小幅降低初始触发与 RT 按下行程，每次只改一档并复测误触。",
                format!("平均方向切换为 {transition:.1} ms；在边沿稳定的前提下仍有提速空间。"),
            )
        } else {
            (
                "balanced",
                "当前参数稳定",
                "保持当前数值；不要仅为追求更小的毫米数牺牲稳定性。",
                "未检测到明显重复边沿、过量 A/D 重叠或异常切换延迟。".to_owned(),
            )
        };

    let starting_plan = if (actuation, press, release)
        != (
            request.actuation_mm,
            request.rapid_trigger_press_mm,
            request.rapid_trigger_release_mm,
        ) {
        format!(
            "建议起点：触发 {} mm，RT 按下 {} mm，RT 抬起 {} mm。",
            general(actuation),
            general(press),
            general(release)
        )
    } else {
        "维持当前触发与 Rapid Trigger 数值，不做无依据的激进下调。".to_owned()
    };
    let per_key = TRACKED_KEYS
        .into_iter()
        .zip(&keys)
        .filter(|(_, key)| key.presses > 0)
        .map(|(code, key)| {
            let average = mean(&key.holds);
            (
                code,
                KeyMetrics {
                    presses: key.presses,
                    mean_hold_ms: round(average, 2),
                    jitter_ms: round(population_stddev(&key.holds, average), 2),
                },
            )
        })
        .collect();

    Ok(InputLabResult {
        total_presses,
        presses_per_second: round(
            total_presses as f64 / (request.duration_ms as f64 / 1000.0),
            2,
        ),
        mean_hold_ms: round(mean_hold, 2),
        hold_jitter_ms: round(jitter, 2),
        mean_transition_ms: mean_transition.map(|mean| round(mean, 2)),
        overlap_ratio: round(overlap, 3),
        chatter_count: chatter,
        stability_score: round(stability, 1),
        recommendation,
        per_key,
        diagnosis,
        diagnosis_label: label,
        recommended_actuation_mm: actuation,
        recommended_rt_press_mm: press,
        recommended_rt_release_mm: release,
        issues: vec![details],
        action_plan: vec![
            starting_plan,
            "每次只调整 0.05–0.10 mm，并使用同一测试模式复测，便于判断因果。".to_owned(),
            "先用 30 秒测试排除偶然波动，再进入 CS2 验证急停、长按稳定和误触。".to_owned(),
        ],
        safety_notes: [
            "CS2 官匹中请关闭 Snap Tap、Rapid Tap、Snappy Tappy、SOCD/LKP 等自动反向输入功能。",
            "普通 Rapid Trigger 可用于缩短按键复位，但建议只在 WASD 等确有收益的按键启用。",
        ],
    })
}
