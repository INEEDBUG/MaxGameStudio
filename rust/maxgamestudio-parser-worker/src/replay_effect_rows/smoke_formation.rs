//! Preserve the legacy animation for seed-only smoke snapshots.

use super::{same_cells, SmokeSample};
use std::collections::{HashMap, HashSet, VecDeque};

const FORMATION_SECONDS: f64 = 1.2;
const FORMATION_STEPS: usize = 8;
const GRID_CENTER: f64 = 15.5;
const CELL_SIZE: f64 = 12.0;
const NEIGHBORS: [[i16; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

#[derive(Clone, Copy)]
struct Seed {
    position: [i16; 3],
    density: u8,
}

pub(super) fn merge_formation<'a>(
    mut samples: Vec<SmokeSample>,
    mut journals: impl Iterator<Item = &'a [u8]>,
    origin: [f64; 3],
    begin_tick: i64,
    tick_rate: f64,
) -> Vec<SmokeSample> {
    let Some(first) = samples.first() else {
        return samples;
    };
    // A journal with a changing occupancy count already supplies formation.
    if samples
        .iter()
        .skip(1)
        .any(|sample| sample.cells.len() != first.cells.len())
    {
        return samples;
    }
    let Some(seeds) = journals.find_map(first_seed_keyframe) else {
        return samples;
    };
    if seeds.is_empty() {
        return samples;
    }
    let seed_cells = project(&seeds, origin);
    if !samples
        .iter()
        .any(|sample| same_cells(&sample.cells, &seed_cells))
    {
        return samples;
    }

    let mut end_tick = begin_tick.saturating_add((FORMATION_SECONDS * tick_rate) as i64);
    if begin_tick < first.tick && first.tick <= end_tick {
        end_tick = first.tick;
    }
    if end_tick <= begin_tick {
        return samples;
    }

    let ordered = bfs_order(&seeds);
    let mut formation = Vec::with_capacity(FORMATION_STEPS);
    let duration = end_tick.saturating_sub(begin_tick) as f64;
    for step in 1..=FORMATION_STEPS {
        let fraction = step as f64 / FORMATION_STEPS as f64;
        let count = ((ordered.len() as f64 * fraction).round_ties_even() as usize).max(1);
        let subset = &ordered[..count];
        formation.push(SmokeSample {
            seq: step as i64 - FORMATION_STEPS as i64 - 1,
            tick: (begin_tick as f64 + duration * fraction).round_ties_even() as i64,
            cells: project(subset, origin),
            cell_size: CELL_SIZE,
            anchor_mode: "formation_bfs",
            voxel_update: None,
            voxel_count: Some(subset.len()),
        });
    }

    let final_cells = &formation[FORMATION_STEPS - 1].cells;
    samples.retain(|sample| sample.tick > end_tick || !same_cells(&sample.cells, final_cells));
    formation.append(&mut samples);
    formation
}

fn first_seed_keyframe(mut journal: &[u8]) -> Option<Vec<Seed>> {
    let mut first = None;
    while journal.len() >= 4 {
        let size = u16::from_le_bytes([journal[2], journal[3]]) as usize;
        let payload = journal.get(4..4 + size)?;
        if first.is_none() && payload.len() >= 3 && payload[1] == 3 {
            first = Some(payload);
        }
        // A later record with an overrun invalidates the entire journal.
        journal = &journal[4 + size..];
    }
    let payload = first?;
    let (entries, _) = payload[3..].as_chunks::<8>();
    Some(
        entries
            .iter()
            .take(usize::from(payload[2]))
            .map(|entry| Seed {
                position: [
                    i16::from(entry[0]),
                    i16::from(entry[1]),
                    i16::from(entry[2]),
                ],
                density: entry[3],
            })
            .collect(),
    )
}

fn bfs_order(seeds: &[Seed]) -> Vec<Seed> {
    let mut ordered = Vec::with_capacity(seeds.len());
    let Some(&center_seed) = seeds.iter().min_by_key(|seed| {
        // Squared distance to grid center, scaled by four for integral comparisons.
        seed.position
            .iter()
            .map(|&axis| (i32::from(axis) * 2 - 31).pow(2))
            .sum::<i32>()
    }) else {
        return ordered;
    };
    let by_position: HashMap<_, _> = seeds.iter().map(|&seed| (seed.position, seed)).collect();
    let mut seen = HashSet::with_capacity(by_position.len());
    let mut queue = VecDeque::new();
    for root in std::iter::once(center_seed).chain(seeds.iter().copied()) {
        if !seen.insert(root.position) {
            continue;
        }
        queue.push_back(root);
        while let Some(seed) = queue.pop_front() {
            ordered.push(seed);
            for neighbor in NEIGHBORS {
                let position = std::array::from_fn(|axis| seed.position[axis] + neighbor[axis]);
                if let Some(&next) = by_position.get(&position) {
                    if seen.insert(position) {
                        queue.push_back(next);
                    }
                }
            }
        }
    }
    ordered
}

fn project(seeds: &[Seed], origin: [f64; 3]) -> Vec<[f64; 4]> {
    let mut buckets = HashMap::new();
    let mut cells: Vec<[f64; 4]> = Vec::new();
    for seed in seeds {
        let [x, y, z]: [f64; 3] = std::array::from_fn(|axis| {
            (f64::from(seed.position[axis]) - GRID_CENTER) * CELL_SIZE + origin[axis]
        });
        // Preserve Python's bucket equality for signed zero and its insertion order.
        let bucket = |coordinate: f64| ((coordinate / CELL_SIZE).round_ties_even() + 0.0).to_bits();
        let key = [bucket(x), bucket(y)];
        let density = (f64::from(seed.density) / 5.0).clamp(0.15, 1.0);
        let quantize = |coordinate: f64| (coordinate * 2.0).round_ties_even() / 2.0;
        let cell = [quantize(x), quantize(y), quantize(z), density];
        let index = *buckets.entry(key).or_insert(cells.len());
        if index == cells.len() {
            cells.push(cell);
        } else if density > cells[index][3] || (density == cells[index][3] && z > cells[index][2]) {
            cells[index] = cell;
        }
    }
    cells
}
