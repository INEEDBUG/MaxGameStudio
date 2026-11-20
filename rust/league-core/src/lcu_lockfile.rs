//! Parse the temporary lockfile emitted by the locally running League client.

/// Contains a session secret, so intentionally has no Debug or serde implementation.
pub struct LcuLockfile<'a> {
    pub pid: u32,
    pub port: u16,
    pub token: &'a str,
}

impl<'a> LcuLockfile<'a> {
    pub fn parse(contents: &'a str) -> Option<Self> {
        let mut parts = contents.trim_end_matches(['\r', '\n']).split(':');
        if parts.next()? != "LeagueClient" {
            return None;
        }

        let pid = parts.next()?.parse().ok()?;
        let port = parts.next()?.parse().ok()?;
        let token = parts.next()?;
        if pid == 0
            || port == 0
            || token.is_empty()
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || parts.next()? != "https"
            || parts.next().is_some()
        {
            return None;
        }

        Some(Self { pid, port, token })
    }
}
