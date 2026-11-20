//! Initial state for the bundled League renderers and their existing SQLite preferences.

use maxgamestudio_league_core::{
    profile_database::{LeagueProfileDatabase, ProfileDatabaseError},
    resg::RESG_NAMESPACE,
    shell_preferences::{LEAGUE_CLIENT as CLIENT, MAIN_WINDOW},
};
use serde_json::{json, Map, Value};
use std::io;

pub(super) const WINDOW_MANAGER: &str = "window-manager-main";
pub(super) const AUX_WINDOW: &str = "window-manager-main/aux-window";
pub(super) const OPGG_WINDOW: &str = "window-manager-main/opgg-window";
pub(super) const AUTO_SELECT: &str = "auto-select-main";

pub(super) fn initial_state(
    db: &LeagueProfileDatabase,
    namespace: &str,
    name: &str,
) -> Result<Option<Value>, ProfileDatabaseError> {
    if name == "settings" {
        if namespace == AUTO_SELECT {
            return Ok(Some(json!({
                "pickConfig": aram_config(db, "pickConfig")?,
                "banConfig": aram_config(db, "banConfig")?,
            })));
        }
        return settings(db, namespace);
    }

    // Dynamic LCU and window fields are handled by their owners before reaching this table.
    let state = match (namespace, name) {
        ("akari-api-main", "state") => {
            json!({"featureGates":null,"notice":null,"contactChannels":null})
        }
        ("auto-gameflow-main", "state") => json!({
            "willAcceptAt":-1,"willSearchMatch":false,"willSearchMatchAt":-1,
            "activityStartStatus":"unavailable","friendsToBeInvited":[],
        }),
        (AUTO_SELECT, "state") => json!({
            "groups":[{
                "groupId":"aram","name":{"zh-CN":"大乱斗类","en":"ARAM-like"},
                "iconPath":"/lol-game-data/assets/content/src/LeagueClient/GameModeAssets/ARAM/img/game-select-icon-hover.png",
                "isCustom":false,"supportedSgpServers":["*"],"positions":["default"],
                "targetGameModes":[{"gameMode":"ARAM","queueTypes":["*"]},{"gameMode":"KIWI","queueTypes":["*"]}],
                "additionalPicks":[],"additionalBans":[],"excludedPicks":[-1],"excludedBans":[],
            }],
            "temporarilyDisabled":false,"delayedBan":null,"delayedPick":null,
            "delayedBenchSwap":null,"delayedChampionSwap":null,"expectedPicks":null,
            "expectedBans":null,"expectedSwaps":null,"activeGroupConfigId":null,
        }),
        ("client-installation-main", "state") => json!({
            "leagueClientExecutablePaths":[],"tencentInstallationPath":null,"tclsExecutablePath":null,
            "weGameLauncherExecutablePath":null,"weGameExecutablePath":null,"officialRiotClientExecutablePath":null,
        }),
        ("extra-assets-main", "gtimg") => json!({"heroList":null,"kiwiAugments":null}),
        ("extra-assets-main", "opgg") => json!({"aramBalance":null}),
        (CLIENT, "initialization") => json!({"progress":null}),
        (CLIENT, "gameData") => json!({
            "champions":{},"augments":{},"perks":{},"perkstyles":{"schemaVersion":0,"styles":{}},
            "queues":{},"items":{},"summonerSpells":{},"gameModeMutators":{},"maps":{},
        }),
        (CLIENT, "honor") => json!({"ballot":null}),
        // The pinned Vue store already has native Sets for the three champion-ID lists.
        (CLIENT, "champSelect") => json!({
            "session":null,"currentChampion":null,"ongoingChampionSwap":null,
            "gridChampions":{},"skinSelectorInfo":null,
        }),
        (CLIENT, "chat") => json!({
            "me":null,"participants.postGame":null,"participants.customGame":null,
            "participants.championSelect":null,"conversations.postGame":null,
            "conversations.customGame":null,"conversations.championSelect":null,
        }),
        (CLIENT, "matchmaking") => json!({"readyCheck":null,"search":null}),
        (CLIENT, "lobby") => json!({"lobby":null,"receivedInvitations":[]}),
        (CLIENT, "login") => json!({"loginQueueState":null}),
        (CLIENT, "lobbyTeamBuilder") => json!({"champSelect.subsetChampionList":[]}),
        ("logger-factory-main", "state") => json!({"logLevel":"info"}),
        ("ongoing-game-main", "state") => json!({
            "championSelections":{},"positionAssignments":{},"analysis":null,
            "queryStage":{"phase":"unavailable","gameInfo":null},"teams":{},"isInEog":false,
            "matchHistoryLoadingState":{},"summonerLoadingState":{},"savedInfoLoadingState":{},
            "rankedStatsLoadingState":{},"championMasteryLoadingState":{},"teamParticipantGroups":{},
            "draft":null,"mergedPremadeTeamMap":{},"inferredPremadeTeams":[],"matchHistoryTagParams":{},
        }),
        ("renderer-debug-main", "state") => {
            json!({"sendAllNativeLcuEvents":false,"logAllLcuEvents":false})
        }
        ("self-update-main", "state") => json!({
            "isUpdateSupportedOnCurrentPlatform":false,"isCheckingUpdates":false,"releaseInfo":null,
            "updateProgressInfo":null,"lastUpdateSucceeded":null,
        }),
        ("sgp-main", "state") => json!({
            "availability":{"region":"","rsoPlatform":"","sgpServerId":"",
                "serversSupported":{"matchHistory":false,"common":false}},
            "isTokenReady":false,"supportedQueues":[],"connectionSuccessesCounted":0,"connectionFailuresCounted":0,
        }),
        (WINDOW_MANAGER, "state") => json!({"supportsMica":false,"downloadTasks":[]}),
        _ => return Ok(None),
    };
    Ok(Some(state))
}

pub(super) fn ongoing_game_data() -> Value {
    json!({
        "matchHistory":{},"summoner":{},"rankedStats":{},"savedInfo":{},
        "championMastery":{},"gameDetails":{},"additionalGames":{},
    })
}

fn setting_defaults(namespace: &str) -> Option<Value> {
    Some(match namespace {
        "auto-gameflow-main" => json!({
            "autoAcceptEnabled":false,"autoAcceptDelaySeconds":0,"autoHonorEnabled":false,
            "autoHonorStrategy":"prefer-lobby-member","playAgainEnabled":false,"autoReconnectEnabled":false,
            "autoMatchmakingEnabled":false,"autoMatchmakingMaximumMatchDuration":0,
            "autoMatchmakingRematchStrategy":"never","autoMatchmakingRematchFixedDuration":2,
            "autoMatchmakingDelaySeconds":5,"autoMatchmakingMinimumMembers":1,"autoMatchmakingWaitForInvitees":true,
            "autoSkipLeaderEnabled":false,"autoHandleInvitationsEnabled":false,"rejectInvitationWhenAway":false,
            "invitationHandlingStrategies":{},"autoSendARAMTeamSideEnabled":false,"autoSendARAMTeamSideVisibleToTeam":false,
        }),
        "auto-misc-main" => json!({
            "autoReplyEnabled":false,"autoReplyEnableOnAway":false,"autoReplyText":"","lockOfflineStatus":false,
            "autoSetStatusMessageEnabled":false,"statusMessage":"","autoSetRankedStatusEnabled":false,
            "rankedStatus":{"queue":"RANKED_SOLO_5x5","tier":"CHALLENGER","division":"I"},
        }),
        "game-client-main" => {
            json!({"terminateGameClientWithShortcut":false,"terminateShortcut":null})
        }
        "league-client-ux-main" => return None,
        "ongoing-game-main" => json!({
            "enabled":true,"matchHistoryLoadCount":50,"gameDetailsLoadCount":20,"concurrency":4,
            "matchHistoryTagPreference":"current","orderPlayerBy":"default","showChampionUsage":"recent",
            "showMatchHistoryItemBorder":false,"showJunglePathing":true,"showJunglePathingForAllPlayers":false,
            "autoRouteWhenGameStarts":false,"queryInLobbyPhase":true,"premadeTeamInferMatchCountThreshold":5,
            "playerCardTags":{
                "showPremadeTeamTag":true,"showSuspiciousFlashPositionTag":true,"showWinningStreakTag":true,
                "showLosingStreakTag":true,"showSoloKillsTag":true,"showEasyGankTag":true,"showGreatPerformanceTag":true,
                "showAverageTeamDamageTag":false,"showAverageTeamDamageTakenTag":false,"showAverageTeamGoldTag":false,
                "showAverageCsPerMinuteTag":false,"showAverageDamageGoldEfficiencyTag":false,
                "showAverageEnemyMissingPingsTag":false,"showAverageVisionScoreTag":false,
                "showAverageKillDamageEfficiencyTag":true,"showSelfTag":true,"showMetTag":true,"showTaggedTag":true,
                "showWinRateTeamTag":true,"showPrivacyTag":true,"showAkariScoreTag":false,
            },
        }),
        "self-update-main" => {
            json!({"autoCheckUpdates":true,"autoDownloadUpdates":true,"ignoreVersion":null})
        }
        MAIN_WINDOW => json!({"pinned":false,"opacity":1}),
        AUX_WINDOW => {
            json!({"enabled":true,"autoShow":true,"pinned":true,"opacity":1,"showSkinSelector":false})
        }
        OPGG_WINDOW => {
            json!({"enabled":true,"autoShow":true,"pinned":true,"opacity":1,"showShortcut":null})
        }
        RESG_NAMESPACE => {
            json!({"enabled":false,"autoShow":true,"alwaysOnTop":false,"displayMode":"all"})
        }
        _ => return None,
    })
}

fn settings(
    db: &LeagueProfileDatabase,
    namespace: &str,
) -> Result<Option<Value>, ProfileDatabaseError> {
    match namespace {
        WINDOW_MANAGER => {
            return Ok(Some(
                json!({"backgroundMaterial":"none","contentProtection":false}),
            ))
        }
        "league-client-ux-main" => return Ok(Some(json!({"useWmi":false}))),
        _ => {}
    }
    let Some(Value::Object(defaults)) = setting_defaults(namespace) else {
        return Ok(None);
    };
    let mut values = Map::new();
    for (key, default) in defaults {
        let preference = Preference {
            namespace,
            key: &key,
            default,
        };
        values.insert(key.clone(), preference.read(db)?);
    }
    Ok(Some(Value::Object(values)))
}

pub(super) fn backend_pref<'a>(namespace: &'a str, key: &'a str) -> Option<Preference<'a>> {
    let default = setting_defaults(namespace)?.as_object()?.get(key)?.clone();
    Some(Preference {
        namespace,
        key,
        default,
    })
}

pub(super) fn renderer_pref<'a>(
    label: &str,
    namespace: &'a str,
    key: &'a str,
) -> Option<Preference<'a>> {
    // The first value supplies the expected JSON type. Null lets the pinned Vue store
    // supply its own original default whenever this key has never been saved.
    let default = match (label, namespace, key) {
        (
            "main",
            "main-window-ui-renderer",
            "useProfileSkinAsBackground" | "sidebarCollapsed" | "showTestPage",
        ) => json!(false),
        (
            "main",
            "player-tabs-renderer",
            "matchHistoryUseSgpApi"
            | "refreshTabsAfterGameEnds"
            | "defaultShowPractice"
            | "defaultShowIrregularGames",
        ) => json!(false),
        ("main", "player-tabs-renderer", "loadCount") => json!(20),
        (
            "main",
            "player-tabs-renderer",
            "defaultMatchHistoryTag" | "defaultMatchHistoryTimeRange",
        ) => json!(""),
        ("main", "player-tabs-renderer", "searchHistory")
        | ("main", "renderer-debug-renderer", "savedRules") => json!([]),
        ("main", "simple-notifications-renderer", "lastNoticeRevision") => Value::Null,
        ("opgg", "opgg-renderer", "autoApplyItems" | "autoApplyRunes" | "autoApplySpells") => {
            json!(false)
        }
        ("opgg", "opgg-renderer", "savedPreferences") => json!({}),
        _ => return None,
    };
    Some(Preference {
        namespace,
        key,
        default,
    })
}

pub(super) struct Preference<'a> {
    namespace: &'a str,
    key: &'a str,
    default: Value,
}

impl Preference<'_> {
    pub(super) fn read(&self, db: &LeagueProfileDatabase) -> Result<Value, ProfileDatabaseError> {
        Ok(self.saved(db)?.unwrap_or_else(|| self.default.clone()))
    }

    pub(super) fn saved(
        &self,
        db: &LeagueProfileDatabase,
    ) -> Result<Option<Value>, ProfileDatabaseError> {
        Ok(db
            .setting(&self.storage_key())?
            .filter(|value| valid_setting(&self.default, value)))
    }

    pub(super) fn write(
        &self,
        db: &LeagueProfileDatabase,
        value: &Value,
    ) -> Result<(), ProfileDatabaseError> {
        if !valid_setting(&self.default, value) {
            return Err(invalid_setting("设置值无效。"));
        }
        if self.key == "displayMode" && !matches!(value.as_str(), Some("all" | "compact")) {
            return Err(invalid_setting("无效的 RESG 显示方式。"));
        }
        db.set_setting(&self.storage_key(), value)
    }

    fn storage_key(&self) -> String {
        format!("{}/{}", self.namespace, self.key)
    }
}

fn valid_setting(default: &Value, value: &Value) -> bool {
    match default {
        Value::Null => value.is_null() || value.is_string(),
        _ => std::mem::discriminant(default) == std::mem::discriminant(value),
    }
}

pub(super) fn resg_preferences(db: &LeagueProfileDatabase) -> Result<Value, ProfileDatabaseError> {
    Ok(settings(db, RESG_NAMESPACE)?.expect("known RESG settings"))
}

fn aram_defaults(key: &str) -> Option<Value> {
    let mut defaults = json!({
        "enabled":false,"champions":{"default":[],"top":[],"jungle":[],"middle":[],"bottom":[],"utility":[]},
        "delaySeconds":0,"strategy":"show-and-lock-in",
    });
    match key {
        "banConfig" => {}
        "pickConfig" => defaults.as_object_mut()?.extend([
            ("ignoreIntent".into(), json!(false)),
            ("showIntent".into(), json!(false)),
            ("benchHandleTradeEnabled".into(), json!(false)),
            ("benchSelectFirstAvailableChampion".into(), json!(false)),
            ("benchSwapAccumulatedDelaySeconds".into(), json!(2.9)),
        ]),
        _ => return None,
    }
    Some(defaults)
}

fn aram_config(db: &LeagueProfileDatabase, key: &str) -> Result<Value, ProfileDatabaseError> {
    let mut config = aram_defaults(key).ok_or_else(|| invalid_setting("无效的大乱斗配置。"))?;
    let saved = db.setting(&format!("{AUTO_SELECT}/{key}"))?;
    let Some(legacy) = saved.as_ref().and_then(|value| value["aram"].as_object()) else {
        return Ok(json!({}));
    };
    merge_aram(&mut config, legacy, false)?;
    Ok(json!({"aram":config}))
}

pub(super) fn set_aram_config(
    db: &LeagueProfileDatabase,
    key: &str,
    group: &str,
    patch: &Value,
) -> Result<Value, ProfileDatabaseError> {
    if group != "aram" {
        return Err(invalid_setting("只支持大乱斗选人设置。"));
    }
    let storage_key = format!("{AUTO_SELECT}/{key}");
    let mut config = aram_defaults(key).ok_or_else(|| invalid_setting("无效的大乱斗配置。"))?;
    let mut saved = db.setting(&storage_key)?.unwrap_or_else(|| json!({}));
    let all = saved
        .as_object_mut()
        .ok_or_else(|| invalid_setting("已有选人配置无法读取。"))?;
    if let Some(legacy) = all.get("aram").and_then(Value::as_object) {
        merge_aram(&mut config, legacy, false)?;
    }
    let patch = patch
        .as_object()
        .ok_or_else(|| invalid_setting("无效的大乱斗配置。"))?;
    merge_aram(&mut config, patch, true)?;
    all.insert("aram".into(), config.clone());
    db.set_setting(&storage_key, &saved)?;
    Ok(json!({"aram":config}))
}

fn merge_aram(
    config: &mut Value,
    patch: &Map<String, Value>,
    from_renderer: bool,
) -> Result<(), ProfileDatabaseError> {
    for (key, value) in patch {
        let valid = match key.as_str() {
            "champions" => {
                let Some(positions) = value.as_object() else {
                    if from_renderer {
                        return Err(invalid_setting("无效的英雄列表。"));
                    }
                    continue;
                };
                for (position, ids) in positions {
                    if (!from_renderer || position == "default")
                        && ids
                            .as_array()
                            .is_some_and(|ids| ids.iter().all(|id| id.as_i64().is_some()))
                    {
                        config[key][position] = ids.clone();
                    } else if from_renderer {
                        return Err(invalid_setting("无效的英雄列表。"));
                    }
                }
                continue;
            }
            "strategy" => matches!(
                value.as_str(),
                Some("just-show" | "show-and-lock-in" | "lock-in-immediately")
            ),
            key => config
                .get(key)
                .is_some_and(|default| valid_setting(default, value)),
        };
        if valid || (!from_renderer && config.get(key).is_none()) {
            config[key] = value.clone();
        } else if from_renderer {
            return Err(invalid_setting("无效的大乱斗配置。"));
        }
    }
    Ok(())
}

fn invalid_setting(message: &'static str) -> ProfileDatabaseError {
    io::Error::new(io::ErrorKind::InvalidInput, message).into()
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
