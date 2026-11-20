use crate::{parser_inputs, replay_io, Failure};
use parser::{
    first_pass::{
        parser_settings::{rm_user_friendly_names, ParserInputs},
        prop_controller::{GRENADE_TYPE_ID, TICK_ID},
    },
    parse_demo::{DemoOutput, Parser, ParsingMode},
    second_pass::{parser_settings::create_huffman_lookup_table, variants::VarVec},
};
use serde::{
    ser::{Error as _, SerializeMap, SerializeSeq},
    Deserialize, Serialize,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufWriter, Write},
    path::PathBuf,
};

const MAX_TICK_WINDOW: i64 = 40_000;
const MAX_RAW_BYTES: usize = 64 * 1024 * 1024;
const UTILITY_FIELDS: &[&str] = &[
    "m_firePositions",
    "m_fireParentPositions",
    "m_bFireIsBurning",
    "m_fireCount",
    "m_nFireEffectTickBegin",
    "m_nFireLifetime",
    "m_VoxelFrameData",
    "m_nVoxelFrameDataSize",
    "m_nVoxelUpdate",
    "m_vSmokeDetonationPos",
    "m_bDidSmokeEffect",
    "m_nSmokeEffectTickBegin",
];

#[derive(Deserialize)]
pub(crate) struct ExtractReplayEffectRowsRequest {
    path: PathBuf,
    output_path: PathBuf,
    #[serde(flatten)]
    window: TickWindow,
}

#[derive(Clone, Copy, Deserialize)]
struct TickWindow {
    start_tick: i32,
    end_tick: i32,
}

impl TickWindow {
    fn validate(self) -> Result<(), Failure> {
        let width = i64::from(self.end_tick) - i64::from(self.start_tick) + 1;
        if self.start_tick < 0 || !(1..=MAX_TICK_WINDOW).contains(&width) {
            return Err(Failure::request(
                "expected an inclusive replay effect window of at most 40000 ticks",
            ));
        }
        Ok(())
    }

    fn contains(self, tick: i32) -> bool {
        (self.start_tick..=self.end_tick).contains(&tick)
    }
}

#[derive(Clone, Copy)]
enum Pass {
    Utility,
    Trajectories,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RowKind {
    Smoke,
    Inferno,
    Trajectory,
}

impl Pass {
    fn row_kind(self, class: &str) -> Option<RowKind> {
        match (self, class) {
            (Self::Utility, "CSmokeGrenadeProjectile") => Some(RowKind::Smoke),
            (Self::Utility, "CInferno") => Some(RowKind::Inferno),
            (
                Self::Trajectories,
                "CSmokeGrenadeProjectile"
                | "CFlashbangProjectile"
                | "CHEGrenadeProjectile"
                | "CMolotovProjectile"
                | "CDecoyProjectile",
            ) => Some(RowKind::Trajectory),
            _ => None,
        }
    }
}

#[derive(Default, Serialize)]
struct RowCounts {
    smoke: usize,
    inferno: usize,
    trajectory: usize,
}

impl RowCounts {
    fn add(&mut self, kind: RowKind) {
        match kind {
            RowKind::Smoke => self.smoke += 1,
            RowKind::Inferno => self.inferno += 1,
            RowKind::Trajectory => self.trajectory += 1,
        }
    }
}

pub(crate) fn extract(request: ExtractReplayEffectRowsRequest) -> Result<Value, Failure> {
    request.window.validate()?;
    replay_io::validate_destination(
        &request.output_path,
        ".effects.jsonl.zst.partial",
        "expected an absolute .effects.jsonl.zst.partial output path",
    )?;
    let (rows, raw_bytes, file_bytes) = replay_io::with_demo_bytes(&request.path, |bytes| {
        replay_io::stage(&request.output_path, |file| {
            encode(file, bytes, request.window)
        })
    })?;
    Ok(json!({
        "format": "mgs-replay-effect-rows-v1",
        "compression": "zstd",
        "start_tick": request.window.start_tick,
        "end_tick": request.window.end_tick,
        "rows": rows,
        "uncompressed_bytes": raw_bytes,
        "file_bytes": file_bytes,
    }))
}

fn encode(
    file: &mut File,
    bytes: &[u8],
    window: TickWindow,
) -> Result<(RowCounts, usize, u64), Failure> {
    let encoder = zstd::stream::write::Encoder::new(file, 3).map_err(write_failure)?;
    let mut output = BoundedWrite {
        writer: BufWriter::new(encoder),
        limit: MAX_RAW_BYTES,
        written: 0,
    };
    let mut rows = RowCounts::default();
    let huffman = create_huffman_lookup_table();

    // The pinned parser uses mutually exclusive collectors for these modes.
    // Run each on the same stable file and release its columns before starting the next pass.
    for pass in [Pass::Utility, Pass::Trajectories] {
        let result = parse(bytes, &huffman, pass)?;
        write_rows(&mut output, &result, pass, window, &mut rows)?;
    }

    let raw_bytes = output.written;
    let encoder = output
        .writer
        .into_inner()
        .map_err(|error| write_failure(error.into_error()))?;
    let file = encoder.finish().map_err(write_failure)?;
    let file_bytes = file.metadata().map_err(write_failure)?.len();
    Ok((rows, raw_bytes, file_bytes))
}

fn parse(bytes: &[u8], huffman: &Vec<(u8, u8)>, pass: Pass) -> Result<DemoOutput, Failure> {
    let wanted_fields: Vec<String> = match pass {
        Pass::Utility => UTILITY_FIELDS
            .iter()
            .map(|field| (*field).to_owned())
            .collect(),
        Pass::Trajectories => Vec::new(),
    };
    let wanted_other_props = rm_user_friendly_names(&wanted_fields)
        .map_err(|_| Failure::replay_parse("native parser rejected replay effect properties"))?;
    let real_name_to_og_name = wanted_other_props
        .iter()
        .cloned()
        .zip(wanted_fields)
        .collect();
    let inputs = ParserInputs {
        real_name_to_og_name,
        wanted_other_props,
        parse_ents: true,
        parse_projectiles: true,
        parse_grenades: true,
        parse_infernos: matches!(pass, Pass::Utility),
        only_header: false,
        // Filter emitted rows below. Filtering the parser's input ticks would
        // alter smoke and inferno state transitions at the window boundaries.
        ..parser_inputs(huffman)
    };
    Parser::new(inputs, ParsingMode::Normal)
        .parse_demo(bytes)
        .map_err(|_| {
            Failure::replay_parse(match pass {
                Pass::Utility => "native parser rejected utility effect extraction",
                Pass::Trajectories => "native parser rejected projectile extraction",
            })
        })
}

fn write_rows(
    mut output: impl Write,
    parsed: &DemoOutput,
    pass: Pass,
    window: TickWindow,
    counts: &mut RowCounts,
) -> Result<(), Failure> {
    if parsed.df.is_empty() {
        return Ok(());
    }
    let column = |id| parsed.df.get(&id).and_then(|column| column.data.as_ref());
    let (Some(VarVec::I32(ticks)), Some(VarVec::String(classes))) =
        (column(TICK_ID), column(GRENADE_TYPE_ID))
    else {
        return Err(Failure::replay_parse(
            "replay effect rows are missing their ticks or classes",
        ));
    };
    let columns = parsed
        .prop_controller
        .prop_infos
        .iter()
        .filter_map(|prop| {
            let source = parsed.df.get(&prop.id)?;
            source
                .data
                .as_ref()
                .map(|values| (prop.prop_friendly_name.as_str(), (source.len(), values)))
        })
        .collect::<BTreeMap<_, _>>();
    if ticks.len() != classes.len() || columns.values().any(|(len, _)| *len != ticks.len()) {
        return Err(Failure::replay_parse(
            "replay effect columns have inconsistent row counts",
        ));
    }
    for (index, (tick, class)) in ticks.iter().zip(classes).enumerate() {
        if !tick.is_some_and(|tick| window.contains(tick)) {
            continue;
        }
        let Some(kind) = class.as_deref().and_then(|class| pass.row_kind(class)) else {
            continue;
        };
        serde_json::to_writer(
            &mut output,
            &RawRow {
                kind,
                columns: &columns,
                index,
            },
        )
        .map_err(|error| match error.io_error_kind() {
            Some(kind) => write_failure(kind.into()),
            None => Failure::replay_parse("unsupported replay effect column type"),
        })?;
        output.write_all(b"\n").map_err(write_failure)?;
        counts.add(kind);
    }
    Ok(())
}

struct RawRow<'a> {
    kind: RowKind,
    columns: &'a BTreeMap<&'a str, (usize, &'a VarVec)>,
    index: usize,
}

impl Serialize for RawRow<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.columns.len() + 1))?;
        map.serialize_entry("kind", &self.kind)?;
        for (name, (_, values)) in self.columns {
            map.serialize_entry(name, &RawCell(values, self.index))?;
        }
        map.end()
    }
}

struct RawCell<'a>(&'a VarVec, usize);

impl Serialize for RawCell<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let index = self.1;
        match self.0 {
            VarVec::I32(values) => values[index].serialize(serializer),
            VarVec::U32(values) => values[index].serialize(serializer),
            VarVec::U64(values) => values[index].serialize(serializer),
            VarVec::Bool(values) => values[index].serialize(serializer),
            VarVec::F32(values) => values[index].map(f64::from).serialize(serializer),
            VarVec::String(values) => values[index].serialize(serializer),
            VarVec::U8Vec(values) => values[index].serialize(serializer),
            VarVec::BoolVec(values) => values[index].serialize(serializer),
            VarVec::XYZVec(values) => values[index]
                .map(|point| point.map(f64::from))
                .serialize(serializer),
            VarVec::VecXYZList(values) => {
                values[index].as_deref().map(Points).serialize(serializer)
            }
            _ => Err(S::Error::custom("unsupported replay effect column")),
        }
    }
}

struct Points<'a>(&'a [[f32; 3]]);

impl Serialize for Points<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for point in self.0 {
            sequence.serialize_element(&point.map(f64::from))?;
        }
        sequence.end()
    }
}

struct BoundedWrite<W> {
    writer: W,
    limit: usize,
    written: usize,
}

impl<W: Write> Write for BoundedWrite<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.written {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        let count = self.writer.write(bytes)?;
        self.written += count;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

fn write_failure(error: io::Error) -> Failure {
    let detail = if error.kind() == io::ErrorKind::FileTooLarge {
        "raw replay effect output exceeds 64 MiB"
    } else {
        "could not write compressed replay effect rows"
    };
    Failure::replay_write(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_rows_keep_exact_floats_unsigned_ids_nullable_cells_and_journals() {
        let floating = VarVec::F32(vec![Some(0.2), None]);
        let unsigned = VarVec::U64(vec![None, Some(u64::MAX)]);
        let bytes = VarVec::U8Vec(vec![Some(vec![0, 71, 255]), None]);
        let cells = VarVec::VecXYZList(vec![None, Some(vec![[1.0, 0.2, -3.0]])]);
        let columns = BTreeMap::from([
            ("x", (2, &floating)),
            ("steamid", (2, &unsigned)),
            ("m_VoxelFrameData", (2, &bytes)),
            ("m_firePositions", (2, &cells)),
        ]);
        let smoke = serde_json::to_value(RawRow {
            kind: RowKind::Smoke,
            columns: &columns,
            index: 0,
        })
        .unwrap();
        assert_eq!(smoke["kind"], "smoke");
        assert_eq!(smoke["x"], f64::from(0.2_f32));
        assert_eq!(smoke["m_VoxelFrameData"], json!([0, 71, 255]));
        assert!(smoke["m_firePositions"].is_null());
        let fire = serde_json::to_value(RawRow {
            kind: RowKind::Inferno,
            columns: &columns,
            index: 1,
        })
        .unwrap();
        assert_eq!(fire["steamid"], u64::MAX);
        assert_eq!(
            fire["m_firePositions"],
            json!([[1.0, f64::from(0.2_f32), -3.0]])
        );
        assert!(fire["m_VoxelFrameData"].is_null());
    }

    #[test]
    fn uncompressed_output_cap_does_not_write_a_partial_buffer() {
        let mut output = BoundedWrite {
            writer: Vec::new(),
            limit: 5,
            written: 0,
        };
        output.write_all(b"raw\n").unwrap();
        assert_eq!(
            output.write_all(b"{}\n").unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
        assert_eq!(output.writer, b"raw\n");
        assert_eq!(output.written, 4);
    }
}
