//! CS2RPL01 encoder adapted from the checked-in demoparser2-v0.41.4.patch.
//! Read exactly one row group with the same pinned Parquet backend as the shipped wheel.

use parquet::{
    file::reader::{FileReader, SerializedFileReader},
    record::Field as ParquetField,
};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

pub(crate) const MAX_SAMPLE_TICKS: usize = 40_000;
const MAX_ROW_GROUP_ROWS: i64 = 500_000;
const MAX_ROW_GROUP_BYTES: i64 = 128 * 1024 * 1024;
const MAX_FOOTER_BYTES: u64 = 8 * 1024 * 1024;

fn check_footer(file: &mut File) -> io::Result<()> {
    const FOOTER: u64 = 8;
    let file_len = file.metadata()?.len();
    if file_len < FOOTER + 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short Parquet file",
        ));
    }
    file.seek(SeekFrom::End(-(FOOTER as i64)))?;
    let mut footer = [0u8; FOOTER as usize];
    file.read_exact(&mut footer)?;
    let metadata_size = u64::from(u32::from_le_bytes(footer[..4].try_into().unwrap()));
    if &footer[4..] != b"PAR1"
        || metadata_size > MAX_FOOTER_BYTES
        || metadata_size > file_len - FOOTER - 4
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported Parquet footer size or format",
        ));
    }
    file.rewind()
}

const REPLAY_BINARY_MAGIC: &[u8; 8] = b"CS2RPL01";
const REPLAY_BINARY_VERSION: u16 = 1;

#[derive(Default)]
struct ReplayBinaryRow {
    steamid: u64,
    name_id: u32,
    team_num: i8,
    flags: u8,
    health: u16,
    armor: u16,
    balance: i32,
    equipment_value: i32,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    flash_duration: f32,
    inventory_id: u32,
    active_weapon_id: u32,
    active_weapon_name_id: u32,
    player_color_id: u32,
}

fn replay_field_i64(field: &ParquetField) -> Option<i64> {
    match field {
        ParquetField::Byte(value) => Some(i64::from(*value)),
        ParquetField::Short(value) => Some(i64::from(*value)),
        ParquetField::Int(value) => Some(i64::from(*value)),
        ParquetField::Long(value) => Some(*value),
        ParquetField::UByte(value) => Some(i64::from(*value)),
        ParquetField::UShort(value) => Some(i64::from(*value)),
        ParquetField::UInt(value) => Some(i64::from(*value)),
        ParquetField::ULong(value) => i64::try_from(*value).ok(),
        _ => None,
    }
}

fn replay_field_u64(field: &ParquetField) -> Option<u64> {
    match field {
        ParquetField::Byte(value) => u64::try_from(*value).ok(),
        ParquetField::Short(value) => u64::try_from(*value).ok(),
        ParquetField::Int(value) => u64::try_from(*value).ok(),
        ParquetField::Long(value) => u64::try_from(*value).ok(),
        ParquetField::UByte(value) => Some(u64::from(*value)),
        ParquetField::UShort(value) => Some(u64::from(*value)),
        ParquetField::UInt(value) => Some(u64::from(*value)),
        ParquetField::ULong(value) => Some(*value),
        _ => None,
    }
}

fn replay_field_f32(field: &ParquetField) -> Option<f32> {
    match field {
        ParquetField::Float16(value) => Some(f32::from(*value)),
        ParquetField::Float(value) => Some(*value),
        ParquetField::Double(value) => Some(*value as f32),
        other => replay_field_i64(other).map(|value| value as f32),
    }
}

fn replay_field_bool(field: &ParquetField) -> Option<bool> {
    match field {
        ParquetField::Bool(value) => Some(*value),
        other => replay_field_i64(other).map(|value| value != 0),
    }
}

fn replay_intern_string(
    value: &str,
    ids: &mut HashMap<String, u32>,
    values: &mut Vec<String>,
) -> u32 {
    if value.is_empty() {
        return 0;
    }
    if let Some(id) = ids.get(value) {
        return *id;
    }
    let id = values.len() as u32;
    let owned = value.to_string();
    ids.insert(owned.clone(), id);
    values.push(owned);
    id
}

fn replay_intern_field(
    field: &ParquetField,
    ids: &mut HashMap<String, u32>,
    values: &mut Vec<String>,
    stringify_number: bool,
) -> u32 {
    match field {
        ParquetField::Str(value) => replay_intern_string(value, ids, values),
        ParquetField::Bytes(value) => std::str::from_utf8(value.data())
            .ok()
            .map(|text| replay_intern_string(text, ids, values))
            .unwrap_or(0),
        other if stringify_number => replay_field_i64(other)
            .map(|number| replay_intern_string(&number.to_string(), ids, values))
            .unwrap_or(0),
        _ => 0,
    }
}

fn replay_packet_align(packet: &mut Vec<u8>, alignment: usize) {
    let padding = (alignment - (packet.len() % alignment)) % alignment;
    packet.resize(packet.len() + padding, 0);
}

fn replay_packet_u16(packet: &mut Vec<u8>, value: u16) {
    packet.extend_from_slice(&value.to_le_bytes());
}

fn replay_packet_u32(packet: &mut Vec<u8>, value: u32) {
    packet.extend_from_slice(&value.to_le_bytes());
}

fn replay_packet_i32(packet: &mut Vec<u8>, value: i32) {
    packet.extend_from_slice(&value.to_le_bytes());
}

fn replay_packet_u64(packet: &mut Vec<u8>, value: u64) {
    packet.extend_from_slice(&value.to_le_bytes());
}

fn replay_packet_f32(packet: &mut Vec<u8>, value: f32) {
    packet.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn replay_parquet_binary_packet(
    parquet_path: &Path,
    row_group: usize,
    sample_ticks: &[i32],
    mut metadata: serde_json::Value,
) -> Result<Vec<u8>, String> {
    if sample_ticks.is_empty() || sample_ticks.len() > MAX_SAMPLE_TICKS {
        return Err("replay sample tick count is outside the supported range".to_owned());
    }
    let mut file =
        File::open(parquet_path).map_err(|error| format!("open replay parquet failed: {error}"))?;
    check_footer(&mut file).map_err(|error| format!("invalid replay parquet footer: {error}"))?;
    let reader = SerializedFileReader::new(file)
        .map_err(|error| format!("read replay parquet metadata failed: {error}"))?;
    if row_group >= reader.num_row_groups() {
        return Err(format!("row_group {row_group} is out of range"));
    }
    let row_group_reader = reader
        .get_row_group(row_group)
        .map_err(|error| format!("open replay row group failed: {error}"))?;
    let group = row_group_reader.metadata();
    if !(0..=MAX_ROW_GROUP_ROWS).contains(&group.num_rows())
        || !(0..=MAX_ROW_GROUP_BYTES).contains(&group.total_byte_size())
        || group.num_columns() > 64
    {
        return Err("replay row group exceeds the supported bounds".to_owned());
    }
    let rows = row_group_reader
        .get_row_iter(None)
        .map_err(|error| format!("open replay row iterator failed: {error}"))?;
    let tick_to_frame: HashMap<i32, usize> = sample_ticks
        .iter()
        .copied()
        .enumerate()
        .map(|(index, tick)| (tick, index))
        .collect();
    let mut rows_by_frame: Vec<Vec<ReplayBinaryRow>> =
        (0..sample_ticks.len()).map(|_| Vec::new()).collect();
    let mut string_ids = HashMap::default();
    let mut strings = vec![String::new()];
    string_ids.insert(String::new(), 0);

    for row_result in rows {
        let row = row_result.map_err(|error| format!("read replay row failed: {error}"))?;
        let mut tick = None;
        let mut packed = ReplayBinaryRow::default();
        for (name, field) in row.into_columns() {
            match name.as_str() {
                "tick" => {
                    tick = replay_field_i64(&field).and_then(|value| i32::try_from(value).ok())
                }
                "steamid" => packed.steamid = replay_field_u64(&field).unwrap_or(0),
                "name" => {
                    packed.name_id =
                        replay_intern_field(&field, &mut string_ids, &mut strings, false)
                }
                "team_num" => {
                    packed.team_num = replay_field_i64(&field)
                        .and_then(|value| i8::try_from(value).ok())
                        .unwrap_or(-1)
                }
                "X" => packed.x = replay_field_f32(&field).unwrap_or(0.0),
                "Y" => packed.y = replay_field_f32(&field).unwrap_or(0.0),
                "Z" => packed.z = replay_field_f32(&field).unwrap_or(0.0),
                "yaw" => packed.yaw = replay_field_f32(&field).unwrap_or(0.0),
                "is_alive" if replay_field_bool(&field).unwrap_or(false) => packed.flags |= 1,
                "has_helmet" if replay_field_bool(&field).unwrap_or(false) => packed.flags |= 2,
                "has_defuser" if replay_field_bool(&field).unwrap_or(false) => packed.flags |= 4,
                "has_c4" if replay_field_bool(&field).unwrap_or(false) => packed.flags |= 8,
                "health" => {
                    packed.health = replay_field_i64(&field)
                        .and_then(|value| u16::try_from(value.max(0)).ok())
                        .unwrap_or(0)
                }
                "armor" => {
                    packed.armor = replay_field_i64(&field)
                        .and_then(|value| u16::try_from(value.max(0)).ok())
                        .unwrap_or(0)
                }
                "balance" => {
                    packed.balance = replay_field_i64(&field)
                        .and_then(|value| i32::try_from(value.max(0)).ok())
                        .unwrap_or(0)
                }
                "current_equip_value" => {
                    packed.equipment_value = replay_field_i64(&field)
                        .and_then(|value| i32::try_from(value.max(0)).ok())
                        .unwrap_or(0)
                }
                "inventory" => {
                    packed.inventory_id =
                        replay_intern_field(&field, &mut string_ids, &mut strings, false)
                }
                "active_weapon" => {
                    packed.active_weapon_id =
                        replay_intern_field(&field, &mut string_ids, &mut strings, false)
                }
                "active_weapon_name" => {
                    packed.active_weapon_name_id =
                        replay_intern_field(&field, &mut string_ids, &mut strings, false)
                }
                "flash_duration" => {
                    packed.flash_duration = replay_field_f32(&field).unwrap_or(0.0).max(0.0)
                }
                "player_color" => {
                    packed.player_color_id =
                        replay_intern_field(&field, &mut string_ids, &mut strings, true)
                }
                _ => {}
            }
        }
        if packed.team_num != 2 && packed.team_num != 3 {
            continue;
        }
        if let Some(frame_index) = tick.and_then(|value| tick_to_frame.get(&value).copied()) {
            rows_by_frame[frame_index].push(packed);
        }
    }

    let mut frame_offsets = Vec::with_capacity(sample_ticks.len() + 1);
    let row_count = rows_by_frame.iter().map(Vec::len).sum::<usize>();
    let mut flat_rows = Vec::with_capacity(row_count);
    for frame_rows in rows_by_frame {
        frame_offsets.push(flat_rows.len() as u32);
        flat_rows.extend(frame_rows);
    }
    frame_offsets.push(flat_rows.len() as u32);

    let metadata_object = metadata
        .as_object_mut()
        .ok_or_else(|| "metadata JSON must be an object".to_string())?;
    metadata_object.insert(
        "protocol".to_string(),
        serde_json::Value::String("cs2-replay-binary-v1".to_string()),
    );
    metadata_object.insert(
        "frame_count".to_string(),
        serde_json::Value::from(sample_ticks.len() as u64),
    );
    metadata_object.insert(
        "row_count".to_string(),
        serde_json::Value::from(flat_rows.len() as u64),
    );
    metadata_object.insert(
        "strings".to_string(),
        serde_json::Value::Array(strings.into_iter().map(serde_json::Value::String).collect()),
    );
    // Keep byte ordering stable even if the desktop enables serde_json's preserve_order feature.
    metadata.sort_all_objects();
    let header = serde_json::to_vec(&metadata)
        .map_err(|error| format!("serialize replay binary metadata failed: {error}"))?;
    let mut packet =
        Vec::with_capacity(header.len() + sample_ticks.len() * 8 + flat_rows.len() * 62 + 64);
    packet.extend_from_slice(REPLAY_BINARY_MAGIC);
    replay_packet_u16(&mut packet, REPLAY_BINARY_VERSION);
    replay_packet_u16(&mut packet, 0);
    replay_packet_u32(&mut packet, header.len() as u32);
    packet.extend_from_slice(&header);
    replay_packet_align(&mut packet, 4);
    for tick in sample_ticks {
        replay_packet_i32(&mut packet, *tick);
    }
    for offset in &frame_offsets {
        replay_packet_u32(&mut packet, *offset);
    }
    replay_packet_align(&mut packet, 8);
    for row in &flat_rows {
        replay_packet_u64(&mut packet, row.steamid);
    }
    for row in &flat_rows {
        replay_packet_u32(&mut packet, row.name_id);
    }
    for row in &flat_rows {
        packet.push(row.team_num as u8);
    }
    for row in &flat_rows {
        packet.push(row.flags);
    }
    replay_packet_align(&mut packet, 2);
    for row in &flat_rows {
        replay_packet_u16(&mut packet, row.health);
    }
    for row in &flat_rows {
        replay_packet_u16(&mut packet, row.armor);
    }
    replay_packet_align(&mut packet, 4);
    for row in &flat_rows {
        replay_packet_i32(&mut packet, row.balance);
    }
    for row in &flat_rows {
        replay_packet_i32(&mut packet, row.equipment_value);
    }
    let f32_getters: [fn(&ReplayBinaryRow) -> f32; 5] = [
        |row: &ReplayBinaryRow| row.x,
        |row: &ReplayBinaryRow| row.y,
        |row: &ReplayBinaryRow| row.z,
        |row: &ReplayBinaryRow| row.yaw,
        |row: &ReplayBinaryRow| row.flash_duration,
    ];
    for getter in f32_getters {
        for row in &flat_rows {
            replay_packet_f32(&mut packet, getter(row));
        }
    }
    let u32_getters: [fn(&ReplayBinaryRow) -> u32; 4] = [
        |row: &ReplayBinaryRow| row.inventory_id,
        |row: &ReplayBinaryRow| row.active_weapon_id,
        |row: &ReplayBinaryRow| row.active_weapon_name_id,
        |row: &ReplayBinaryRow| row.player_color_id,
    ];
    for getter in u32_getters {
        for row in &flat_rows {
            replay_packet_u32(&mut packet, getter(row));
        }
    }
    Ok(packet)
}
