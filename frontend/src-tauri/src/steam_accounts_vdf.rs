use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

const BAD_VDF: &str = "steam_invalid_accounts";
pub(crate) const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

pub(super) fn account_id(steam_id: &str) -> Option<u32> {
    if steam_id.len() != 17 || !steam_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let account_id = steam_id.parse::<u64>().ok()?.checked_sub(STEAM_ID64_BASE)?;
    u32::try_from(account_id).ok().filter(|id| *id != 0)
}

struct Scalar {
    text: String,
    span: Range<usize>,
}

enum Value {
    Text(Scalar),
    Object { fields: Vec<Field>, end: usize },
}

struct Field {
    name: String,
    value: Value,
}

struct Parser<'a> {
    source: &'a str,
    cursor: usize,
}

impl Parser<'_> {
    fn whitespace(&mut self) {
        while let Some(byte) = self.source.as_bytes().get(self.cursor) {
            if byte.is_ascii_whitespace() {
                self.cursor += 1;
            } else if self.source[self.cursor..].starts_with("//") {
                self.cursor = self.source[self.cursor..]
                    .find('\n')
                    .map_or(self.source.len(), |length| self.cursor + length);
            } else {
                break;
            }
        }
    }

    fn scalar(&mut self) -> Result<Scalar, &'static str> {
        let bytes = self.source.as_bytes();
        let quoted = bytes.get(self.cursor) == Some(&b'"');
        if quoted {
            self.cursor += 1;
        }
        let start = self.cursor;
        while let Some(&byte) = bytes.get(self.cursor) {
            if quoted {
                if byte == b'"' {
                    let span = start..self.cursor;
                    self.cursor += 1;
                    let mut chars = self.source[span.clone()].chars().peekable();
                    let mut text = String::new();
                    while let Some(ch) = chars.next() {
                        if ch == '\\' && matches!(chars.peek(), Some('\\' | '"')) {
                            text.push(chars.next().unwrap());
                        } else {
                            text.push(ch);
                        }
                    }
                    return Ok(Scalar { text, span });
                }
                if byte == b'\\' && matches!(bytes.get(self.cursor + 1), Some(b'\\' | b'"')) {
                    self.cursor += 1;
                }
            } else if byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}' | b'"') {
                break;
            }
            self.cursor += 1;
        }
        if quoted || self.cursor == start {
            return Err(BAD_VDF);
        }
        let span = start..self.cursor;
        Ok(Scalar {
            text: self.source[span.clone()].to_owned(),
            span,
        })
    }

    fn fields(&mut self, depth: u8) -> Result<Vec<Field>, &'static str> {
        if depth > 64 {
            return Err(BAD_VDF);
        }
        let mut fields = Vec::new();
        loop {
            self.whitespace();
            match self.source.as_bytes().get(self.cursor) {
                None if depth == 0 => return Ok(fields),
                Some(b'}') if depth > 0 => return Ok(fields),
                _ => {}
            }
            let name = self.scalar()?.text;
            self.whitespace();
            let value = if self.source.as_bytes().get(self.cursor) == Some(&b'{') {
                self.cursor += 1;
                let nested = self.fields(depth + 1)?;
                let end = self.cursor;
                self.cursor += 1;
                Value::Object {
                    fields: nested,
                    end,
                }
            } else {
                Value::Text(self.scalar()?)
            };
            fields.push(Field { name, value });
        }
    }
}

fn field<'a>(fields: &'a [Field], name: &str) -> Result<Option<&'a Value>, &'static str> {
    let mut matches = fields
        .iter()
        .filter(|field| field.name.eq_ignore_ascii_case(name));
    let result = matches.next().map(|field| &field.value);
    if matches.next().is_some() {
        return Err(BAD_VDF);
    }
    Ok(result)
}

fn scalar_field(fields: &[Field], name: &str) -> Result<Option<Scalar>, &'static str> {
    match field(fields, name)? {
        Some(Value::Text(value)) => Ok(Some(Scalar {
            text: value.text.clone(),
            span: value.span.clone(),
        })),
        Some(Value::Object { .. }) => Err(BAD_VDF),
        None => Ok(None),
    }
}

fn parse_fields(source: &str) -> Result<Vec<Field>, &'static str> {
    let cursor = if source.starts_with('\u{feff}') { 3 } else { 0 };
    Parser { source, cursor }.fields(0)
}

/// Read-only account metadata allows older rows without AccountName.
pub(crate) struct LocalLoginUser {
    pub(crate) account_name: String,
    pub(crate) persona_name: String,
    pub(crate) most_recent: bool,
    pub(crate) remember_password: bool,
    pub(crate) login_timestamp: i64,
}

pub(crate) fn local_login_users(source: &str) -> HashMap<u32, LocalLoginUser> {
    let Ok(root) = parse_fields(source) else {
        return HashMap::new();
    };
    let Ok(Some(Value::Object { fields: users, .. })) = field(&root, "users") else {
        return HashMap::new();
    };
    users
        .iter()
        .filter_map(|entry| {
            let id = account_id(&entry.name)?;
            let Value::Object { fields, .. } = &entry.value else {
                return None;
            };
            let get = |name: &str| {
                fields
                    .iter()
                    .rev()
                    .find_map(|field| match &field.value {
                        Value::Text(value) if field.name == name => Some(value.text.as_str()),
                        _ => None,
                    })
                    .unwrap_or_default()
            };
            Some((
                id,
                LocalLoginUser {
                    account_name: get("AccountName").to_owned(),
                    persona_name: get("PersonaName").to_owned(),
                    most_recent: get("MostRecent") == "1",
                    remember_password: get("RememberPassword") == "1",
                    login_timestamp: get("Timestamp").trim().parse().unwrap_or_default(),
                },
            ))
        })
        .collect()
}

pub(crate) fn flat_settings(source: &str) -> HashMap<String, String> {
    fn quoted_pair(source: &str) -> Option<(String, String, &str)> {
        if !source.starts_with('"') {
            return None;
        }
        let mut parser = Parser { source, cursor: 0 };
        let key = parser.scalar().ok()?.text;
        let after_key = &source[parser.cursor..];
        let value = after_key.trim_start();
        if key.is_empty() || after_key.len() == value.len() || !value.starts_with('"') {
            return None;
        }
        parser.cursor = source.len() - value.len();
        let value = parser.scalar().ok()?.text;
        Some((key, value, &source[parser.cursor..]))
    }

    // CS2 config import can still use quoted pairs when the rest of a file is incomplete.
    let mut output = HashMap::new();
    let mut remaining = source.strip_prefix('\u{feff}').unwrap_or(source);
    loop {
        let row = remaining.trim_start();
        let rest = if let Some((key, value, rest)) = quoted_pair(row) {
            output.insert(key, value);
            rest
        } else {
            row
        };
        let Some((_, next)) = rest.split_once(['\r', '\n']) else {
            break;
        };
        remaining = next;
    }
    output
}

pub(super) struct Account {
    pub(super) steam_id: String,
    pub(super) account_name: String,
    pub(super) persona_name: String,
    pub(super) remembered: bool,
    pub(super) preferred: bool,
    pub(super) timestamp: u64,
    auto_login: Option<Scalar>,
    most_recent: Option<Scalar>,
    end: usize,
}

#[derive(Default)]
pub(super) struct LoginUsers {
    pub(super) accounts: Vec<Account>,
}

impl LoginUsers {
    pub(super) fn parse(source: &str) -> Result<Self, &'static str> {
        let fields = parse_fields(source)?;
        let Some(Value::Object { fields: users, .. }) = field(&fields, "users")? else {
            return Err(BAD_VDF);
        };
        let mut accounts = Vec::new();
        let mut seen = HashSet::new();
        for user in users {
            if account_id(&user.name).is_none() {
                continue;
            }
            if !seen.insert(&user.name) {
                return Err(BAD_VDF);
            }
            let Value::Object { fields, end } = &user.value else {
                return Err(BAD_VDF);
            };
            let Some(name) = scalar_field(fields, "AccountName")? else {
                continue;
            };
            let account_name = name.text;
            if account_name.trim().is_empty() || account_name.contains('\0') {
                continue;
            }
            let remembered =
                scalar_field(fields, "RememberPassword")?.is_some_and(|value| value.text == "1");
            let auto_login = scalar_field(fields, "AutoLogin")?;
            let most_recent = scalar_field(fields, "MostRecent")?;
            let preferred = most_recent
                .as_ref()
                .or(auto_login.as_ref())
                .is_some_and(|value| value.text == "1");
            accounts.push(Account {
                steam_id: user.name.clone(),
                account_name,
                persona_name: scalar_field(fields, "PersonaName")?
                    .map_or_else(String::new, |value| value.text),
                remembered,
                preferred,
                timestamp: scalar_field(fields, "Timestamp")?
                    .and_then(|value| value.text.parse().ok())
                    .unwrap_or(0),
                auto_login,
                most_recent,
                end: *end,
            });
        }
        Ok(Self { accounts })
    }

    pub(super) fn remembered_account(&self, target: &str) -> Result<&Account, &'static str> {
        let account = self
            .accounts
            .iter()
            .find(|user| user.steam_id == target)
            .ok_or("steam_unknown_account")?;
        if !account.remembered {
            return Err("steam_not_remembered");
        }
        Ok(account)
    }

    pub(super) fn select(&self, source: &str, target: &str) -> Result<String, &'static str> {
        let selected = self.remembered_account(target)?;
        let new_schema = self.accounts.iter().any(|user| user.auto_login.is_some());
        let legacy_schema = self.accounts.iter().any(|user| user.most_recent.is_some());
        let mut edits = Vec::new();
        for account in &self.accounts {
            let selected = account.steam_id == target;
            let value = if selected { "1" } else { "0" };
            for existing in [account.auto_login.as_ref(), account.most_recent.as_ref()]
                .into_iter()
                .flatten()
            {
                if existing.text != value {
                    edits.push((existing.span.clone(), value.to_owned()));
                }
            }
        }
        // Keep the file's field family. AllowAutoLogin is the legacy permission,
        // so changing the preferred account must leave that field intact.
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let mut insert = String::new();
        if new_schema && selected.auto_login.is_none() {
            insert.push_str(&format!("{newline}\t\t\"AutoLogin\"\t\t\"1\"{newline}\t"));
        }
        if legacy_schema && selected.most_recent.is_none() {
            insert.push_str(&format!("{newline}\t\t\"MostRecent\"\t\t\"1\"{newline}\t"));
        }
        if !insert.is_empty() {
            edits.push((selected.end..selected.end, insert));
        }
        let mut output = source.to_owned();
        edits.sort_unstable_by_key(|(span, _)| std::cmp::Reverse(span.start));
        for (span, replacement) in edits {
            output.replace_range(span, &replacement);
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "76561197960265730";
    const SECOND: &str = "76561197960265731";

    #[test]
    fn updates_current_schema_and_preserves_payload_bytes() {
        let source = concat!(
            "\u{feff}\"users\"\r\n{\r\n",
            "\t\"76561197960265730\" { \"AccountName\" \"test1\" \"PersonaName\" \"{水} \\\"名\\\"\" ",
            "\"AutoLogin\" \"1\" \"RememberPassword\" \"1\" \"Other\" { \"AutoLogin\" \"7\" } }\r\n",
            "\t// preserve me\r\n",
            "\t\"76561197960265731\" { \"AccountName\" \"test2\" \"AutoLogin\" \"0\" \"RememberPassword\" \"1\" }\r\n}",
        );
        let parsed = LoginUsers::parse(source).unwrap();
        assert_eq!(parsed.accounts[0].persona_name, "{水} \"名\"");
        assert!(parsed.accounts[0].preferred);
        assert!(!parsed.accounts[1].preferred);
        let after = parsed.select(source, SECOND).unwrap();
        let expected = source
            .replace("\"AutoLogin\" \"1\"", "\"AutoLogin\" \"0\"")
            .replace(
                "\"test2\" \"AutoLogin\" \"0\"",
                "\"test2\" \"AutoLogin\" \"1\"",
            );
        assert_eq!(after, expected);
    }

    #[test]
    fn handles_legacy_recent_and_leaves_permission_and_preference_fields_independent() {
        let source = concat!(
            "users { 76561197960265730 { AccountName test1 mostrecent 1 AllowAutoLogin 1 RememberPassword 0 } ",
            "76561197960265731 { AccountName test2 MostRecent 0 AllowAutoLogin 1 RememberPassword 1 } }",
        );
        let parsed = LoginUsers::parse(source).unwrap();
        assert_eq!(
            parsed.select(source, FIRST).unwrap_err(),
            "steam_not_remembered"
        );
        assert_eq!(
            parsed.select(source, SECOND).unwrap(),
            source
                .replace("mostrecent 1", "mostrecent 0")
                .replace("MostRecent 0", "MostRecent 1")
        );
    }

    #[test]
    fn malformed_and_ambiguous_files_are_rejected() {
        for input in [
            "users { 76561197960265730 { AccountName test1 }",
            "users { 76561197960265730 { AccountName test1 accountname test2 } }",
            "users { 76561197960265730 { AccountName test1 } 76561197960265730 { AccountName test2 } }",
            "users { 76561197960265730 { AccountName \"no closing quote } }",
        ] {
            assert_eq!(LoginUsers::parse(input).err(), Some(BAD_VDF));
        }
    }

    #[test]
    fn reads_cfg_pairs_around_an_incomplete_object() {
        let source = "\"sensitivity\" \"0.2\"\n}\n\"sensitivity\" \"0.35\"\n\"m_yaw\"\n\"0.022\"\n";
        let values = flat_settings(source);
        assert_eq!(values.get("sensitivity").map(String::as_str), Some("0.35"));
        assert_eq!(values.get("m_yaw").map(String::as_str), Some("0.022"));
    }
}
