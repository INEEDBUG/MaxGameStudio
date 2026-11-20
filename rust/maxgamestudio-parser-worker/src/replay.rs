use crate::{parser_inputs, replay_io, Failure};
use parquet::{
    basic::{Compression, ConvertedType, Repetition, Type as PhysicalType},
    data_type::{BoolType, ByteArray, ByteArrayType, DataType, FloatType, Int32Type, Int64Type},
    errors::{ParquetError, Result as ParquetResult},
    file::{
        properties::WriterProperties,
        writer::{SerializedColumnWriter, SerializedFileWriter},
    },
    schema::types::Type,
};
use parser::{
    first_pass::{
        parser_settings::rm_user_friendly_names, prop_controller::TICK_ID,
        read_bits::DemoParserError,
    },
    parse_demo::{DemoOutput, Parser, ParsingMode},
    second_pass::{parser_settings::create_huffman_lookup_table, variants::VarVec},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    path::PathBuf,
    sync::Arc,
};

const MAX_SAMPLE_TICKS: usize = 262_144;
// Keep generated rounds within the native replay packet reader's sample and row limits.
const MAX_ROUND_SAMPLE_TICKS: usize = 40_000;
const MAX_ROW_GROUP_ROWS: usize = 500_000;
const RAW_PLAYER_FIELDS: &[&str] = &[
    "X",
    "Y",
    "Z",
    "yaw",
    "name",
    "steamid",
    "team_num",
    "is_alive",
    "health",
    "armor",
    "has_helmet",
    "balance",
    "current_equip_value",
    "inventory",
    "active_weapon",
    "active_weapon_name",
    "has_defuser",
    "has_c4",
    "flash_duration",
    "player_color",
];

#[derive(Deserialize)]
pub(crate) struct WriteReplayRequest {
    path: PathBuf,
    output_path: PathBuf,
    tick_rate: f64,
    fps: f64,
    rounds: Vec<RoundRange>,
}

#[derive(Deserialize, Serialize)]
struct RoundRange {
    round_number: i32,
    start_tick: i32,
    end_tick: i32,
}

#[derive(Serialize)]
struct SampledRound {
    #[serde(flatten)]
    range: RoundRange,
    sample_ticks: Vec<i32>,
}

#[derive(Serialize)]
struct RowGroup {
    round_number: i32,
    row_group: usize,
    rows: usize,
}

pub(crate) fn write(request: WriteReplayRequest) -> Result<Value, Failure> {
    if !request.path.is_absolute() {
        return Err(Failure::request("expected an absolute demo path"));
    }
    let staged_path = &request.output_path;
    replay_io::validate_destination(
        staged_path,
        ".parquet.partial",
        "expected an absolute .parquet.partial output path",
    )?;

    let rounds = sample_rounds(request.rounds, request.tick_rate, request.fps)?;
    let mut wanted_ticks = Vec::new();
    let mut tick_to_round = HashMap::new();
    for round in &rounds {
        wanted_ticks.extend_from_slice(&round.sample_ticks);
        // Legacy tick ownership follows the last round in sorted sample order.
        for &tick in &round.sample_ticks {
            tick_to_round.insert(tick, round.range.round_number);
        }
    }
    let output = replay_io::with_demo_bytes(&request.path, |bytes| parse(bytes, wanted_ticks))?;
    let ticks = match output
        .df
        .get(&TICK_ID)
        .and_then(|column| column.data.as_ref())
    {
        Some(VarVec::I32(ticks)) => ticks,
        _ => {
            return Err(Failure::replay_parse(
                "replay parse did not return an int32 tick column",
            ))
        }
    };
    let mut indices_by_round = BTreeMap::<i32, Vec<usize>>::new();
    for (index, tick) in ticks.iter().enumerate() {
        if let Some(round) = tick.and_then(|tick| tick_to_round.get(&tick)) {
            indices_by_round.entry(*round).or_default().push(index);
        }
    }
    if indices_by_round.is_empty() {
        return Err(Failure::replay_parse(
            "replay parse returned no rows for requested ticks",
        ));
    }
    if indices_by_round.len() != rounds.len() {
        return Err(Failure::replay_parse(
            "replay parse omitted a requested round",
        ));
    }
    if indices_by_round
        .values()
        .any(|indices| indices.len() > MAX_ROW_GROUP_ROWS)
    {
        return Err(Failure::replay_parse(
            "replay row group exceeds the supported row count",
        ));
    }
    let columns: Vec<_> = output
        .prop_controller
        .prop_infos
        .iter()
        .filter_map(|prop| {
            output
                .df
                .get(&prop.id)
                .and_then(|column| column.data.as_ref())
                .map(|values| (prop.prop_friendly_name.as_str(), values))
        })
        .collect();

    let (row_groups, parquet_rows) = replay_io::stage(staged_path, |file| {
        write_parquet(file, &columns, indices_by_round)
            .map_err(|_| Failure::replay_write("could not encode replay parquet"))
    })?;
    Ok(json!({
        "rows": ticks.len(),
        "parquet_rows": parquet_rows,
        "row_groups": row_groups,
        "rounds": rounds,
    }))
}

fn sample_rounds(
    mut ranges: Vec<RoundRange>,
    tick_rate: f64,
    fps: f64,
) -> Result<Vec<SampledRound>, Failure> {
    if !tick_rate.is_finite() || tick_rate <= 0.0 || !fps.is_finite() || fps <= 0.0 {
        return Err(Failure::request(
            "tick rate and fps must be finite and positive",
        ));
    }
    if ranges.is_empty() {
        return Err(Failure::request("expected at least one replay round"));
    }
    ranges.sort_by_key(|range| (range.round_number, range.start_tick));
    let mut remaining = MAX_SAMPLE_TICKS;
    let mut last_round = 0;
    let mut rounds = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.round_number <= last_round
            || range.start_tick < 0
            || range.end_tick <= range.start_tick
        {
            return Err(Failure::request(
                "expected unique positive rounds with valid tick bounds",
            ));
        }
        last_round = range.round_number;
        let span = range.end_tick - range.start_tick;
        let duration = (span as f64 / tick_rate.max(0.001)).max(0.01);
        let frames = (duration * fps.max(0.01)).round_ties_even().max(1.0);
        if frames > MAX_ROUND_SAMPLE_TICKS as f64 {
            return Err(Failure::request(
                "replay round exceeds the sample tick limit",
            ));
        }
        if frames > remaining as f64 {
            return Err(Failure::request(
                "selected replay exceeds the sample tick limit",
            ));
        }
        let frames = frames as usize;
        remaining -= frames;
        let sample_ticks = if frames == 1 {
            vec![range.start_tick]
        } else {
            (0..frames)
                .map(|index| {
                    let fraction = index as f64 / (frames - 1) as f64;
                    let offset = (fraction * (span - 1) as f64).round_ties_even() as i32;
                    range.start_tick + offset
                })
                .collect()
        };
        rounds.push(SampledRound {
            range,
            sample_ticks,
        });
    }
    Ok(rounds)
}

fn parse(bytes: &[u8], wanted_ticks: Vec<i32>) -> Result<DemoOutput, Failure> {
    let wanted_props = RAW_PLAYER_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect();
    let real_props = rm_user_friendly_names(&wanted_props)
        .map_err(|_| Failure::replay_parse("native parser rejected replay properties"))?;
    let real_name_to_og_name = real_props.iter().cloned().zip(wanted_props).collect();
    let huffman = create_huffman_lookup_table();
    let inputs = parser::first_pass::parser_settings::ParserInputs {
        real_name_to_og_name,
        wanted_player_props: real_props,
        wanted_ticks,
        parse_ents: true,
        ..parser_inputs(&huffman)
    };
    Parser::new(inputs, ParsingMode::Normal)
        .parse_demo(bytes)
        .map_err(|error| {
            Failure::replay_parse(match error {
                DemoParserError::MalformedMessage => "malformed replay demo frame",
                DemoParserError::OutOfBitsError | DemoParserError::OutOfBytesError => {
                    "truncated replay demo"
                }
                _ => "native parser rejected the replay demo",
            })
        })
}

fn write_parquet(
    file: &mut File,
    columns: &[(&str, &VarVec)],
    indices_by_round: BTreeMap<i32, Vec<usize>>,
) -> ParquetResult<(Vec<RowGroup>, i64)> {
    let round = Type::primitive_type_builder("round_number", PhysicalType::INT32)
        .with_repetition(Repetition::REQUIRED)
        .build()?;
    let mut fields = vec![Arc::new(round)];
    for (name, values) in columns {
        fields.push(Arc::new(parquet_field(name, values)?));
    }
    let schema = Arc::new(
        Type::group_type_builder("replay")
            .with_fields(fields)
            .build()?,
    );
    let properties = Arc::new(
        WriterProperties::builder()
            .set_compression(Compression::ZSTD(Default::default()))
            .set_dictionary_enabled(true)
            .build(),
    );
    let mut writer = SerializedFileWriter::new(file, schema, properties)?;
    let mut groups = Vec::with_capacity(indices_by_round.len());
    for (row_group, (round_number, indices)) in indices_by_round.into_iter().enumerate() {
        let mut group = writer.next_row_group()?;
        let mut round_col = group.next_column()?.ok_or_else(missing_column)?;
        round_col.typed::<Int32Type>().write_batch(
            &vec![round_number; indices.len()],
            None,
            None,
        )?;
        round_col.close()?;
        for (_, values) in columns {
            let mut column = group.next_column()?.ok_or_else(missing_column)?;
            write_column(&mut column, values, &indices)?;
            column.close()?;
        }
        group.close()?;
        groups.push(RowGroup {
            round_number,
            row_group,
            rows: indices.len(),
        });
    }
    let num_rows = writer.close()?.file_metadata().num_rows();
    Ok((groups, num_rows))
}

fn missing_column() -> ParquetError {
    ParquetError::General("missing replay parquet column".into())
}

fn parquet_field(name: &str, values: &VarVec) -> ParquetResult<Type> {
    let (physical, converted) = match values {
        VarVec::Bool(_) => (PhysicalType::BOOLEAN, ConvertedType::NONE),
        VarVec::I32(_) => (PhysicalType::INT32, ConvertedType::NONE),
        VarVec::F32(_) => (PhysicalType::FLOAT, ConvertedType::NONE),
        VarVec::U32(_) => (PhysicalType::INT32, ConvertedType::UINT_32),
        VarVec::U64(_) => (PhysicalType::INT64, ConvertedType::UINT_64),
        VarVec::String(_) | VarVec::StringVec(_) => (PhysicalType::BYTE_ARRAY, ConvertedType::UTF8),
        _ => {
            return Err(ParquetError::General(
                "unsupported replay parquet column type".into(),
            ))
        }
    };
    Type::primitive_type_builder(name, physical)
        .with_repetition(Repetition::OPTIONAL)
        .with_converted_type(converted)
        .build()
}

fn write_column(
    column: &mut SerializedColumnWriter<'_>,
    values: &VarVec,
    indices: &[usize],
) -> ParquetResult<()> {
    match values {
        VarVec::Bool(source) => {
            write_nullable::<BoolType, _>(column, source, indices, |value| *value)
        }
        VarVec::I32(source) => {
            write_nullable::<Int32Type, _>(column, source, indices, |value| *value)
        }
        VarVec::F32(source) => {
            write_nullable::<FloatType, _>(column, source, indices, |value| *value)
        }
        VarVec::U32(source) => {
            write_nullable::<Int32Type, _>(column, source, indices, |value| *value as i32)
        }
        VarVec::U64(source) => {
            write_nullable::<Int64Type, _>(column, source, indices, |value| *value as i64)
        }
        VarVec::String(source) => {
            write_nullable::<ByteArrayType, _>(column, source, indices, |value| {
                ByteArray::from(value.as_str())
            })
        }
        // Inventories are compact JSON strings, including an empty JSON list for zero items.
        VarVec::StringVec(source) => {
            let rows = indices
                .iter()
                .map(|index| {
                    let value = serde_json::to_vec(source.get(*index).ok_or_else(short_column)?)
                        .map_err(|error| ParquetError::External(Box::new(error)))?;
                    Ok(ByteArray::from(value))
                })
                .collect::<ParquetResult<Vec<_>>>()?;
            column
                .typed::<ByteArrayType>()
                .write_batch(&rows, Some(&vec![1; rows.len()]), None)?;
            Ok(())
        }
        _ => Err(ParquetError::General(
            "unsupported replay parquet column type".into(),
        )),
    }
}

fn write_nullable<T: DataType, S>(
    column: &mut SerializedColumnWriter<'_>,
    source: &[Option<S>],
    indices: &[usize],
    encode: impl Fn(&S) -> T::T,
) -> ParquetResult<()> {
    let mut values = Vec::with_capacity(indices.len());
    let mut definitions = Vec::with_capacity(indices.len());
    for &index in indices {
        let value = source.get(index).ok_or_else(short_column)?;
        definitions.push(i16::from(value.is_some()));
        if let Some(value) = value {
            values.push(encode(value));
        }
    }
    column
        .typed::<T>()
        .write_batch(&values, Some(&definitions), None)?;
    Ok(())
}

fn short_column() -> ParquetError {
    ParquetError::General("replay parquet column has fewer values than the tick column".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::{
        file::reader::{FileReader, SerializedFileReader},
        record::Field,
    };

    fn range(round_number: i32, start_tick: i32, end_tick: i32) -> RoundRange {
        RoundRange {
            round_number,
            start_tick,
            end_tick,
        }
    }

    #[test]
    fn sampling_uses_legacy_half_even_and_excludes_the_end_tick() {
        let rounds = sample_rounds(vec![range(4, 20, 25), range(2, 10, 16)], 64.0, 32.0).unwrap();
        assert_eq!(rounds[0].range.round_number, 2);
        assert_eq!(rounds[0].sample_ticks, [10, 12, 15]); // Half-even at a 2.5-tick offset.
        assert_eq!(rounds[1].range.round_number, 4);
        assert_eq!(rounds[1].sample_ticks, [20, 24]); // Half-even frame count 2.5 -> 2.
    }

    #[test]
    fn sampling_limits_work_and_requires_distinct_rounds() {
        assert!(sample_rounds(vec![range(1, 0, i32::MAX)], 64.0, 32.0).is_err());
        assert!(sample_rounds(vec![range(1, 0, 80_002)], 64.0, 32.0).is_err());
        assert!(sample_rounds(vec![range(1, 0, 20), range(1, 20, 40)], 64.0, 32.0).is_err());
        assert!(sample_rounds(vec![range(1, 0, 20)], f64::INFINITY, 32.0).is_err());
    }

    #[test]
    fn parquet_group_bindings_keep_unsigned_null_and_inventory_values() {
        let mut file = tempfile::tempfile().unwrap();
        let ticks = VarVec::I32(vec![Some(20), Some(10)]);
        let identities = VarVec::U64(vec![Some(u64::MAX), None]);
        let inventories = VarVec::StringVec(vec![vec![], vec!["smoke".to_string()]]);
        let columns = [
            ("tick", &ticks),
            ("steamid", &identities),
            ("inventory", &inventories),
        ];
        let (groups, rows) = write_parquet(
            &mut file,
            &columns,
            BTreeMap::from([(2, vec![0]), (1, vec![1])]),
        )
        .unwrap();
        assert_eq!(rows, 2);
        assert_eq!(
            groups
                .iter()
                .map(|group| (group.round_number, group.row_group, group.rows))
                .collect::<Vec<_>>(),
            [(1, 0, 1), (2, 1, 1)]
        );

        let reader = SerializedFileReader::new(file).unwrap();
        let read_group = |index| {
            let group = reader.get_row_group(index).unwrap();
            let mut rows = group.get_row_iter(None).unwrap();
            let row = rows.next().unwrap().unwrap();
            assert!(rows.next().is_none());
            row.into_columns()
                .into_iter()
                .map(|(_, field)| field)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            read_group(0),
            [
                Field::Int(1),
                Field::Int(10),
                Field::Null,
                Field::Str("[\"smoke\"]".into())
            ]
        );
        assert_eq!(
            read_group(1),
            [
                Field::Int(2),
                Field::Int(20),
                Field::ULong(u64::MAX),
                Field::Str("[]".into())
            ]
        );
    }
}
