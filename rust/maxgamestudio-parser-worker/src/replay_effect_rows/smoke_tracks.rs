#[path = "smoke_formation.rs"]
mod formation;

use super::{
    smoke_projection::decode_smoke_projection_samples, visit_rows, Pass, RawRow, RowKind,
    TickWindow,
};
use crate::Failure;
use parser::{parse_demo::DemoOutput, second_pass::variants::VarVec};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
};

const CELL_SIZE: f64 = 12.0;

pub(super) fn build(
    parsed: &DemoOutput,
    window: TickWindow,
    tick_rate: f64,
) -> Result<(Vec<Value>, Vec<String>), Failure> {
    let mut entities = BTreeMap::<Option<i64>, Vec<SmokeRow<'_>>>::new();
    visit_rows(parsed, Pass::Utility, window, |tick, row| {
        if matches!(row.kind, RowKind::Smoke) && effect_started(&row) {
            entities
                .entry(row.entity())
                .or_default()
                .push(SmokeRow::read(tick, row));
        }
        Ok(())
    })?;
    let gap = (tick_rate * 5.0).max(64.0) as i64;
    let mut tracks = Vec::new();
    let mut warnings = Vec::new();
    for (id, mut rows) in entities {
        let id_text = id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "None".to_owned());
        rows.sort_by_key(|row| row.tick);
        for group in rows.chunk_by(|previous, next| next.tick - previous.tick <= gap) {
            let source = SmokeSource {
                id,
                id_text: &id_text,
                rows: group,
            };
            if let Some(track) = track(source, i64::from(window.end_tick), tick_rate, &mut warnings)
            {
                tracks.push(track);
            }
        }
    }
    tracks.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok((
        tracks.into_iter().map(|(_, track)| track).collect(),
        warnings,
    ))
}

fn effect_started(row: &RawRow<'_, '_>) -> bool {
    match row.field("m_bDidSmokeEffect") {
        Some(VarVec::Bool(flags)) => flags[row.index] == Some(true),
        Some(_) => row.integer("m_bDidSmokeEffect") == Some(1),
        None => true,
    }
}

struct SmokeRow<'a> {
    tick: i64,
    begin: Option<i64>,
    update: Option<i64>,
    origin: Option<[f64; 3]>,
    journal: Option<&'a [u8]>,
    name: Option<&'a str>,
    steam_id: Option<u64>,
}

impl<'a> SmokeRow<'a> {
    fn read(tick: i32, row: RawRow<'_, 'a>) -> Self {
        let origin = match row.field("m_vSmokeDetonationPos") {
            Some(VarVec::XYZVec(origins)) => origins[row.index]
                .map(|point| point.map(f64::from))
                .filter(|point| point.iter().all(|coordinate| coordinate.is_finite())),
            _ => None,
        };
        let journal = match row.field("m_VoxelFrameData") {
            Some(VarVec::U8Vec(rows)) => rows[row.index].as_deref().map(|data| {
                let size = row
                    .integer("m_nVoxelFrameDataSize")
                    .map_or(data.len(), |size| size.clamp(0, data.len() as i64) as usize);
                &data[..size]
            }),
            _ => None,
        };
        let (name, steam_id) = row.thrower();
        Self {
            tick: i64::from(tick),
            origin,
            journal,
            name,
            steam_id,
            begin: row.integer("m_nSmokeEffectTickBegin"),
            update: row.integer("m_nVoxelUpdate"),
        }
    }
}

struct SmokeSource<'a> {
    id: Option<i64>,
    id_text: &'a str,
    rows: &'a [SmokeRow<'a>],
}

#[derive(Serialize)]
struct SmokeSample {
    tick: i64,
    seq: i64,
    cells: Vec<[f64; 4]>,
    cell_size: f64,
    anchor_mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voxel_update: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    voxel_count: Option<usize>,
}

fn track(
    source: SmokeSource<'_>,
    end_tick: i64,
    tick_rate: f64,
    warnings: &mut Vec<String>,
) -> Option<((i64, String), Value)> {
    let rows = source.rows;
    let first = rows.first()?;
    let last = rows.last()?.tick;
    let oldest = first.tick - (tick_rate.max(64.0) * 2.0) as i64;
    let begin = first
        .begin
        .filter(|tick| (oldest..=last).contains(tick))
        .unwrap_or(first.tick);
    let origin = rows.iter().find_map(|row| row.origin)?;
    let anchors: BTreeMap<_, _> = rows
        .iter()
        .filter_map(|row| row.update.map(|update| (update, row.tick)))
        .collect();
    let samples = decode_segments(
        source.id_text,
        &segments(rows),
        &anchors,
        begin,
        origin,
        warnings,
    );
    if samples.is_empty() {
        return None;
    }
    let mut samples = formation::merge_formation(
        samples,
        rows.iter().filter_map(|row| row.journal),
        origin,
        begin,
        tick_rate,
    );
    samples.sort_by_key(|sample| (sample.tick, sample.seq));
    samples.dedup_by(|next, previous| same_cells(&next.cells, &previous.cells));
    let effect_start = samples.first()?.tick;
    let start = effect_start.min(begin);
    let end = end_tick.min(samples.last()?.tick.max(start + (18.0 * tick_rate) as i64));
    let mut effect = json!({
        "id":format!("smoke:0:{}:{effect_start}",source.id_text), "type":"smoke",
        "entity_id":source.id.map_or_else(|| json!(source.id_text), |id| json!(id)),
        "start_tick":start, "end_tick":end, "stable_origin":origin,
        "source":"smoke_voxels", "cell_size":CELL_SIZE, "samples":samples,
    });
    if let Some(thrower) = rows
        .iter()
        .find(|row| row.name.is_some() || row.steam_id.is_some())
    {
        if let Some(name) = thrower.name {
            effect["thrower_name"] = json!(name);
        }
        if let Some(id) = thrower.steam_id {
            effect["thrower_steamid64"] = json!(id.to_string());
        }
    }
    Some(((start, source.id_text.to_owned()), effect))
}

struct Segment<'a> {
    data: &'a [u8],
    tick: i64,
    update: Option<i64>,
    frames: Vec<(i64, Option<i64>)>,
}

fn segments<'a>(rows: &[SmokeRow<'a>]) -> Vec<Segment<'a>> {
    let mut segments = Vec::<Segment<'a>>::new();
    let mut previous_update = None;
    for row in rows {
        if row.update.is_some() && row.update == previous_update {
            continue;
        }
        let Some(data) = row.journal else { continue };
        if let Some(previous) = segments.last_mut() {
            if data.starts_with(previous.data) {
                previous.frames.extend(
                    geometry_frames(data, previous.data.len()).map(|seq| (seq, row.update)),
                );
                previous.data = data;
                previous.tick = row.tick;
                previous.update = row.update;
                previous_update = row.update;
                continue;
            }
            if previous.data.starts_with(data) {
                previous_update = row.update;
                continue;
            }
        }
        segments.push(Segment {
            data,
            tick: row.tick,
            update: row.update,
            frames: geometry_frames(data, 0)
                .map(|seq| (seq, row.update))
                .collect(),
        });
        previous_update = row.update;
    }
    segments
}

fn geometry_frames(bytes: &[u8], mut offset: usize) -> impl Iterator<Item = i64> + '_ {
    std::iter::from_fn(move || loop {
        let header = bytes.get(offset..offset + 4)?;
        let end = offset + 4 + usize::from(u16::from_le_bytes([header[2], header[3]]));
        let payload = bytes.get(offset + 4..end)?;
        offset = end;
        if matches!(payload, [_, 3, ..])
            || matches!(payload, [_, 2, lo, hi, ..] if *lo != 0 || *hi != 0)
        {
            return Some(i64::from(u16::from_le_bytes([header[0], header[1]])));
        }
    })
}

fn decode_segments(
    id: &str,
    segments: &[Segment<'_>],
    anchors: &BTreeMap<i64, i64>,
    begin: i64,
    origin: [f64; 3],
    warnings: &mut Vec<String>,
) -> Vec<SmokeSample> {
    let mut seen = HashSet::new();
    let mut samples = Vec::<SmokeSample>::new();
    for segment in segments {
        let limit = segment
            .update
            .map(|update| update.clamp(0, i64::from(u16::MAX)) as u16);
        let sequence = match decode_smoke_projection_samples(
            segment.data,
            segment.data.len(),
            origin,
            limit,
        ) {
            Ok(sequence) => sequence,
            Err(error) => {
                warnings.push(format!(
                    "smoke entity {id} tick {}: Rust voxel decode failed: {error}",
                    segment.tick
                ));
                continue;
            }
        };
        let count = sequence.len();
        for (index, projection) in sequence.into_iter().enumerate() {
            let seq = i64::from(projection.seq);
            if let Some(update) = segment.update.filter(|update| seq > *update) {
                warnings.push(format!(
                    "smoke entity {id} tick {}: decoded_seq {seq} > m_nVoxelUpdate {update}",
                    segment.tick
                ));
                continue;
            }
            let cells = projection.cells;
            if cells.is_empty() {
                continue;
            }
            let availability = segment
                .frames
                .get(index)
                .filter(|(frame, _)| *frame == seq)
                .map_or(segment.update, |(_, availability)| *availability);
            let threshold = seq.max(availability.unwrap_or(seq));
            let exposed = anchors
                .range(threshold..)
                .next()
                .map(|(update, _)| *update)
                .or(segment.update);
            let same_previous = samples
                .last()
                .is_some_and(|sample| same_cells(&sample.cells, &cells));
            let (tick, anchor_mode) = if seen.insert(seq) {
                if same_previous {
                    continue;
                }
                map_sequence(seq, anchors, begin)
            } else {
                if index + 1 != count || same_previous {
                    continue;
                }
                let tick = exposed
                    .and_then(|update| anchors.get(&update))
                    .copied()
                    .unwrap_or(segment.tick);
                (tick, "row")
            };
            samples.push(SmokeSample {
                tick,
                seq,
                cells,
                cell_size: CELL_SIZE,
                anchor_mode,
                voxel_update: segment.update.map(|_| exposed),
                voxel_count: None,
            });
        }
    }
    samples
}

fn map_sequence(seq: i64, anchors: &BTreeMap<i64, i64>, begin: i64) -> (i64, &'static str) {
    if let Some(tick) = anchors.get(&seq) {
        return (*tick, "anchor");
    }
    let lower = anchors.range(..seq).next_back();
    let upper = anchors.range(seq..).next();
    match (lower, upper) {
        (Some((&low, &first)), Some((&high, &last))) => {
            let ratio = (seq - low) as f64 / (high - low) as f64;
            (
                (first as f64 + (last - first) as f64 * ratio).round_ties_even() as i64,
                "interpolated",
            )
        }
        (None, Some((&high, &tick))) if high > 0 => {
            let ratio = (seq as f64 / high as f64).clamp(0.0, 1.0);
            (
                (begin as f64 + (tick - begin) as f64 * ratio).round_ties_even() as i64,
                "pre_anchor",
            )
        }
        (Some((_, &tick)), None) => (tick, "post_anchor"),
        _ => (begin, "begin"),
    }
}

fn same_cells(left: &[[f64; 4]], right: &[[f64; 4]]) -> bool {
    if left == right {
        return true;
    }
    if left.len() != right.len() {
        return false;
    }
    let order = |lhs: &[f64; 4], rhs: &[f64; 4]| {
        for (lhs, rhs) in lhs.iter().zip(rhs) {
            let order = lhs.partial_cmp(rhs).expect("finite smoke cell");
            if order != Ordering::Equal {
                return order;
            }
        }
        Ordering::Equal
    };
    let mut a = left.to_vec();
    let mut b = right.to_vec();
    a.sort_unstable_by(order);
    b.sort_unstable_by(order);
    a == b
}
