use serde::Serialize;
use serde_json::Value;
use std::{error::Error, fmt};

const REPOSITORY: &str = "INEEDBUG/MaxGameStudio";
const PRODUCT: &str = "maxgamestudio-league";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeagueUpdate {
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateError {
    Version,
    Manifest,
    Channel,
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Version => "Invalid League version",
            Self::Manifest => "Invalid League update manifest",
            Self::Channel => "Invalid League update channel",
        })
    }
}

impl Error for UpdateError {}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    numbers: [u64; 3],
    beta: Option<u64>,
}

impl Version {
    fn channel(&self) -> &'static str {
        if self.beta.is_some() {
            "beta"
        } else {
            "stable"
        }
    }
}

fn parse_component(value: &str) -> Option<u64> {
    if value.is_empty()
        || (value.starts_with('0') && value.len() > 1)
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value
        .parse()
        .ok()
        .filter(|&number| number <= MAX_SAFE_INTEGER)
}

fn parse_version(value: &str) -> Option<Version> {
    let (core, beta) = match value.split_once("-beta.") {
        Some((core, beta)) => (core, Some(parse_component(beta)?)),
        None => (value, None),
    };
    let mut parts = core.split('.');
    let numbers = [
        parse_component(parts.next()?)?,
        parse_component(parts.next()?)?,
        parse_component(parts.next()?)?,
    ];
    if parts.next().is_some() {
        return None;
    }
    Some(Version { numbers, beta })
}

pub fn league_update_feed(current: &str) -> Result<&'static str, UpdateError> {
    let installed = parse_version(current).ok_or(UpdateError::Version)?;
    Ok(if installed.beta.is_some() {
        "https://raw.githubusercontent.com/INEEDBUG/MaxGameStudio/updater/league-beta.json"
    } else {
        "https://raw.githubusercontent.com/INEEDBUG/MaxGameStudio/updater/league-latest.json"
    })
}

pub fn league_update(info: &Value, current: &str) -> Result<Option<LeagueUpdate>, UpdateError> {
    // The original checks the installed version before inspecting an object or array's fields.
    if !info.is_object() && !info.is_array() {
        return Err(UpdateError::Manifest);
    }
    let installed = parse_version(current).ok_or(UpdateError::Version)?;
    let channel = info["channel"].as_str().ok_or(UpdateError::Manifest)?;
    let version = info["version"].as_str().ok_or(UpdateError::Manifest)?;
    let url = info["url"].as_str().ok_or(UpdateError::Manifest)?;
    let tag = format!("league-v{version}");
    if info["product"].as_str() != Some(PRODUCT)
        || !matches!(channel, "stable" | "beta")
        || info["tag"].as_str() != Some(tag.as_str())
        || url
            != format!(
                "https://github.com/{REPOSITORY}/releases/download/{tag}/MaxGameStudioLeague_{version}_x64-setup.exe"
            )
    {
        return Err(UpdateError::Manifest);
    }
    let remote = parse_version(version).ok_or(UpdateError::Version)?;
    if channel != remote.channel() {
        return Err(UpdateError::Channel);
    }
    if channel != installed.channel() {
        return if installed.beta.is_none() {
            Ok(None)
        } else {
            Err(UpdateError::Channel)
        };
    }
    Ok((remote > installed).then(|| LeagueUpdate {
        version: version.to_owned(),
        url: url.to_owned(),
    }))
}
