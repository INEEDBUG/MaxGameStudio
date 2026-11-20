//! One request per process. Stdout contains the protocol reply and stderr records failures.

use parser::first_pass::{
    parser::HEADER_ENDS_AT_BYTE,
    parser_settings::{FirstPassParser, ParserInputs},
    read_bits::DemoParserError,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    panic,
    path::{Path, PathBuf},
    process::ExitCode,
};

const PROTOCOL: u8 = 1;
const PARSER_COMMIT: &str = "db3ba5f7a1b02b8cb27d0063d52649883a98469c";
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_HEADER_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
struct Request {
    protocol: u8,
    #[serde(flatten)]
    action: Action,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    Probe,
    InspectHeader { path: PathBuf },
}

struct Failure {
    code: &'static str,
    stage: &'static str,
    detail: &'static str,
    exit: u8,
}

impl Failure {
    fn request(detail: &'static str) -> Self {
        Self {
            code: "invalid_request",
            stage: "request",
            detail,
            exit: 2,
        }
    }

    fn read(detail: &'static str) -> Self {
        Self {
            code: "unreadable_demo",
            stage: "open_demo",
            detail,
            exit: 3,
        }
    }

    fn header(detail: &'static str) -> Self {
        Self {
            code: "invalid_demo_header",
            stage: "parse_header",
            detail,
            exit: 4,
        }
    }

    fn panic() -> Self {
        Self {
            code: "native_panic",
            stage: "worker",
            detail: "parser worker panicked",
            exit: 70,
        }
    }
}

fn main() -> ExitCode {
    panic::set_hook(Box::new(|_| eprintln!("native parser worker panic")));
    let result =
        panic::catch_unwind(|| run(io::stdin().lock())).unwrap_or_else(|_| Err(Failure::panic()));
    let (reply, exit) = match result {
        Ok(result) => (
            json!({"protocol": PROTOCOL, "parser_commit": PARSER_COMMIT, "ok": true, "result": result}),
            0,
        ),
        Err(error) => {
            eprintln!("native parser worker {}: {}", error.stage, error.code);
            (
                json!({
                    "protocol": PROTOCOL, "parser_commit": PARSER_COMMIT, "ok": false,
                    "error": {"code":error.code, "stage":error.stage, "detail":error.detail}
                }),
                error.exit,
            )
        }
    };
    let mut out = io::stdout().lock();
    if serde_json::to_writer(&mut out, &reply).is_err()
        || out.write_all(b"\n").is_err()
        || out.flush().is_err()
    {
        eprintln!("native parser worker output failed");
        return ExitCode::from(74);
    }
    ExitCode::from(exit)
}

fn run(input: impl Read) -> Result<Value, Failure> {
    let mut bytes = Vec::new();
    input
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::request("could not read request"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(Failure::request("request exceeds 64 KiB"));
    }
    let request: Request = serde_json::from_slice(&bytes)
        .map_err(|_| Failure::request("expected one parser request object"))?;
    if request.protocol != PROTOCOL {
        return Err(Failure::request("unsupported parser worker protocol"));
    }
    match request.action {
        Action::Probe => {
            Ok(json!({"actions": ["probe", "inspect_header"], "upstream_tag": "v0.41.4"}))
        }
        Action::InspectHeader { path } => inspect_header(&path),
    }
}

fn inspect_header(path: &Path) -> Result<Value, Failure> {
    let metadata = fs::metadata(path).map_err(|_| Failure::read("could not open demo"))?;
    if !metadata.is_file() {
        return Err(Failure::read("expected a regular demo file"));
    }
    let file = File::open(path).map_err(|_| Failure::read("could not open demo"))?;
    let mut prefix = Vec::new();
    file.take(MAX_HEADER_BYTES)
        .read_to_end(&mut prefix)
        .map_err(|_| Failure::read("could not read demo header"))?;
    if prefix.len() < HEADER_ENDS_AT_BYTE {
        return Err(Failure::header("truncated demo framing"));
    }
    if std::str::from_utf8(&prefix[..8]).is_err() {
        return Err(Failure::header("unrecognized demo format"));
    }
    let huffman = Vec::new(); // Header framing uses varints and protobuf fields.
    let inputs = ParserInputs {
        real_name_to_og_name: Default::default(),
        wanted_players: Vec::new(),
        wanted_player_props: Vec::new(),
        wanted_other_props: Vec::new(),
        wanted_prop_states: Default::default(),
        wanted_ticks: Vec::new(),
        wanted_events: Vec::new(),
        parse_ents: false,
        parse_projectiles: false,
        parse_grenades: false,
        parse_infernos: false,
        only_header: true,
        only_convars: false,
        huffman_lookup_table: &huffman,
        order_by_steamid: false,
        list_props: false,
        fallback_bytes: None,
    };
    let header = FirstPassParser::new(&inputs)
        .parse_header_only(&prefix)
        .map_err(|error| {
            Failure::header(match error {
                DemoParserError::Source1DemoError => "Source 1 demo",
                DemoParserError::UnknownFile => "unrecognized demo format",
                DemoParserError::OutOfBytesError => "truncated demo header",
                DemoParserError::MalformedMessage => "malformed or truncated demo header",
                _ => "native parser rejected the demo header",
            })
        })?;
    let header: BTreeMap<_, _> = header.into_iter().collect();
    Ok(json!({"header": header}))
}
