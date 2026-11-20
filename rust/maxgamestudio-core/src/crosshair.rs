//! Native VALORANT crosshair codes and their editor JSON representation.

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::fmt;

const FORMAT: &str = "valorant-native-v0";
const COLORS: [&str; 8] = [
    "white",
    "green",
    "yellowgreen",
    "greenyellow",
    "yellow",
    "cyan",
    "pink",
    "red",
];

#[derive(Debug, PartialEq, Eq)]
pub struct CrosshairError(String);

impl fmt::Display for CrosshairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CrosshairError {}

fn error(message: impl Into<String>) -> CrosshairError {
    CrosshairError(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Primary,
    Ads,
    Sniper,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Self::Primary => "P",
            Self::Ads => "A",
            Self::Sniper => "S",
        }
    }

    fn from_marker(marker: &str) -> Option<Self> {
        match marker {
            "P" => Some(Self::Primary),
            "A" => Some(Self::Ads),
            "S" => Some(Self::Sniper),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrosshairField {
    pub key: String,
    pub value: String,
    pub raw_value: String,
    pub known: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrosshairSection {
    pub name: Section,
    pub fields: Vec<CrosshairField>,
}

impl CrosshairSection {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .rev()
            .find(|field| field.key == key)
            .map(|field| field.value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrosshairAst {
    pub start_fields: Vec<CrosshairField>,
    pub sections: Vec<CrosshairSection>,
}

impl CrosshairAst {
    pub fn section(&self, name: Section) -> Option<&CrosshairSection> {
        self.sections.iter().find(|section| section.name == name)
    }
}

pub struct ParseOptions {
    pub strict: bool,
    pub allow_unknown: bool,
    pub require_primary: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            strict: true,
            allow_unknown: true,
            require_primary: true,
        }
    }
}

pub struct SerializeOptions {
    pub preserve_unknown: bool,
    pub canonical: bool,
}

impl Default for SerializeOptions {
    fn default() -> Self {
        Self {
            preserve_unknown: true,
            canonical: false,
        }
    }
}

#[derive(Clone, Copy)]
enum FieldRule {
    Boolean,
    Integer(u32),
    Decimal(u32),
    HexColor,
}

use FieldRule::{Boolean, Decimal, HexColor, Integer};

// Table order is also the native canonical key order.
const START_RULES: &[(&str, FieldRule)] = &[("s", Boolean), ("c", Boolean), ("p", Boolean)];
const PROFILE_RULES: &[(&str, FieldRule)] = &[
    ("c", Integer(8)),
    ("u", HexColor),
    ("b", Boolean),
    ("h", Boolean),
    ("o", Decimal(1)),
    ("t", Integer(10)),
    ("d", Boolean),
    ("a", Decimal(1)),
    ("z", Integer(6)),
    ("f", Boolean),
    ("s", Boolean),
    ("m", Boolean),
    ("0b", Boolean),
    ("0a", Decimal(1)),
    ("0l", Integer(20)),
    ("0v", Integer(20)),
    ("0g", Boolean),
    ("0t", Integer(10)),
    ("0o", Integer(20)),
    ("0m", Boolean),
    ("0s", Decimal(3)),
    ("0f", Boolean),
    ("0e", Decimal(3)),
    ("1b", Boolean),
    ("1a", Decimal(1)),
    ("1l", Integer(20)),
    ("1v", Integer(20)),
    ("1g", Boolean),
    ("1t", Integer(10)),
    ("1o", Integer(40)),
    ("1m", Boolean),
    ("1s", Decimal(3)),
    ("1f", Boolean),
    ("1e", Decimal(3)),
];
const SNIPER_RULES: &[(&str, FieldRule)] = &[
    ("c", Integer(8)),
    ("s", Decimal(4)),
    ("o", Decimal(1)),
    ("d", Boolean),
    ("t", HexColor),
    ("u", HexColor),
];

fn rules(section: Option<Section>) -> &'static [(&'static str, FieldRule)] {
    match section {
        None => START_RULES,
        Some(Section::Primary | Section::Ads) => PROFILE_RULES,
        Some(Section::Sniper) => SNIPER_RULES,
    }
}

fn section_name(section: Option<Section>) -> &'static str {
    section.map_or("start", Section::name)
}

fn trim(value: &str) -> &str {
    value.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch))
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_:-".contains(&byte))
}

fn boolean_text(value: &str, strict: bool) -> Option<bool> {
    if strict {
        return match value {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
    }
    // These are the extra case-folding forms that can produce a recognized alias.
    let folded = value.to_lowercase().replace('ſ', "s").replace('ﬀ', "ff");
    match folded.as_str() {
        "0" | "false" | "no" | "off" => Some(false),
        "1" | "true" | "yes" | "on" => Some(true),
        _ => None,
    }
}

fn ascii_digits(input: &str) -> Cow<'_, str> {
    // Unicode 15.0 decimal-zero blocks accepted by the original CPython 3.12 codec.
    // Other numeric characters such as roman numerals are outside this grammar.
    const ZEROES: &[u32] = &[
        0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66, 0xde6,
        0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
        0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
        0xff10, 0x104a0, 0x10d30, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0,
        0x11650, 0x116c0, 0x11730, 0x118e0, 0x11950, 0x11c50, 0x11d50, 0x11da0, 0x11f50, 0x16a60,
        0x16ac0, 0x16b50, 0x1d7ce, 0x1d7d8, 0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0, 0x1e4f0,
        0x1e950, 0x1fbf0,
    ];
    if input.is_ascii() {
        return Cow::Borrowed(input);
    }
    Cow::Owned(
        input
            .chars()
            .map(|ch| {
                let code = ch as u32;
                let end = ZEROES.partition_point(|zero| *zero <= code);
                end.checked_sub(1)
                    .map(|index| code - ZEROES[index])
                    .filter(|digit| *digit < 10)
                    .map_or(ch, |digit| char::from(b'0' + digit as u8))
            })
            .collect(),
    )
}

fn split_sign(value: &str) -> (bool, &str) {
    if let Some(unsigned) = value.strip_prefix('-') {
        return (true, unsigned);
    }
    (false, value.strip_prefix('+').unwrap_or(value))
}

fn digits_only(value: &str) -> bool {
    value.bytes().all(|byte| byte.is_ascii_digit())
}

fn non_finite(unsigned: &str) -> bool {
    let text = unsigned.to_ascii_lowercase();
    text == "inf"
        || text == "infinity"
        || text
            .strip_prefix("nan")
            .or_else(|| text.strip_prefix("snan"))
            .is_some_and(digits_only)
}

/// A finite decimal with no leading or trailing zero digits and a point
/// relative to the first digit. This keeps native rounding decimal-exact.
struct NativeNumber {
    negative: bool,
    digits: Vec<u8>,
    point: i64,
}

impl NativeNumber {
    fn read(negative: bool, text: &str, allow_exponent: bool) -> Option<Self> {
        let (mantissa, exponent) = if allow_exponent {
            match text.split_once(['e', 'E']) {
                Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
                None => (text, 0),
            }
        } else {
            (text, 0)
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if (!digits_only(whole) || !digits_only(fraction))
            || (whole.is_empty() && fraction.is_empty())
        {
            return None;
        }
        // CPython's Decimal limits the stored exponent after removing the point.
        const MIN_DECIMAL_EXPONENT: i64 = -1_999_999_999_999_999_997;
        const MAX_DECIMAL_EXPONENT: i64 = 999_999_999_999_999_999;
        if !(MIN_DECIMAL_EXPONENT..=MAX_DECIMAL_EXPONENT)
            .contains(&exponent.saturating_sub(fraction.len() as i64))
        {
            return None;
        }
        let mut digits: Vec<u8> = whole
            .bytes()
            .chain(fraction.bytes())
            .map(|byte| byte - b'0')
            .collect();
        let first = digits.iter().position(|digit| *digit != 0);
        if let Some(first) = first {
            let last = digits.iter().rposition(|digit| *digit != 0).unwrap();
            digits.truncate(last + 1);
            digits.drain(..first);
            Some(Self {
                negative,
                digits,
                point: (whole.len() as i64 - first as i64).saturating_add(exponent),
            })
        } else {
            Some(Self {
                negative,
                digits: Vec::new(),
                point: 0,
            })
        }
    }

    fn digit(&self, index: i64) -> u32 {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.digits.get(index))
            .map_or(0, |digit| u32::from(*digit))
    }

    fn has_fraction(&self) -> bool {
        !self.digits.is_empty() && self.point < self.digits.len() as i64
    }

    fn bounded_whole(&self, maximum: u32) -> Result<u32, CrosshairError> {
        if self.digits.is_empty() {
            return Ok(0);
        }
        if self.negative {
            return Err(error("must be at least 0"));
        }
        // Every native upper bound is two digits or less.
        if self.point > 2 {
            return Err(error(format!("must be at most {maximum}")));
        }
        let whole = (0..self.point).fold(0, |total, index| total * 10 + self.digit(index));
        if whole > maximum || (whole == maximum && self.has_fraction()) {
            return Err(error(format!("must be at most {maximum}")));
        }
        Ok(whole)
    }

    fn round_to_thousandths(&self, whole: u32) -> String {
        let point = self.point;
        let fraction = self.digit(point) * 100 + self.digit(point + 1) * 10 + self.digit(point + 2);
        let scaled = whole * 1000 + fraction + u32::from(self.digit(point + 3) >= 5);
        let sign = if self.negative { "-" } else { "" };
        let remainder = scaled % 1000;
        if remainder == 0 {
            return format!("{sign}{}", scaled / 1000);
        }
        let tail = format!("{remainder:03}");
        format!("{sign}{}.{}", scaled / 1000, tail.trim_end_matches('0'))
    }
}

fn native_number(
    value: &str,
    strict: bool,
    integer: bool,
    maximum: u32,
) -> Result<String, CrosshairError> {
    let value = ascii_digits(value);
    let (_, original_digits) = split_sign(&value);
    // A plain integer token enters CPython's int(text) path before model validation.
    if integer && original_digits.len() > 4300 && digits_only(original_digits) {
        return Err(error(format!(
            "Exceeds the limit (4300 digits) for integer string conversion: value has {} digits; use sys.set_int_max_str_digits() to increase the limit",
            original_digits.len()
        )));
    }
    let value = if !strict && value.contains('_') {
        Cow::Owned(value.replace('_', ""))
    } else {
        value
    };
    let (negative, unsigned) = split_sign(&value);
    let invalid = if integer {
        "expected an integer"
    } else {
        "expected a finite decimal number"
    };
    if strict && integer && !digits_only(unsigned) {
        return Err(error(invalid));
    }
    if !strict && non_finite(unsigned) && !integer {
        return Err(error("NaN and infinity are not valid native values"));
    }
    let number = NativeNumber::read(negative, unsigned, !strict).ok_or_else(|| error(invalid))?;
    if integer && number.has_fraction() {
        return Err(error(invalid));
    }
    let whole = number.bounded_whole(maximum)?;
    if integer {
        Ok(whole.to_string())
    } else {
        Ok(number.round_to_thousandths(whole))
    }
}

fn normalize_field(
    section: Option<Section>,
    key: &str,
    raw: &str,
    strict: bool,
) -> Result<(String, bool), CrosshairError> {
    fn normalize_known(
        rule: FieldRule,
        value: &str,
        strict: bool,
    ) -> Result<String, CrosshairError> {
        match rule {
            Boolean => boolean_text(value, strict)
                .map(|value| u8::from(value).to_string())
                .ok_or_else(|| error("expected 0 or 1")),
            Integer(maximum) => native_number(value, strict, true, maximum),
            Decimal(maximum) => native_number(value, strict, false, maximum),
            HexColor => {
                let value = if strict {
                    value
                } else {
                    value.strip_prefix('#').unwrap_or(value)
                };
                if !matches!(value.len(), 6 | 8)
                    || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(error("expected a 6- or 8-digit hexadecimal color"));
                }
                Ok(value.to_ascii_uppercase())
            }
        }
    }

    let value = trim(raw);
    let contextualize = |err: CrosshairError| {
        error(format!(
            "{err} (section {}, field {key})",
            section_name(section)
        ))
    };
    if !valid_key(key) {
        return Err(contextualize(error("invalid field key")));
    }
    if value.is_empty() {
        return Err(contextualize(error("field value must not be blank")));
    }
    let rule = rules(section)
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, rule)| *rule);
    match rule {
        Some(rule) => normalize_known(rule, value, strict)
            .map(|normalized| (normalized, true))
            .map_err(contextualize),
        None => Ok((value.to_owned(), false)),
    }
}

fn python_string_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::new();
    result.push(quote);
    for ch in value.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            _ if ch == quote => {
                result.push('\\');
                result.push(ch);
            }
            _ if ch.is_control() || (ch.is_whitespace() && ch != ' ') => {
                let code = ch as u32;
                if code <= 0xff {
                    result.push_str(&format!("\\x{code:02x}"));
                } else {
                    result.push_str(&format!("\\u{code:04x}"));
                }
            }
            _ => result.push(ch),
        }
    }
    result.push(quote);
    result
}

fn short_pydantic_repr(repr: &str) -> String {
    let length = repr.chars().count();
    if length <= 50 {
        return repr.to_owned();
    }
    let start: String = repr.chars().take(25).collect();
    let end: String = repr.chars().skip(length - 24).collect();
    format!("{start}...{end}")
}

// The Python API returns these model diagnostics as the semantic 422 detail.
struct ModelError {
    path: &'static str,
    message: String,
    code: &'static str,
    input: String,
    input_type: &'static str,
}

fn model_error(name: &str, issues: &[ModelError]) -> CrosshairError {
    let plural = if issues.len() == 1 { "" } else { "s" };
    let mut detail = format!("{} validation error{plural} for {name}", issues.len());
    for issue in issues {
        if !issue.path.is_empty() {
            detail.push('\n');
            detail.push_str(issue.path);
        }
        let input = short_pydantic_repr(&issue.input);
        detail.push_str(&format!(
            "\n  {} [type={}, input_value={input}, input_type={}]\n    For further information visit https://errors.pydantic.dev/2.13/v/{}",
            issue.message, issue.code, issue.input_type, issue.code
        ));
    }
    error(detail)
}

fn field_validation(field: &CrosshairField) -> Result<(), CrosshairError> {
    let mut issues = Vec::new();
    for (path, input, limit) in [
        ("key", &field.key, 64),
        ("value", &field.value, 1024),
        ("raw_value", &field.raw_value, 1024),
    ] {
        let issue = if input.chars().count() > limit {
            Some((
                format!("String should have at most {limit} characters"),
                "string_too_long",
            ))
        } else if input.contains(['\r', '\n', ';']) {
            let name = if path == "raw_value" {
                "raw crosshair"
            } else {
                "crosshair field"
            };
            Some((
                format!("Value error, {name} values cannot contain ';' or line breaks"),
                "value_error",
            ))
        } else {
            None
        };
        if let Some((message, code)) = issue {
            issues.push(ModelError {
                path,
                message,
                code,
                input: python_string_repr(input),
                input_type: "str",
            });
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(model_error("CrosshairField", &issues))
    }
}

fn field_list_repr(fields: &[CrosshairField], section: &str) -> String {
    let values: Vec<String> = fields
        .iter()
        .map(|field| {
            let known = if field.known { "True" } else { "False" };
            format!(
                "CrosshairField(key={}, value={}, raw_value={}, known={known}, section='{section}')",
                python_string_repr(&field.key),
                python_string_repr(&field.value),
                python_string_repr(&field.raw_value),
            )
        })
        .collect();
    format!("[{}]", values.join(", "))
}

fn syntax(message: &str, index: usize, context: Option<(&str, &str)>) -> CrosshairError {
    match context {
        Some((kind, value)) => error(format!("{message} (token {index}, {kind} {value})")),
        None => error(format!("{message} (token {index})")),
    }
}

pub fn parse_crosshair_code(
    code: &str,
    options: ParseOptions,
) -> Result<CrosshairAst, CrosshairError> {
    let text = trim(code).trim_start_matches('\u{feff}');
    if text.is_empty() {
        return Err(error("crosshair code must not be blank"));
    }
    let mut tokens: Vec<_> = text.split(';').map(trim).collect();
    if !options.strict {
        while tokens.last() == Some(&"") {
            tokens.pop();
        }
    }
    if tokens.first() != Some(&"0") {
        return Err(syntax(
            "VALORANT crosshair code must start with version 0",
            0,
            None,
        ));
    }
    let mut ast = CrosshairAst {
        start_fields: Vec::new(),
        sections: Vec::new(),
    };
    let mut index = 1;
    while let Some(&key) = tokens.get(index) {
        if key.is_empty() {
            return Err(syntax("empty token is not valid", index, None));
        }
        if let Some(section) = Section::from_marker(key) {
            if ast.section(section).is_some() {
                return Err(syntax(
                    "profile section is repeated",
                    index,
                    Some(("section", key)),
                ));
            }
            ast.sections.push(CrosshairSection {
                name: section,
                fields: Vec::new(),
            });
            index += 1;
            continue;
        }
        if key == "0" {
            return Err(syntax(
                "version token may only appear at the beginning",
                index,
                None,
            ));
        }
        if !valid_key(key) {
            return Err(syntax("invalid field key", index, Some(("field", key))));
        }
        let &raw_value = tokens
            .get(index + 1)
            .ok_or_else(|| syntax("field key is missing a value", index, Some(("field", key))))?;
        if raw_value.is_empty() {
            return Err(syntax(
                "field value must not be blank",
                index + 1,
                Some(("field", key)),
            ));
        }
        let (section, fields) = match ast.sections.last_mut() {
            Some(section) => (Some(section.name), &mut section.fields),
            None => (None, &mut ast.start_fields),
        };
        let name = section_name(section);
        let token_error = |message: &str| {
            error(format!(
                "{message} (token {index}, section {name}, field {key})"
            ))
        };
        let (value, known) = normalize_field(section, key, raw_value, options.strict)
            .map_err(|err| token_error(&err.0))?;
        if !options.allow_unknown && !known {
            return Err(token_error("unknown field is not allowed"));
        }
        let field = CrosshairField {
            key: key.to_owned(),
            value,
            raw_value: raw_value.to_owned(),
            known,
        };
        field_validation(&field)?;
        if options.strict && known && fields.iter().any(|earlier| earlier.key == key) {
            return Err(token_error("known field is repeated"));
        }
        fields.push(field);
        index += 2;
    }
    let has_primary = ast.section(Section::Primary).is_some();
    if options.require_primary && !has_primary {
        return Err(error("crosshair code must contain a P primary section"));
    }
    if ast.start_fields.len() > 128 {
        return Err(model_error(
            "ValorantCrosshairAST",
            &[ModelError {
                path: "start_fields",
                message: format!(
                    "List should have at most 128 items after validation, not {}",
                    ast.start_fields.len()
                ),
                code: "too_long",
                input: field_list_repr(&ast.start_fields, "start"),
                input_type: "list",
            }],
        ));
    }
    if !has_primary {
        let sections: Vec<_> = ast
            .sections
            .iter()
            .map(|section| {
                let name = section.name.name();
                format!(
                    "CrosshairSection(name='{name}', fields={})",
                    field_list_repr(&section.fields, name)
                )
            })
            .collect();
        return Err(model_error(
            "ValorantCrosshairAST",
            &[ModelError {
                path: "",
                message: "Value error, a VALORANT crosshair AST requires exactly one P section"
                    .to_owned(),
                code: "value_error",
                input: format!(
                    "{{'version': 0, 'start_fields': {}, 'sections': [{}]}}",
                    field_list_repr(&ast.start_fields, "start"),
                    sections.join(", ")
                ),
                input_type: "dict",
            }],
        ));
    }
    Ok(ast)
}

pub fn serialize_crosshair_code(
    ast: &CrosshairAst,
    options: SerializeOptions,
) -> Result<String, CrosshairError> {
    fn append_fields(
        output: &mut String,
        fields: &[CrosshairField],
        section: Option<Section>,
        options: &SerializeOptions,
    ) -> Result<(), CrosshairError> {
        let mut append = |field: &CrosshairField| {
            let value = if field.known {
                let (value, known) = normalize_field(section, &field.key, &field.value, true)?;
                if !known {
                    return Err(error(format!(
                        "known field is not recognized for this section (section {}, field {})",
                        section_name(section),
                        field.key
                    )));
                }
                value
            } else {
                field.raw_value.clone()
            };
            output.push(';');
            output.push_str(&field.key);
            output.push(';');
            output.push_str(&value);
            Ok(())
        };
        if options.canonical {
            for (key, _) in rules(section) {
                if let Some(field) = fields
                    .iter()
                    .rev()
                    .find(|field| field.known && field.key == *key)
                {
                    append(field)?;
                }
            }
            if options.preserve_unknown {
                for field in fields.iter().filter(|field| !field.known) {
                    append(field)?;
                }
            }
        } else {
            for field in fields
                .iter()
                .filter(|field| field.known || options.preserve_unknown)
            {
                append(field)?;
            }
        }
        Ok(())
    }

    let mut output = String::from("0");
    append_fields(&mut output, &ast.start_fields, None, &options)?;
    for section in &ast.sections {
        output.push(';');
        output.push_str(section.name.name());
        append_fields(&mut output, &section.fields, Some(section.name), &options)?;
    }
    Ok(output)
}

#[derive(Debug, Serialize)]
pub struct EncodedCrosshair {
    pub code: String,
    pub format: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DecodedCrosshair {
    pub code: String,
    pub profiles: Value,
    pub format: &'static str,
}

pub fn encode_crosshair(profiles: &Value) -> Result<EncodedCrosshair, CrosshairError> {
    ui_profiles_to_code(profiles)
        .map(|code| EncodedCrosshair {
            code,
            format: FORMAT,
        })
        .map_err(|err| error(format!("invalid crosshair profile: {err}")))
}

pub fn decode_crosshair(code: &str) -> Result<DecodedCrosshair, CrosshairError> {
    (|| {
        let ast = parse_crosshair_code(code, ParseOptions::default())?;
        let code = serialize_crosshair_code(&ast, SerializeOptions::default())?;
        let profiles = profiles_from_ast(&ast);
        Ok(DecodedCrosshair {
            code,
            profiles,
            format: FORMAT,
        })
    })()
    .map_err(|err: CrosshairError| error(format!("invalid VALORANT crosshair code: {err}")))
}

struct EditorToggle {
    key: &'static str,
    native: &'static str,
    default: bool,
    absent_field: bool,
}

const EDITOR_TOGGLES: &[EditorToggle] = &[
    EditorToggle {
        key: "outlines",
        native: "h",
        default: true,
        absent_field: true,
    },
    EditorToggle {
        key: "centerDot",
        native: "d",
        default: false,
        absent_field: false,
    },
    EditorToggle {
        key: "innerLines",
        native: "0b",
        default: true,
        absent_field: true,
    },
    EditorToggle {
        key: "outerLines",
        native: "1b",
        default: false,
        absent_field: true,
    },
    EditorToggle {
        key: "firingError",
        native: "0f",
        default: false,
        absent_field: false,
    },
    EditorToggle {
        key: "movementError",
        native: "0m",
        default: false,
        absent_field: false,
    },
];

struct EditorNumber {
    key: &'static str,
    native: &'static str,
    default: f64,
    absent_field: f64,
    maximum: u32,
    integer: bool,
}

const EDITOR_NUMBERS: &[EditorNumber] = &[
    EditorNumber {
        key: "outlineOpacity",
        native: "o",
        default: 0.8,
        absent_field: 0.5,
        maximum: 1,
        integer: false,
    },
    EditorNumber {
        key: "outlineThickness",
        native: "t",
        default: 1.0,
        absent_field: 1.0,
        maximum: 10,
        integer: true,
    },
    EditorNumber {
        key: "centerDotOpacity",
        native: "a",
        default: 1.0,
        absent_field: 1.0,
        maximum: 1,
        integer: false,
    },
    EditorNumber {
        key: "centerDotThickness",
        native: "z",
        default: 2.0,
        absent_field: 2.0,
        maximum: 6,
        integer: true,
    },
    EditorNumber {
        key: "innerLinesOpacity",
        native: "0a",
        default: 1.0,
        absent_field: 0.8,
        maximum: 1,
        integer: false,
    },
    EditorNumber {
        key: "innerLinesLength",
        native: "0l",
        default: 4.0,
        absent_field: 6.0,
        maximum: 20,
        integer: true,
    },
    EditorNumber {
        key: "innerLinesThickness",
        native: "0t",
        default: 2.0,
        absent_field: 2.0,
        maximum: 10,
        integer: true,
    },
    EditorNumber {
        key: "innerLinesOffset",
        native: "0o",
        default: 2.0,
        absent_field: 3.0,
        maximum: 20,
        integer: true,
    },
    EditorNumber {
        key: "outerLinesOpacity",
        native: "1a",
        default: 0.5,
        absent_field: 0.35,
        maximum: 1,
        integer: false,
    },
    EditorNumber {
        key: "outerLinesLength",
        native: "1l",
        default: 2.0,
        absent_field: 2.0,
        maximum: 20,
        integer: true,
    },
    EditorNumber {
        key: "outerLinesThickness",
        native: "1t",
        default: 2.0,
        absent_field: 2.0,
        maximum: 10,
        integer: true,
    },
    EditorNumber {
        key: "outerLinesOffset",
        native: "1o",
        default: 3.0,
        absent_field: 10.0,
        maximum: 40,
        integer: true,
    },
];

fn editor_boolean(value: &Value) -> Result<bool, CrosshairError> {
    let parsed = match value {
        Value::Bool(value) => Some(*value),
        Value::Number(number) => match number.as_u64() {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        },
        Value::String(value) => boolean_text(trim(value), false),
        _ => None,
    };
    parsed.ok_or_else(|| error("boolean editor fields must be true/false or 0/1"))
}

fn float_string(value: &str) -> Option<f64> {
    let value = ascii_digits(value.trim());
    if !value.contains('_') {
        return value.parse().ok();
    }
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'_'
            && (index == 0
                || index + 1 == bytes.len()
                || !bytes[index - 1].is_ascii_digit()
                || !bytes[index + 1].is_ascii_digit())
        {
            return None;
        }
    }
    value.replace('_', "").parse().ok()
}

fn editor_number(value: &Value) -> Result<f64, CrosshairError> {
    let parsed = match value {
        Value::Number(value) => value.as_f64(),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        Value::String(value) => float_string(value),
        _ => None,
    };
    parsed.ok_or_else(|| error("numeric editor fields must be finite numbers"))
}

fn number_text(value: f64, maximum: u32, integer: bool) -> Result<String, CrosshairError> {
    if !value.is_finite() || !(0.0..=f64::from(maximum)).contains(&value) {
        return Err(error(format!(
            "numeric editor field must be finite and within [0, {maximum}]"
        )));
    }
    if value.fract() == 0.0 {
        return Ok((value as u32).to_string());
    }
    if integer {
        return Err(error("this editor field must be an integer"));
    }
    Ok(format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned())
}

fn known_field(key: &str, value: impl Into<String>) -> CrosshairField {
    let value = value.into();
    CrosshairField {
        key: key.to_owned(),
        raw_value: value.clone(),
        value,
        known: true,
    }
}

fn editor_section(
    section: Section,
    source: Option<&Value>,
) -> Result<CrosshairSection, CrosshairError> {
    let source = match source {
        None | Some(Value::Null) => None,
        Some(Value::Object(profile)) => Some(profile),
        _ => return Err(error("each crosshair profile must be an object")),
    };
    let get = |key: &str| source.and_then(|profile| profile.get(key));
    let color = match get("color") {
        None => Some(1),
        Some(value) => value
            .as_str()
            .and_then(|name| COLORS.iter().position(|color| name == *color)),
    }
    .ok_or_else(|| error("crosshair color is not a supported preset"))?;

    let mut fields = vec![known_field("c", color.to_string())];
    for toggle in EDITOR_TOGGLES {
        let value = get(toggle.key)
            .map(editor_boolean)
            .transpose()?
            .unwrap_or(toggle.default);
        fields.push(known_field(toggle.native, if value { "1" } else { "0" }));
    }
    for number in EDITOR_NUMBERS {
        let value = get(number.key)
            .map(editor_number)
            .transpose()?
            .unwrap_or(number.default);
        fields.push(known_field(
            number.native,
            number_text(value, number.maximum, number.integer)?,
        ));
    }

    let mut native = CrosshairSection {
        name: section,
        fields,
    };
    if section == Section::Sniper {
        let dot = native.get("d").unwrap();
        let opacity = native.get("a").unwrap();
        let thickness = native.get("z").unwrap().parse::<f64>().unwrap();
        native.fields = vec![
            known_field("c", color.to_string()),
            known_field("d", dot),
            known_field("o", opacity),
            known_field("s", number_text(thickness, 4, false)?),
        ];
    } else {
        let movement = native.get("0m").unwrap().to_owned();
        let firing = native.get("0f").unwrap().to_owned();
        native.fields.extend([
            known_field("f", "0"),
            known_field("1m", movement),
            known_field("1f", firing),
        ]);
    }
    Ok(native)
}

pub fn ui_profiles_to_code(profiles: &Value) -> Result<String, CrosshairError> {
    let profiles = profiles
        .as_object()
        .ok_or_else(|| error("crosshair profiles must be an object"))?;
    let mut unknown: Vec<_> = profiles
        .keys()
        .filter(|name| Section::from_marker(name).is_none())
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(error(format!(
            "unsupported crosshair profile(s): {}",
            unknown.join(", ")
        )));
    }
    let sections = [Section::Primary, Section::Ads, Section::Sniper]
        .into_iter()
        .map(|section| editor_section(section, profiles.get(section.name())))
        .collect::<Result<Vec<_>, CrosshairError>>()?;
    serialize_crosshair_code(
        &CrosshairAst {
            start_fields: vec![known_field("s", "1")],
            sections,
        },
        SerializeOptions {
            canonical: true,
            ..SerializeOptions::default()
        },
    )
}

fn default_profile() -> Map<String, Value> {
    let mut profile = Map::new();
    profile.insert("color".to_owned(), json!("green"));
    for toggle in EDITOR_TOGGLES {
        profile.insert(toggle.key.to_owned(), json!(toggle.default));
    }
    for number in EDITOR_NUMBERS {
        let value = if number.default.fract() == 0.0 {
            json!(number.default as u32)
        } else {
            json!(number.default)
        };
        profile.insert(number.key.to_owned(), value);
    }
    profile
}

fn section_profile(section: Option<&CrosshairSection>) -> Value {
    let mut profile = default_profile();
    let Some(section) = section else {
        return Value::Object(profile);
    };
    let color = section
        .get("c")
        .map_or(1, |value| value.parse::<usize>().unwrap());
    profile.insert(
        "color".to_owned(),
        json!(COLORS.get(color).unwrap_or(&"white")),
    );
    let mut set = |key: &str, value: Value| {
        profile.insert(key.to_owned(), value);
    };
    let field_number = |key: &str, default: f64| {
        section
            .get(key)
            .map_or(default, |value| value.parse::<f64>().unwrap())
    };
    if section.name == Section::Sniper {
        set("centerDot", json!(section.get("d").unwrap_or("1") == "1"));
        set("centerDotOpacity", json!(field_number("o", 1.0)));
        set("centerDotThickness", json!(field_number("s", 2.0)));
        set("innerLines", json!(false));
        set("outerLines", json!(false));
    } else {
        for toggle in EDITOR_TOGGLES {
            let value = section.get(toggle.native).or_else(|| match toggle.native {
                "0m" => section.get("1m"),
                "0f" => section.get("1f"),
                _ => None,
            });
            set(
                toggle.key,
                json!(value.map_or(toggle.absent_field, |value| value == "1")),
            );
        }
        for number in EDITOR_NUMBERS {
            set(
                number.key,
                json!(field_number(number.native, number.absent_field)),
            );
        }
    }
    Value::Object(profile)
}

fn profiles_from_ast(ast: &CrosshairAst) -> Value {
    json!({
        "P": section_profile(ast.section(Section::Primary)),
        "A": section_profile(ast.section(Section::Ads)),
        "S": section_profile(ast.section(Section::Sniper)),
    })
}
