use super::{visit_rows, Pass, RawRow, RowKind, TickWindow};
use crate::Failure;
use parser::{parse_demo::DemoOutput, second_pass::variants::VarVec};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn build(
    parsed: &DemoOutput,
    window: TickWindow,
    tick_rate: f64,
) -> Result<Vec<Value>, Failure> {
    let mut entities = BTreeMap::<Option<i64>, Vec<FireRow<'_>>>::new();
    visit_rows(parsed, Pass::Utility, window, |tick, row| {
        if matches!(row.kind, RowKind::Inferno) {
            entities
                .entry(row.entity())
                .or_default()
                .push(FireRow::read(tick, row));
        }
        Ok(())
    })?;
    let gap = (tick_rate * 3.0).max(64.0) as i32;
    let mut tracks = Vec::new();
    for (id, mut frames) in entities {
        frames.sort_by_key(|row| row.tick);
        let id_text = id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "None".to_owned());
        for group in frames.chunk_by(|previous, next| next.tick - previous.tick <= gap) {
            if let Some(track) = track(group, id, &id_text, i64::from(window.end_tick), tick_rate) {
                tracks.push(track);
            }
        }
    }
    tracks.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok(tracks.into_iter().map(|(_, track)| track).collect())
}

struct FireRow<'a> {
    tick: i32,
    begin: Option<i64>,
    lifetime: Option<f64>,
    name: Option<&'a str>,
    steam_id: Option<u64>,
    positions: &'a [[f32; 3]],
    burning: &'a [bool],
    count: Option<i64>,
}

impl<'a> FireRow<'a> {
    fn read(tick: i32, row: RawRow<'_, 'a>) -> Self {
        let positions = match row.field("m_firePositions") {
            Some(VarVec::VecXYZList(points)) => points[row.index].as_deref().unwrap_or_default(),
            _ => &[],
        };
        let burning = match row.field("m_bFireIsBurning") {
            Some(VarVec::BoolVec(flags)) => flags[row.index].as_deref().unwrap_or_default(),
            _ => &[],
        };
        let (name, steam_id) = row.thrower();
        let lifetime = match row.field("m_nFireLifetime") {
            Some(VarVec::F32(values)) => values[row.index].map(f64::from),
            _ => row.integer("m_nFireLifetime").map(|value| value as f64),
        };
        Self {
            tick,
            positions,
            burning,
            lifetime,
            begin: row.integer("m_nFireEffectTickBegin"),
            count: row.integer("m_fireCount"),
            name,
            steam_id,
        }
    }

    fn cells(&self) -> Vec<[f64; 4]> {
        let count = self
            .count
            .map_or(self.positions.len(), |count| count.max(0) as usize);
        let mut cells = Vec::new();
        for (index, point) in self.positions.iter().take(count).enumerate() {
            if self.burning.get(index) == Some(&false) {
                continue;
            }
            let point = point.map(f64::from);
            if point.iter().any(|coordinate| !coordinate.is_finite())
                || point.iter().all(|coordinate| coordinate.abs() < 1e-6)
            {
                continue;
            }
            let [x, y, z] = point.map(|coordinate| {
                let rounded = (coordinate * 2.0).round_ties_even() / 2.0;
                if rounded == 0.0 {
                    0.0
                } else {
                    rounded
                }
            });
            cells.push([x, y, z, 1.0]);
        }
        cells.sort_by(|left, right| {
            left[0]
                .total_cmp(&right[0])
                .then_with(|| left[1].total_cmp(&right[1]))
                .then_with(|| left[2].total_cmp(&right[2]))
        });
        cells
    }
}

#[derive(Serialize)]
struct Sample {
    tick: i32,
    cells: Vec<[f64; 4]>,
}

fn track(
    frames: &[FireRow<'_>],
    id: Option<i64>,
    id_text: &str,
    end_tick: i64,
    tick_rate: f64,
) -> Option<((i64, String), Value)> {
    let mut samples = Vec::<Sample>::new();
    for frame in frames {
        let cells = frame.cells();
        if !cells.is_empty() && samples.last().is_none_or(|last| last.cells != cells) {
            samples.push(Sample {
                tick: frame.tick,
                cells,
            });
        }
    }
    let start = i64::from(samples.first()?.tick);
    let first = &frames[0];
    let start = start.min(first.begin.unwrap_or(start));
    let life_ticks = match first
        .lifetime
        .filter(|life| life.is_finite() && *life > 0.0)
    {
        Some(life) if life > 120.0 => life as i64,
        Some(life) => (life * tick_rate) as i64,
        None => (7.0 * tick_rate) as i64,
    };
    let last = i64::from(samples.last()?.tick);
    let end = end_tick.min(last.max(start.saturating_add(life_ticks)));
    let mut effect = json!({
        "id":format!("inferno:0:{start}:{id_text}"),"type":"inferno",
        "entity_id":id.map_or_else(|| json!(id_text), |id| json!(id)),
        "start_tick":start,"end_tick":end,"source":"cinferno_cells","samples":samples,
    });
    if let Some(thrower) = frames
        .iter()
        .find(|frame| frame.name.is_some() || frame.steam_id.is_some())
    {
        if let Some(name) = thrower.name {
            effect["thrower_name"] = json!(name);
        }
        if let Some(id) = thrower.steam_id {
            effect["thrower_steamid64"] = json!(id.to_string());
        }
    }
    Some(((start, id_text.to_owned()), effect))
}
