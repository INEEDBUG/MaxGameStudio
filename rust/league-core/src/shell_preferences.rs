//! Read and update the desktop UI settings stored by earlier versions.

use crate::profile_database::{LeagueProfileDatabase, ProfileDatabaseError};
use serde_json::{json, Value};
use std::io;

pub const APP_COMMON: &str = "app-common-main";
pub const MAIN_WINDOW: &str = "window-manager-main/main-window";
pub const LEAGUE_CLIENT: &str = "league-client-main";
const APP_COMMON_RENDERER: &str = "app-common-renderer";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShellPreference {
    Locale,
    Theme,
    ShowDeclaration,
    CloseAction,
    ClientAutoConnect,
    SubscriptionInfo,
}

impl ShellPreference {
    pub const APP_SETTINGS: [Self; 3] = [Self::Locale, Self::Theme, Self::ShowDeclaration];

    pub fn from_ipc(namespace: &str, key: &str) -> Option<Self> {
        [
            Self::Locale,
            Self::Theme,
            Self::ShowDeclaration,
            Self::CloseAction,
            Self::ClientAutoConnect,
            Self::SubscriptionInfo,
        ]
        .into_iter()
        .find(|setting| setting.namespace() == namespace && setting.key() == key)
    }

    pub fn namespace(self) -> &'static str {
        match self {
            Self::Locale | Self::Theme | Self::ShowDeclaration => APP_COMMON,
            Self::CloseAction => MAIN_WINDOW,
            Self::ClientAutoConnect => LEAGUE_CLIENT,
            Self::SubscriptionInfo => APP_COMMON_RENDERER,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Locale => "locale",
            Self::Theme => "theme",
            Self::ShowDeclaration => "showFreeSoftwareDeclaration",
            Self::CloseAction => "closeAction",
            Self::ClientAutoConnect => "autoConnect",
            Self::SubscriptionInfo => "tempAkariSubscriptionInfo",
        }
    }

    pub fn read(
        self,
        database: &LeagueProfileDatabase,
        system_locale: &str,
    ) -> Result<Value, ProfileDatabaseError> {
        let storage_key = format!("{}/{}", self.namespace(), self.key());
        if let Some(saved) = database
            .setting(&storage_key)?
            .filter(|value| self.valid(value))
        {
            return Ok(saved);
        }
        Ok(match self {
            Self::Locale => json!(system_locale),
            Self::Theme => json!("dark"),
            Self::ShowDeclaration | Self::ClientAutoConnect => json!(true),
            Self::CloseAction => json!("ask"),
            Self::SubscriptionInfo => json!({"current": "basic", "shown": false}),
        })
    }

    pub fn write(
        self,
        database: &LeagueProfileDatabase,
        value: &Value,
    ) -> Result<(), ProfileDatabaseError> {
        if !self.valid(value) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "设置值无效。").into());
        }
        database.set_setting(&format!("{}/{}", self.namespace(), self.key()), value)
    }

    fn valid(self, value: &Value) -> bool {
        match self {
            Self::Locale => value.is_string(),
            Self::Theme => matches!(
                value.as_str(),
                Some(
                    "default"
                        | "light"
                        | "sakura"
                        | "butter"
                        | "mint"
                        | "dark"
                        | "graphite"
                        | "cyber"
                        | "aurora"
                )
            ),
            Self::ShowDeclaration | Self::ClientAutoConnect => value.is_boolean(),
            Self::CloseAction => {
                matches!(value.as_str(), Some("ask" | "minimize-to-tray" | "quit"))
            }
            Self::SubscriptionInfo => {
                matches!(value["current"].as_str(), Some("basic" | "pro" | "max"))
                    && value["shown"].is_boolean()
            }
        }
    }
}
