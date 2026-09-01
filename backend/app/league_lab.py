"""Local League Client (LCU) automation for the integrated League lab.

The authentication token is discovered from the running LeagueClientUx process,
kept in memory only, and never written to config or logs.
"""

from __future__ import annotations

import asyncio
import base64
import ctypes
import ctypes.wintypes as wintypes
import json
import logging
import math
import os
import random
import re
import ssl
import stat
import subprocess
import threading
import time
from dataclasses import dataclass
from itertools import combinations
from pathlib import Path
from typing import Literal
from urllib.parse import quote

import httpx
import aiosqlite
import websockets
from fastapi import APIRouter, HTTPException, Response
from pydantic import AliasChoices, BaseModel, ConfigDict, Field, model_validator

from .env_utils import get_data_dir


logger = logging.getLogger(__name__)
router = APIRouter(prefix="/api/league-lab", tags=["league-lab"])

_PORT_RE = re.compile(r"--app-port=(\d+)")
_TOKEN_RE = re.compile(r"--remoting-auth-token=([\w_-]+)")
_REGION_RE = re.compile(r"--region=([\w_-]+)", re.IGNORECASE)
_PLATFORM_RE = re.compile(r"--rso[_-]platform[_-]id=([\w_-]+)", re.IGNORECASE)
_RIOT_CLIENT_PORT_RE = re.compile(r"--riotclient-app-port=(\d+)")
_RIOT_CLIENT_TOKEN_RE = re.compile(r"--riotclient-auth-token=([\w_-]+)")


PositionKey = Literal["default", "top", "jungle", "middle", "bottom", "utility"]
PickStrategy = Literal["just-show", "show-and-lock-in", "lock-in-immediately"]


def _empty_position_pool() -> dict[str, list[int]]:
    return {key: [] for key in ("default", "top", "jungle", "middle", "bottom", "utility")}


class PickProfile(BaseModel):
    enabled: bool = False
    champions: dict[str, list[int]] = Field(default_factory=_empty_position_pool)
    # LeagueAkari's empty profile starts with no artificial delay.  The
    # scheduler still calibrates this value against the LCU phase timer.
    delay_seconds: float = Field(default=0.0, ge=0.0)
    ignore_intent: bool = False
    strategy: PickStrategy = "show-and-lock-in"
    show_intent: bool = False
    bench_select_first_available_champion: bool = False
    bench_swap_accumulated_delay_seconds: float = Field(default=2.9, ge=0.0)
    bench_handle_trade_enabled: bool = False


class BanProfile(BaseModel):
    enabled: bool = False
    champions: dict[str, list[int]] = Field(default_factory=_empty_position_pool)
    delay_seconds: float = Field(default=0.0, ge=0.0)
    strategy: PickStrategy = "show-and-lock-in"


class AutoSelectProfile(BaseModel):
    pick: PickProfile = Field(default_factory=PickProfile)
    ban: BanProfile = Field(default_factory=BanProfile)


def _default_auto_select_profiles() -> dict[str, AutoSelectProfile]:
    # Keep LeagueAkari's user-facing mode groups independently configurable.
    # Legacy group keys remain accepted below for a non-destructive migration.
    return {key: AutoSelectProfile() for key in (
        "ranked", "normal", "aram", "cherry", "urf", "oneforall",
        "ultbook", "bot", "custom", "default",
    )}


class InGameFixedTextPreset(BaseModel):
    id: str = Field(pattern=r"^[A-Za-z0-9_-]{1,80}$")
    title: str = Field(default="未命名预设", max_length=64)
    shortcut: str | None = Field(default=None, max_length=80)
    content: str = Field(default="", max_length=65536)


class InGamePresetTargetShortcuts(BaseModel):
    friendly: str | None = Field(default=None, max_length=80)
    enemy: str | None = Field(default=None, max_length=80)
    all: str | None = Field(default=None, max_length=80)


class _InGamePresetDraftBase(BaseModel):
    """Persistent, read-only options for the in-game-send preset drafts.

    The renderer currently keeps these values in its local draft object using
    camelCase keys.  Accept both that shape and the host's normal snake_case
    shape, but serialize the latter so the settings file remains consistent
    with the rest of the backend.  These are presentation/selection options
    only; they do not enable or perform any LCU write.
    """

    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    target_mode: Literal["all", "friendly", "enemy", "selected"] = Field(
        default="all", alias="targetMode"
    )
    selected_puuids: list[str] = Field(
        default_factory=list, max_length=10, alias="selectedPuuids"
    )
    name_display_strategy: Literal[
        "preferName", "preferChampionName", "championNameWithName"
    ] = Field(default="preferChampionName", alias="nameDisplayStrategy")


class InGameRatingPresetDisplay(BaseModel):
    """The twelve explicit Rating display switches from LeagueAkari."""

    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    win_rate: bool = Field(default=True, alias="winRate")
    kda: bool = True
    avg_solo_kills: bool = Field(default=True, alias="avgSoloKills")
    avg_vision_score: bool = Field(default=False, alias="avgVisionScore")
    avg_champion_damage: bool = Field(default=False, alias="avgChampionDamage")
    avg_damage_taken: bool = Field(default=False, alias="avgDamageTaken")
    avg_gold: bool = Field(default=False, alias="avgGold")
    avg_cs_per_minute: bool = Field(default=False, alias="avgCsPerMinute")
    avg_kill_participation: bool = Field(default=False, alias="avgKillParticipation")
    avg_damage_gold_efficiency: bool = Field(default=False, alias="avgDamageGoldEfficiency")
    main_champions: bool = Field(default=True, alias="mainChampions")
    main_positions: bool = Field(default=True, alias="mainPositions")


class InGameJunglePresetDisplay(BaseModel):
    """The six explicit Jungle display switches from LeagueAkari."""

    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    activity_preference: bool = Field(default=True, alias="activityPreference")
    first_clear_distribution: bool = Field(default=True, alias="firstClearDistribution")
    early_gank: bool = Field(default=True, alias="earlyGank")
    dragon_control: bool = Field(default=True, alias="dragonControl")
    monster_control: bool = Field(default=True, alias="monsterControl")
    main_champions: bool = Field(default=True, alias="mainChampions")


class InGameRatingPresetDraft(_InGamePresetDraftBase):
    show_current_champion: bool = Field(default=False, alias="showCurrentChampion")
    display: InGameRatingPresetDisplay = Field(default_factory=InGameRatingPresetDisplay)


class InGameJunglePresetDraft(_InGamePresetDraftBase):
    show_current_champion: bool = Field(default=True, alias="showCurrentChampion")
    display: InGameJunglePresetDisplay = Field(default_factory=InGameJunglePresetDisplay)


class InGamePremadePresetDraft(_InGamePresetDraftBase):
    """Premade has no metric display map in the renderer draft."""

    pass


class LeagueLabSettings(BaseModel):
    safety_migration_version: int = Field(default=2, ge=0)
    automation_enabled: bool = False
    auto_accept_enabled: bool = False
    auto_accept_delay_seconds: float = Field(default=0.0, ge=0.0, le=10.0)
    play_again_enabled: bool = False
    auto_reconnect_enabled: bool = False
    invitation_strategy: Literal["ignore", "accept", "decline"] = "ignore"
    auto_handle_invitations_enabled: bool = False
    reject_invitation_when_away: bool = False
    invitation_handling_strategies: dict[str, Literal["ignore", "accept", "decline"]] = Field(default_factory=dict)
    auto_skip_leader_enabled: bool = False
    auto_select_enabled: bool = False
    auto_pick_champion_ids: list[int] = Field(default_factory=list)
    auto_ban_champion_ids: list[int] = Field(default_factory=list)
    champion_action_delay_seconds: float = Field(default=1.0, ge=0.0, le=10.0)
    champion_lock_in: bool = True
    auto_select_profiles: dict[str, AutoSelectProfile] = Field(default_factory=_default_auto_select_profiles)
    auto_champion_config_enabled: bool = False
    # Canonical LeagueAkari auto-champ-config storage.  The legacy flat
    # ``champion_loadouts`` list remains accepted for backwards compatibility,
    # but the scheduler and renderer prefer these independently scoped pages.
    runes_v2: dict[int, dict[str, ChampionRunesConfig | None]] = Field(
        default_factory=dict, alias="runesV2"
    )
    summoner_spells: dict[int, dict[str, SummonerSpellsConfig | None]] = Field(
        default_factory=dict, alias="summonerSpells"
    )
    champion_loadouts: list["ChampionLoadout"] = Field(default_factory=list)
    auto_honor_enabled: bool = False
    auto_honor_strategy: Literal[
        "prefer-lobby-member", "only-lobby-member", "all-member", "opt-out", "all-member-including-opponent"
    ] = "prefer-lobby-member"
    auto_matchmaking_enabled: bool = False
    auto_matchmaking_delay_seconds: float = Field(default=5.0, ge=0.0)
    auto_matchmaking_maximum_match_duration: float = Field(default=0.0, ge=0.0)
    auto_matchmaking_minimum_members: int = Field(default=1, ge=1, le=99)
    auto_matchmaking_wait_for_invitees: bool = True
    auto_matchmaking_chat_countdown_enabled: bool = False
    auto_matchmaking_rematch_strategy: Literal["never", "fixed-duration", "estimated-duration"] = "never"
    auto_matchmaking_rematch_fixed_duration: float = Field(default=2.0, ge=1.0)
    auto_reply_enabled: bool = False
    auto_reply_only_away: bool = False
    auto_reply_text: str = Field(default="", max_length=500)
    lock_offline_status: bool = False
    auto_set_status_message_enabled: bool = False
    status_message: str = Field(default="", max_length=500)
    auto_set_ranked_status_enabled: bool = False
    ranked_status: "RankedStatusUpdate" = Field(default_factory=lambda: RankedStatusUpdate())
    auto_send_aram_team_side_enabled: bool = False
    auto_send_aram_team_side_visible_to_team: bool = False
    auto_invite_friend_puuids: list[str] = Field(default_factory=list, max_length=20)
    mini_enabled: bool = True
    mini_auto_show: bool = True
    mini_opacity: float = Field(default=1.0, ge=0.4, le=1.0)
    mini_pinned: bool = True
    mini_show_skin_selector: bool = True
    # Keep the native auxiliary-window pin choices with the rest of the
    # League settings.  The Tauri window-state plugin persists geometry only;
    # these fields make the always-on-top choice survive a restart and allow
    # it to travel through settings backups.
    ongoing_pinned: bool = True
    cooldown_pinned: bool = True
    match_history_refresh_after_game: bool = True
    match_history_load_count: int = Field(default=20, ge=1, le=100)
    # LeagueAkari's player-tabs renderer keeps these preferences together with
    # the tab strip.  Keep the two existing match-history keys as the
    # compatibility surface used by this host, and persist the remaining
    # renderer preferences explicitly so an upstream settings export can be
    # migrated without silently dropping them.
    player_tabs_match_history_use_sgp_api: bool = True
    player_tabs_default_match_history_tag: str = Field(default="<akari:all>", max_length=200)
    player_tabs_default_match_history_time_range: Literal["all", "24h", "3d", "7d", "30d"] = "all"
    player_tabs_default_show_practice: bool = False
    player_tabs_default_show_irregular_games: bool = False
    ongoing_auto_route_when_game_starts: bool = False
    ongoing_match_history_load_count: int = Field(default=20, ge=1, le=100)
    ongoing_query_concurrency: int = Field(default=4, ge=1, le=20)
    ongoing_game_details_load_count: int = Field(default=20, ge=0, le=100)
    ongoing_match_history_tag_preference: Literal["current", "all"] = "current"
    ongoing_order_player_by: Literal["win-rate", "kda", "default", "akari-score", "position", "premade-team"] = "default"
    ongoing_query_in_lobby_phase: bool = True
    ongoing_premade_threshold: int = Field(default=5, ge=1, le=20)
    ongoing_jungle_analysis_count: int = Field(default=4, ge=1, le=20)
    ongoing_show_champion_usage: bool = True
    ongoing_champion_usage_mode: Literal["recent", "mastery", "none"] = "recent"
    ongoing_show_match_history_item_border: bool = False
    ongoing_show_jungle_pathing: bool = True
    ongoing_show_jungle_pathing_for_all_players: bool = False
    ongoing_show_premade_tag: bool = True
    ongoing_show_local_tag: bool = True
    ongoing_show_streak_tags: bool = True
    ongoing_show_performance_tags: bool = True
    ongoing_player_card_tag_settings: dict[str, bool] = Field(default_factory=lambda: {
        "self": True,
        "tagged": True,
        "met": True,
        "privacy": True,
        "high-win-rate": True,
        "winning-streak": True,
        "losing-streak": True,
        "great-performance": True,
        "suspicious-flash-position": True,
        "solo-kills": True,
        "easy-gank": True,
        "win-rate-team": True,
        "average-team-damage": False,
        "average-team-damage-taken": False,
        "average-team-gold": False,
        "average-cs-per-minute": False,
        "average-damage-gold-efficiency": False,
        "average-enemy-missing-pings": False,
        "average-vision-score": False,
        "average-kill-damage-efficiency": True,
        "akari-score": False,
    })
    respawn_timer_enabled: bool = False
    cooldown_timer_enabled: bool = False
    cooldown_timer_type: Literal["countdown", "countup"] = "countdown"
    cooldown_timer_reverse_adjustment: bool = False
    streamer_mode_enabled: bool = False
    streamer_mode_use_aliases: bool = False
    streamer_content_protection_enabled: bool = False
    # The single account-write gate is still checked immediately before every
    # LCU write.  New installs start enabled so the prominent toolkit switch
    # is usable out of the box; users can turn it off to return to read-only.
    toolkit_account_actions_enabled: bool = True
    terminate_game_shortcut_enabled: bool = False
    terminate_game_shortcut: str = Field(default="Ctrl+Alt+End", min_length=3, max_length=80)
    in_game_send_enabled: bool = False
    in_game_send_interval_ms: int = Field(default=250, ge=100, le=5000)
    in_game_cancel_shortcut: str | None = Field(default=None, max_length=80)
    in_game_fixed_presets: list[InGameFixedTextPreset] = Field(default_factory=list, max_length=100)
    in_game_rating_shortcuts: InGamePresetTargetShortcuts = Field(default_factory=InGamePresetTargetShortcuts)
    in_game_premade_shortcuts: InGamePresetTargetShortcuts = Field(default_factory=InGamePresetTargetShortcuts)
    in_game_jungle_shortcuts: InGamePresetTargetShortcuts = Field(default_factory=InGamePresetTargetShortcuts)
    # Keep the canonical names close to the upstream preset options while
    # accepting the shorter draft names used by the host UI during migration.
    in_game_rating_preset_options: InGameRatingPresetDraft = Field(
        default_factory=InGameRatingPresetDraft,
        validation_alias=AliasChoices("in_game_rating_preset_options", "in_game_rating_preset"),
    )
    in_game_jungle_preset_options: InGameJunglePresetDraft = Field(
        default_factory=InGameJunglePresetDraft,
        validation_alias=AliasChoices("in_game_jungle_preset_options", "in_game_jungle_preset"),
    )
    in_game_premade_preset_options: InGamePremadePresetDraft = Field(
        default_factory=InGamePremadePresetDraft,
        validation_alias=AliasChoices("in_game_premade_preset_options", "in_game_premade_preset"),
    )
    ongoing_window_shortcut: str | None = Field(default=None, max_length=80)
    cooldown_window_shortcut: str | None = Field(default=None, max_length=80)

    @model_validator(mode="before")
    @classmethod
    def migrate_legacy_ongoing_settings(cls, value):
        if not isinstance(value, dict):
            return value
        migrated = dict(value)
        # Removed feature keys are intentionally discarded during upgrade.
        # Pydantic would ignore them as extras, but removing them explicitly
        # prevents old settings from being written back into a fresh file.
        for legacy_key in tuple(migrated):
            if str(legacy_key).startswith("opgg_"):
                migrated.pop(legacy_key, None)
        if "ongoing_champion_usage_mode" not in migrated and migrated.get("ongoing_show_champion_usage") is False:
            migrated["ongoing_champion_usage_mode"] = "none"
        # Upstream JSON exports store player-tabs settings as one namespaced
        # object.  Accept both the current namespace and the pre-1.4 name,
        # then flatten to this service's stable JSON model.
        nested = None
        for key in (
            "player-tabs-renderer/frontendSettings",
            "match-history-tabs-renderer/frontendSettings",
            "player_tabs_frontend_settings",
        ):
            candidate = migrated.get(key)
            if isinstance(candidate, dict):
                nested = candidate
                break
        if isinstance(nested, dict):
            aliases = {
                "refreshTabsAfterGameEnds": "match_history_refresh_after_game",
                "loadCount": "match_history_load_count",
                "matchHistoryUseSgpApi": "player_tabs_match_history_use_sgp_api",
                "defaultMatchHistoryTag": "player_tabs_default_match_history_tag",
                "defaultMatchHistoryTimeRange": "player_tabs_default_match_history_time_range",
                "defaultShowPractice": "player_tabs_default_show_practice",
                "defaultShowIrregularGames": "player_tabs_default_show_irregular_games",
            }
            for source, target in aliases.items():
                if target not in migrated and source in nested:
                    migrated[target] = nested[source]
            migrated.pop("player-tabs-renderer/frontendSettings", None)
            migrated.pop("match-history-tabs-renderer/frontendSettings", None)
            migrated.pop("player_tabs_frontend_settings", None)
        # Also accept a flat camelCase payload produced by older renderer-only
        # backups.  The explicit keys above remain canonical on disk.
        flat_aliases = {
            "refreshTabsAfterGameEnds": "match_history_refresh_after_game",
            "loadCount": "match_history_load_count",
            "matchHistoryUseSgpApi": "player_tabs_match_history_use_sgp_api",
            "defaultMatchHistoryTag": "player_tabs_default_match_history_tag",
            "defaultMatchHistoryTimeRange": "player_tabs_default_match_history_time_range",
            "defaultShowPractice": "player_tabs_default_show_practice",
            "defaultShowIrregularGames": "player_tabs_default_show_irregular_games",
        }
        for source, target in flat_aliases.items():
            if target not in migrated and source in migrated:
                migrated[target] = migrated[source]
            migrated.pop(source, None)
        profiles = migrated.get("auto_select_profiles")
        if isinstance(profiles, dict):
            next_profiles = dict(profiles)
            legacy_aliases = {
                "arena": "cherry",
                "doom-bots": "ultbook",
            }
            for legacy, current in legacy_aliases.items():
                if current not in next_profiles and legacy in next_profiles:
                    next_profiles[current] = next_profiles[legacy]
            for key, profile in _default_auto_select_profiles().items():
                next_profiles.setdefault(key, profile.model_dump())
            migrated["auto_select_profiles"] = next_profiles
        return migrated

    @property
    def in_game_rating_preset(self) -> InGameRatingPresetDraft:
        """Compatibility accessor for the renderer's shorter draft name."""

        return self.in_game_rating_preset_options

    @property
    def in_game_jungle_preset(self) -> InGameJunglePresetDraft:
        return self.in_game_jungle_preset_options

    @property
    def in_game_premade_preset(self) -> InGamePremadePresetDraft:
        return self.in_game_premade_preset_options


class ChampionLoadout(BaseModel):
    champion_id: int = Field(gt=0)
    config_key: str = Field(default="default", pattern=r"^(default|normal|aram|urf|nexusblitz|ultbook|ranked-default|ranked-(top|jungle|middle|bottom|utility))$")
    primary_style_id: int = Field(gt=0)
    sub_style_id: int = Field(gt=0)
    selected_perk_ids: list[int] = Field(default_factory=list)
    spell1_id: int = Field(gt=0)
    spell2_id: int = Field(gt=0)


class ChampionRunesConfig(BaseModel):
    """LeagueAkari-compatible Rune V2 page shape (LCU schema version 2)."""

    primary_style_id: int = Field(gt=0, alias="primaryStyleId")
    sub_style_id: int = Field(gt=0, alias="subStyleId")
    selected_perk_ids: list[int] = Field(default_factory=list, alias="selectedPerkIds")

    model_config = ConfigDict(populate_by_name=True, extra="forbid")


class SummonerSpellsConfig(BaseModel):
    """LeagueAkari-compatible two summoner spell selection."""

    spell1_id: int = Field(gt=0, alias="spell1Id")
    spell2_id: int = Field(gt=0, alias="spell2Id")

    model_config = ConfigDict(populate_by_name=True, extra="forbid")


class ChatPresenceUpdate(BaseModel):
    availability: Literal["chat", "mobile", "away", "offline", "dnd", "spectating", "online"] | None = None
    status_message: str | None = Field(default=None, max_length=500)


class RankedStatusUpdate(BaseModel):
    queue: str = Field(default="RANKED_SOLO_5x5", min_length=1, max_length=80)
    tier: Literal[
        "IRON", "BRONZE", "SILVER", "GOLD", "PLATINUM", "EMERALD", "DIAMOND", "MASTER", "GRANDMASTER", "CHALLENGER"
    ] = "CHALLENGER"
    division: Literal["I", "II", "III", "IV"] = "I"


class ChatMessageSend(BaseModel):
    lines: list[str] = Field(min_length=1, max_length=10)


class InGameTextSend(BaseModel):
    text: str = Field(min_length=1, max_length=300)


class InGamePresetSend(BaseModel):
    preset_id: str = Field(pattern=r"^[A-Za-z0-9_-]{1,80}$")
    trigger: Literal["manual", "shortcut"] = "manual"
    confirmation: str = Field(default="", max_length=20)


class InGameAdHocSend(BaseModel):
    lines: list[str] = Field(min_length=1, max_length=10)
    trigger: Literal["manual", "shortcut"] = "manual"
    kind: Literal["rating", "premade", "jungle"] | None = None
    target: Literal["friendly", "enemy", "all"] | None = None
    confirmation: str = Field(default="", max_length=20)


class GameSettingsFileModeUpdate(BaseModel):
    mode: Literal["readonly", "writable"]


class LeagueClientWindowResize(BaseModel):
    base_width: int = Field(default=1280, ge=640, le=3840)
    base_height: int = Field(default=720, ge=360, le=2160)


class LeagueClientSelect(BaseModel):
    pid: int = Field(gt=0)


class LeagueClientLaunch(BaseModel):
    kind: Literal["tcls", "wegame-lol", "wegame", "riot"]


class LeagueReplayPrepare(BaseModel):
    game_version: str = Field(default="", max_length=80)
    game_type: str = Field(default="", max_length=80)
    queue_id: int = Field(default=0, ge=0, le=100000)
    game_end: int = Field(default=0, ge=0)


class LeagueMatchCollectRequest(BaseModel):
    """Read-only LeagueAkari-style match-history collection settings.

    Collection intentionally carries only a serializable predicate tree.  It
    never accepts an LCU endpoint or credentials and therefore cannot turn the
    history collector into an arbitrary client writer.
    """

    count_per_iteration: int = Field(default=20, ge=1, le=100)
    expected_count: int = Field(default=20, ge=1, le=1000)
    max_iteration: int = Field(default=50, ge=1, le=100)
    filter_tree: dict = Field(default_factory=dict)
    query: str = Field(default="", max_length=160)
    queue_id: int | None = Field(default=None, ge=0, le=100000)
    result: Literal["all", "win", "loss"] = "all"


class MissionRewardClaim(BaseModel):
    mission_id: str = Field(min_length=1, max_length=160)
    reward_group_ids: list[str] = Field(min_length=1, max_length=20)
    confirmation: Literal["我确认领取"]


class RewardGrantClaim(BaseModel):
    grant_id: str = Field(min_length=1, max_length=160)
    reward_group_id: str = Field(min_length=1, max_length=160)
    selection_ids: list[str] = Field(min_length=1, max_length=20)
    confirmation: Literal["我确认领取"]


class EventRewardClaim(BaseModel):
    event_id: str = Field(min_length=1, max_length=160)
    confirmation: Literal["我确认领取"]


class FriendDeleteRequest(BaseModel):
    friend_ids: list[str] = Field(min_length=1, max_length=50)
    confirmation: Literal["我确认删除"]


class AutoInviteFriendSchedule(BaseModel):
    """Local-only list of friends to invite when they become available.

    The list is a scheduler input, not an invitation command.  Updating it
    only persists the PUUIDs; the background event handler performs an LCU
    invitation only when the existing automation master switch is explicitly
    enabled.
    """

    puuids: list[str] = Field(
        default_factory=list,
        max_length=20,
        validation_alias=AliasChoices("puuids", "friend_puuids", "friendPuuids"),
    )


class SearchHistoryPinBody(BaseModel):
    pinned: bool


class ChampSelectDodgeRequest(BaseModel):
    confirmation: Literal["我确认秒退"]


class ChampSelectCharityRerollRequest(BaseModel):
    """Explicit confirmation for LeagueAkari-style charity reroll.

    A charity reroll is two account writes at most (reroll, then an optional
    bench swap), so it must never be reachable through an implicit/default
    request body.
    """

    confirmation: Literal["我确认慈善重随"]


class ChampSelectDodgeLoopRequest(BaseModel):
    """Explicit confirmation required before starting the dodge loop."""

    confirmation: Literal["我确认秒退"]


class TerminateGameClientRequest(BaseModel):
    confirmation: Literal["我确认结束游戏"]


class AutoSelectTemporaryDisableBody(BaseModel):
    disabled: bool


class QueueLobbyCreate(BaseModel):
    queue_id: int = Field(gt=0, le=100000)
    confirmation: Literal["我确认创建"]


class LeaveLobbyRequest(BaseModel):
    confirmation: Literal["我确认离开"]


class StrawberryPlayerUpdate(BaseModel):
    champion_id: int = Field(gt=0)
    map_item_id: int = Field(default=1, gt=0)
    difficulty: int = Field(default=1, ge=1, le=3)
    confirmation: Literal["我确认修改"]


class StrawberryMapUpdate(BaseModel):
    content_id: str = Field(min_length=1, max_length=200)
    item_id: int = Field(gt=0)
    confirmation: Literal["我确认修改"]


class StrawberryDifficultyUpdate(BaseModel):
    difficulty: int = Field(ge=1, le=3)
    confirmation: Literal["我确认修改"]


class ProfileBackgroundUpdate(BaseModel):
    champion_id: int = Field(gt=0)
    skin_id: int = Field(gt=0)
    augment_id: str | None = Field(default=None, max_length=200)
    confirmation: Literal["我确认修改"]


class ProfileUtilityAction(BaseModel):
    action: Literal["banner-accent", "remove-prestige-crest", "clear-challenge-tokens", "clear-emotes"]
    confirmation: Literal["我确认修改"]


LeagueLabSettings.model_rebuild()


@dataclass(frozen=True)
class LcuCredentials:
    port: int
    token: str
    region: str = ""
    platform_id: str = ""
    riot_client_port: int = 0
    riot_client_token: str = ""
    pid: int = 0

    @property
    def base_url(self) -> str:
        return f"https://127.0.0.1:{self.port}"

    @property
    def auth_header(self) -> str:
        encoded = base64.b64encode(f"riot:{self.token}".encode("utf-8")).decode("ascii")
        return f"Basic {encoded}"

    @property
    def riot_client_base_url(self) -> str:
        return f"https://127.0.0.1:{self.riot_client_port}"

    @property
    def riot_client_auth_header(self) -> str:
        encoded = base64.b64encode(f"riot:{self.riot_client_token}".encode("utf-8")).decode("ascii")
        return f"Basic {encoded}"


def parse_league_client_command_line(command_line: str, *, pid: int = 0) -> LcuCredentials | None:
    port_match = _PORT_RE.search(command_line or "")
    token_match = _TOKEN_RE.search(command_line or "")
    if not port_match or not token_match:
        return None
    region_match = _REGION_RE.search(command_line)
    platform_match = _PLATFORM_RE.search(command_line)
    riot_client_port_match = _RIOT_CLIENT_PORT_RE.search(command_line)
    riot_client_token_match = _RIOT_CLIENT_TOKEN_RE.search(command_line)
    return LcuCredentials(
        port=int(port_match.group(1)),
        token=token_match.group(1),
        region=region_match.group(1) if region_match else "",
        platform_id=platform_match.group(1) if platform_match else "",
        riot_client_port=int(riot_client_port_match.group(1)) if riot_client_port_match else 0,
        riot_client_token=riot_client_token_match.group(1) if riot_client_token_match else "",
        pid=pid,
    )


async def discover_lcu_clients() -> list[LcuCredentials]:
    if os.name != "nt":
        return []
    script = "Get-Process LeagueClientUx -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id"
    creation_flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)

    def read_command_lines() -> list[LcuCredentials]:
        completed = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=5.0,
            check=False,
            creationflags=creation_flags,
        )
        pids = []
        for value in completed.stdout.decode("ascii", errors="ignore").split():
            try:
                pids.append(int(value))
            except ValueError:
                continue
        clients = []
        for pid in pids:
            parsed = parse_league_client_command_line(_read_windows_process_command_line(pid), pid=pid)
            if parsed:
                clients.append(parsed)
        if clients:
            return clients

        # Some Tencent/WeGame or elevated LeagueClientUx processes deny the
        # native ProcessCommandLineInformation query. CIM is slower, but makes
        # a reliable read-only fallback and the returned token never leaves
        # this process or reaches logs/configuration.
        cim_script = (
            "Get-CimInstance Win32_Process -Filter \"Name='LeagueClientUx.exe'\" "
            "| Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress"
        )
        cim = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", cim_script],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=5.0,
            check=False,
            creationflags=creation_flags,
        )
        try:
            rows = json.loads(cim.stdout.decode("utf-8", errors="ignore") or "[]")
        except (json.JSONDecodeError, UnicodeDecodeError):
            rows = []
        if isinstance(rows, dict):
            rows = [rows]
        for row in rows if isinstance(rows, list) else []:
            if not isinstance(row, dict):
                continue
            try:
                pid = int(row.get("ProcessId") or 0)
            except (TypeError, ValueError):
                continue
            parsed = parse_league_client_command_line(str(row.get("CommandLine") or ""), pid=pid)
            if parsed:
                clients.append(parsed)
        return clients

    try:
        return await asyncio.to_thread(read_command_lines)
    except (OSError, subprocess.TimeoutExpired):
        return []


async def discover_lcu_credentials(preferred_pid: int | None = None) -> LcuCredentials | None:
    clients = await discover_lcu_clients()
    if preferred_pid:
        selected = next((client for client in clients if client.pid == preferred_pid), None)
        if selected:
            return selected
    return clients[0] if clients else None


def _read_windows_process_command_line(pid: int) -> str:
    """Read ProcessCommandLineInformation exactly as LeagueAkari's native addon does."""
    if os.name != "nt" or pid <= 0:
        return ""
    process_query_limited_information = 0x1000
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    ntdll = ctypes.WinDLL("ntdll")
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    handle = kernel32.OpenProcess(process_query_limited_information, False, pid)
    if not handle:
        return ""
    try:
        query = ntdll.NtQueryInformationProcess
        query.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.ULONG, ctypes.POINTER(wintypes.ULONG)]
        query.restype = ctypes.c_long
        needed = wintypes.ULONG()
        query(handle, 60, None, 0, ctypes.byref(needed))
        if not needed.value:
            return ""
        buffer = ctypes.create_string_buffer(needed.value)
        status = query(handle, 60, buffer, needed.value, ctypes.byref(needed))
        if status < 0:
            return ""

        class UnicodeString(ctypes.Structure):
            _fields_ = [("Length", wintypes.USHORT), ("MaximumLength", wintypes.USHORT), ("Buffer", ctypes.c_void_p)]

        value = UnicodeString.from_buffer(buffer)
        return ctypes.wstring_at(value.Buffer, value.Length // ctypes.sizeof(ctypes.c_wchar)) if value.Buffer else ""
    finally:
        kernel32.CloseHandle(handle)


def _league_client_window_is_present() -> bool:
    """Detect the logged-in League UX without opening the elevated process.

    Tencent/WeGame commonly launches LeagueClientUx at a higher integrity
    level. Windows then intentionally blocks command-line reads from this
    process, but FindWindow remains a safe way to distinguish that state from
    "the client is not running".
    """
    if os.name != "nt":
        return False
    try:
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
        user32.FindWindowW.restype = wintypes.HWND
        return bool(user32.FindWindowW("RCLIENT", "League of Legends"))
    except (AttributeError, OSError):
        return False


def _read_registry_string(root, key_path: str, value_name: str) -> str:
    try:
        import winreg

        with winreg.OpenKey(root, key_path) as key:
            value, _ = winreg.QueryValueEx(key, value_name)
        return str(value or "").strip()
    except (ImportError, OSError):
        return ""


def _existing_executable(path: str | Path | None) -> str:
    if not path:
        return ""
    candidate = Path(path).expanduser()
    try:
        return str(candidate.resolve()) if candidate.is_file() else ""
    except OSError:
        return ""


def _detect_client_installations_sync() -> dict[str, dict]:
    """Detect the same Tencent/WeGame/Riot launch surfaces exposed by LeagueAkari."""
    if os.name != "nt":
        return {}
    try:
        import winreg
    except ImportError:
        return {}

    detected: dict[str, dict] = {}
    install_dir = _read_registry_string(winreg.HKEY_CURRENT_USER, r"Software\Tencent\LOL", "InstallPath")
    candidates: list[tuple[str, str, Path, list[str]]] = []
    if install_dir:
        root = Path(install_dir)
        candidates.extend([
            ("tcls", "腾讯 TCLS", root / "Launcher" / "Client.exe", []),
            ("wegame-lol", "WeGame 英雄联盟", root / "WeGameLauncher" / "launcher.exe", []),
        ])

    default_icon = _read_registry_string(winreg.HKEY_CURRENT_USER, r"wegame\DefaultIcon", "")
    icon_match = re.match(r'^"([^"]+)"|^([^,]+)', default_icon)
    if icon_match:
        candidates.append(("wegame", "WeGame", Path(icon_match.group(1) or icon_match.group(2).strip()), []))

    for drive_ord in range(ord("C"), ord("Z") + 1):
        root = Path(f"{chr(drive_ord)}:\\WeGameApps\\英雄联盟")
        if root.is_dir():
            candidates.extend([
                ("tcls", "腾讯 TCLS", root / "Launcher" / "Client.exe", []),
                ("wegame-lol", "WeGame 英雄联盟", root / "WeGameLauncher" / "launcher.exe", []),
            ])

    program_data = Path(os.environ.get("ProgramData") or r"C:\ProgramData")
    installs_file = program_data / "Riot Games" / "RiotClientInstalls.json"
    try:
        installs = json.loads(installs_file.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        installs = {}
    for value in (installs.get("associated_client") or {}).values() if isinstance(installs, dict) else []:
        path = Path(str(value or ""))
        if "riot games" in str(path).lower() and "英雄联盟" not in str(path):
            candidates.append(("riot", "Riot Client", path, ["--launch-product=league_of_legends", "--launch-patchline=live"]))
    candidates.append((
        "riot",
        "Riot Client",
        Path(f"{os.environ.get('SystemDrive') or 'C:'}\\") / "Riot Games" / "Riot Client" / "RiotClientServices.exe",
        ["--launch-product=league_of_legends", "--launch-patchline=live"],
    ))

    for kind, label, candidate, args in candidates:
        executable = _existing_executable(candidate)
        if executable and kind not in detected:
            detected[kind] = {"kind": kind, "label": label, "path": executable, "args": args}
    return detected


async def detect_client_installations() -> dict[str, dict]:
    return await asyncio.to_thread(_detect_client_installations_sync)


def _shell_execute_windows(executable: str, args: list[str]) -> None:
    """Launch a Windows shell entry point and fail on ShellExecute error codes."""
    if os.name != "nt":
        raise OSError("Windows shell launch is unavailable on this platform")
    shell32 = ctypes.WinDLL("shell32", use_last_error=True)
    shell32.ShellExecuteW.argtypes = [
        wintypes.HWND,
        wintypes.LPCWSTR,
        wintypes.LPCWSTR,
        wintypes.LPCWSTR,
        wintypes.LPCWSTR,
        ctypes.c_int,
    ]
    shell32.ShellExecuteW.restype = wintypes.HINSTANCE
    parameters = subprocess.list2cmdline(args) if args else None
    result = shell32.ShellExecuteW(
        None,
        "open",
        executable,
        parameters,
        str(Path(executable).parent),
        1,
    )
    result_value = ctypes.cast(result, ctypes.c_void_p).value or 0
    if result_value <= 32:
        raise OSError(f"ShellExecuteW failed with code {result_value}")


async def launch_detected_client(kind: str) -> dict:
    installations = await detect_client_installations()
    target = installations.get(kind)
    if not target:
        raise RuntimeError("未检测到所选客户端安装路径")
    executable = _existing_executable(target.get("path"))
    if not executable:
        raise RuntimeError("客户端安装路径已失效，请重新检测")

    def launch() -> None:
        command = [executable, *(target.get("args") or [])]
        # Tencent's TCLS and WeGame executables are Windows shell entry points.
        # Direct CreateProcess and detached cmd launches can both return success
        # while the login program never appears; ShellExecuteW is the same
        # launch path that Windows Explorer and Start-Process use successfully.
        if kind in {"tcls", "wegame-lol", "wegame"}:
            _shell_execute_windows(executable, list(target.get("args") or []))
            return

        creation_flags = getattr(subprocess, "DETACHED_PROCESS", 0) | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
        popen_options = {
            "cwd": str(Path(executable).parent),
            "stdin": subprocess.DEVNULL,
            "stdout": subprocess.DEVNULL,
            "stderr": subprocess.DEVNULL,
            "close_fds": True,
            "creationflags": creation_flags,
        }
        # RiotClientServices remains an argument-array launch so its fixed
        # product flags never pass through a shell.
        process = subprocess.Popen(command, **popen_options)
        process.poll()

    try:
        await asyncio.to_thread(launch)
    except OSError as exc:
        raise RuntimeError(f"客户端启动失败: {type(exc).__name__}") from exc
    return {"started": True, "kind": kind, "label": target["label"]}


def _terminate_foreground_league_game_client() -> int:
    """Terminate only a foreground League of Legends.exe process."""
    if os.name != "nt":
        raise RuntimeError("游戏进程控制仅支持 Windows")
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.GetForegroundWindow.restype = wintypes.HWND
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowThreadProcessId.restype = wintypes.DWORD
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.QueryFullProcessImageNameW.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        wintypes.LPWSTR,
        ctypes.POINTER(wintypes.DWORD),
    ]
    kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
    kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.TerminateProcess.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        raise RuntimeError("当前没有前台窗口")
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if not pid.value:
        raise RuntimeError("无法识别前台进程")
    process_query_limited_information = 0x1000
    process_terminate = 0x0001
    handle = kernel32.OpenProcess(
        process_query_limited_information | process_terminate,
        False,
        pid.value,
    )
    if not handle:
        raise RuntimeError("无法访问前台进程；若游戏以管理员身份运行，请以相同权限启动本软件")
    try:
        size = wintypes.DWORD(32768)
        image_path = ctypes.create_unicode_buffer(size.value)
        if not kernel32.QueryFullProcessImageNameW(handle, 0, image_path, ctypes.byref(size)):
            raise RuntimeError("无法验证前台进程名称")
        if Path(image_path.value).name.casefold() != "league of legends.exe":
            raise RuntimeError("当前前台窗口不是 League 游戏进程，未执行任何操作")
        if not kernel32.TerminateProcess(handle, 1):
            raise RuntimeError("结束 League 游戏进程失败")
        return int(pid.value)
    finally:
        kernel32.CloseHandle(handle)


def _send_text_to_foreground_league_game(text: str) -> int:
    """Send explicit user-requested chat text only to a foreground League game window."""
    if os.name != "nt":
        raise RuntimeError("游戏内文字发送仅支持 Windows")
    normalized = text.strip()
    if not normalized:
        raise RuntimeError("发送内容不能为空")
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.GetForegroundWindow.restype = wintypes.HWND
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowThreadProcessId.restype = wintypes.DWORD
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.QueryFullProcessImageNameW.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        wintypes.LPWSTR,
        ctypes.POINTER(wintypes.DWORD),
    ]
    kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        raise RuntimeError("当前没有前台窗口")
    pid = wintypes.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if not pid.value:
        raise RuntimeError("无法识别前台进程")
    handle = kernel32.OpenProcess(0x1000, False, pid.value)
    if not handle:
        raise RuntimeError("无法访问前台进程；若游戏以管理员身份运行，请以相同权限启动本软件")
    try:
        size = wintypes.DWORD(32768)
        image_path = ctypes.create_unicode_buffer(size.value)
        if not kernel32.QueryFullProcessImageNameW(handle, 0, image_path, ctypes.byref(size)):
            raise RuntimeError("无法验证前台进程名称")
        if Path(image_path.value).name.casefold() != "league of legends.exe":
            raise RuntimeError("当前前台窗口不是 League 游戏进程，未发送任何按键")
    finally:
        kernel32.CloseHandle(handle)

    class KeyboardInput(ctypes.Structure):
        _fields_ = [
            ("virtual_key", wintypes.WORD),
            ("scan_code", wintypes.WORD),
            ("flags", wintypes.DWORD),
            ("time", wintypes.DWORD),
            ("extra_info", ctypes.c_size_t),
        ]

    class InputUnion(ctypes.Union):
        _fields_ = [("keyboard", KeyboardInput)]

    class Input(ctypes.Structure):
        _anonymous_ = ("value",)
        _fields_ = [("input_type", wintypes.DWORD), ("value", InputUnion)]

    user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(Input), ctypes.c_int]
    user32.SendInput.restype = wintypes.UINT

    def send_key(virtual_key: int = 0, scan_code: int = 0, flags: int = 0) -> None:
        event = Input(input_type=1, keyboard=KeyboardInput(virtual_key, scan_code, flags, 0, 0))
        if user32.SendInput(1, ctypes.byref(event), ctypes.sizeof(Input)) != 1:
            raise RuntimeError("Windows 未接受游戏内键盘输入")

    send_key(0x0D)
    time.sleep(0.02)
    send_key(0x0D, flags=0x0002)
    time.sleep(0.065)
    encoded = normalized.encode("utf-16-le")
    for index in range(0, len(encoded), 2):
        code_unit = int.from_bytes(encoded[index:index + 2], "little")
        send_key(scan_code=code_unit, flags=0x0004)
        send_key(scan_code=code_unit, flags=0x0004 | 0x0002)
    time.sleep(0.065)
    send_key(0x0D)
    time.sleep(0.02)
    send_key(0x0D, flags=0x0002)
    return int(pid.value)


class LeagueLabService:
    _SNAPSHOT_CACHE_TTL_SECONDS = 0.35

    def __init__(self) -> None:
        self.settings = self._load_settings()
        self.credentials: LcuCredentials | None = None
        self.phase = ""
        self.game_mode = ""
        # Read-only gameflow snapshot used by the Mini surface.  LeagueAkari's
        # Lounge renders the current queue/map directly from the LCU session;
        # keep the same evidence available in every lounge phase instead of
        # making the frontend guess from the gameflow phase alone.
        self.gameflow_session: dict = {}
        self.summoner_name = ""
        self.current_summoner: dict = {}
        self.last_error = ""
        self.last_action = ""
        self.last_action_at = 0.0
        self._task: asyncio.Task | None = None
        self._event_task: asyncio.Task | None = None
        self._dodge_loop_task: asyncio.Task | None = None
        self._dodge_loop_cancel_event: asyncio.Event | None = None
        self._dodge_loop_state: dict = {
            "active": False,
            "attempts": 0,
            "concurrency": 5,
            "started_at": None,
            "stopped_at": None,
            "stop_reason": None,
            "last_error": None,
        }
        self._event_wakeup = asyncio.Event()
        self._event_connected = False
        self._event_sequence = 0
        self._auto_accept_terminal_response = False
        self._auto_accept_submitted = False
        self._auto_accept_credentials: LcuCredentials | None = None
        self._auto_accept_observed_phase = ""
        self._auto_accept_round_identity: str | None = None
        self._auto_accept_round_generation = 0
        # ``snapshot`` is called by several status consumers (including the
        # Mini lifecycle).  Keep only a short-lived refresh marker here: the
        # public status is still rebuilt on every call so countdowns remain
        # monotonic and phase-sensitive data is not served from a stale JSON
        # payload.  The lock collapses concurrent callers into one LCU read
        # cycle without changing any automation/write semantics.
        self._snapshot_refresh_lock = asyncio.Lock()
        self._snapshot_cache_at = 0.0
        self._snapshot_generation = 0
        self._snapshot_refresh_count = 0
        self._snapshot_coalesced_count = 0
        self._snapshot_last_duration_ms: float | None = None
        self._accept_due_at: float | None = None
        # LeagueAkari starts a real timeout task as soon as the LCU enters
        # ReadyCheck.  Keep the polling loop as a recovery path, but do not
        # make an auto-accept wait for the next (possibly slow) state refresh.
        self._auto_accept_waiter: asyncio.Task | None = None
        self._acted_phase = ""
        self._phase_action_done = ""
        self._phase_action_due_at: float | None = None
        self._handled_invitations: set[str] = set()
        self._handled_champion_actions: set[str] = set()
        self._champion_action_due_at: dict[str, float] = {}
        # Delayed auto-select work is kept as a read-only, JSON-safe plan.  It
        # mirrors LeagueAkari's delayed task state without making the status
        # endpoint responsible for (or capable of) executing an LCU write.
        self._delayed_action_plan: dict[str, dict] = {}
        self._handled_trades: set[str] = set()
        # LeagueAkari records when an incoming champion swap first becomes
        # RECEIVED.  Keeping the monotonic timestamp here lets a later poll
        # subtract time that has already elapsed from the configured delay.
        self._trade_created_at: dict[str, float] = {}
        self._bench_candidate_since: dict[int, float] = {}
        self.auto_select_temporarily_disabled = False
        self._leader_handoff_lobby = ""
        self._configured_champion_id = 0
        self._active_auto_select_group = "default"
        self._active_auto_select_position = "default"
        self._active_auto_select_queue_type = ""
        self._honored_game_id = ""
        # A ballot can arrive through the event stream and be observed by the
        # polling fallback at the same time.  Keep the in-flight game id so
        # those two paths cannot submit two honor ballots concurrently.
        self._honor_in_progress_game_id = ""
        self._matchmaking_due_at: float | None = None
        self._matchmaking_status = "idle"
        self._matchmaking_status_reason: str | None = None
        self._matchmaking_chat_countdown_second: int | None = None
        self._matchmaking_last_chat_key: str | None = None
        self._last_event_at = 0.0
        self._aram_side_sent_context = ""
        self._chat_ready_since: float | None = None
        self._chat_ready_automation_done = False
        self.champ_select: dict = {}
        # Read-only lifecycle snapshots used by the status surface.  These are
        # intentionally kept separate from the action scheduler so that a
        # missing optional LCU endpoint cannot create or trigger an account
        # write.
        self.ready_check: dict | None = None
        self.matchmaking_search: dict | None = None
        self.ongoing_champion_swap: dict | None = None
        self.respawn_timer: dict = {"available": False, "dead": False, "time_left": 0.0, "total_time": 0.0}
        self._last_discovery_at = 0.0
        self._selected_client_pid = 0
        self._available_clients: list[LcuCredentials] = []

    @staticmethod
    def _settings_path() -> Path:
        return get_data_dir() / "league-lab.json"

    def _load_settings(self) -> LeagueLabSettings:
        try:
            raw = json.loads(self._settings_path().read_text(encoding="utf-8"))
            stored_migration_version = int(raw.get("safety_migration_version", 0) or 0)
            settings = LeagueLabSettings.model_validate(raw)
        except (OSError, ValueError):
            return LeagueLabSettings()
        if stored_migration_version < 1:
            settings = self._apply_account_action_safety_migration(settings)
            self._write_settings(settings)
        elif stored_migration_version < 2:
            # Preserve every automation the user explicitly opted into after
            # the original safety migration; only make the manual account
            # toolkit available by default in this product revision.
            settings = settings.model_copy(update={
                "toolkit_account_actions_enabled": True,
                "safety_migration_version": 2,
            })
            self._write_settings(settings)
        return settings

    @staticmethod
    def _apply_account_action_safety_migration(settings: LeagueLabSettings) -> LeagueLabSettings:
        disabled = {
            "automation_enabled": False,
            "auto_accept_enabled": False,
            "play_again_enabled": False,
            "auto_reconnect_enabled": False,
            "auto_handle_invitations_enabled": False,
            "auto_skip_leader_enabled": False,
            "auto_select_enabled": False,
            "auto_champion_config_enabled": False,
            "auto_honor_enabled": False,
            "auto_matchmaking_enabled": False,
            "auto_matchmaking_chat_countdown_enabled": False,
            "auto_reply_enabled": False,
            "lock_offline_status": False,
            "auto_set_status_message_enabled": False,
            "auto_set_ranked_status_enabled": False,
            "auto_send_aram_team_side_enabled": False,
            # The product now treats the account-write toolkit as an available
            # capability by default. Individual automations remain disabled
            # and still require their own explicit switches.
            "toolkit_account_actions_enabled": True,
            "terminate_game_shortcut_enabled": False,
            "in_game_send_enabled": False,
            "safety_migration_version": 2,
        }
        profiles = {}
        for key, profile in settings.auto_select_profiles.items():
            profile_body = profile.model_dump()
            profile_body["pick"]["enabled"] = False
            profile_body["pick"]["bench_handle_trade_enabled"] = False
            profile_body["ban"]["enabled"] = False
            profiles[key] = AutoSelectProfile.model_validate(profile_body)
        return settings.model_copy(update={**disabled, "auto_select_profiles": profiles})

    @classmethod
    def _write_settings(cls, settings: LeagueLabSettings) -> None:
        path = cls._settings_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        temp = path.with_suffix(".tmp")
        temp.write_text(json.dumps(settings.model_dump(), ensure_ascii=False, indent=2), encoding="utf-8")
        temp.replace(path)

    def update_settings(self, settings: LeagueLabSettings) -> LeagueLabSettings:
        self._write_settings(settings)
        self.settings = settings
        self._invalidate_snapshot_cache()
        if not settings.automation_enabled or not settings.auto_accept_enabled:
            self._cancel_auto_accept_waiter()
            self._accept_due_at = None
        if not settings.auto_select_enabled:
            self._handled_champion_actions.clear()
            self._champion_action_due_at.clear()
            self._delayed_action_plan.clear()
            self._configured_champion_id = 0
            self._active_auto_select_group = "default"
            self._active_auto_select_position = "default"
            self._active_auto_select_queue_type = ""
            self.champ_select = {}
        return settings

    async def start(self) -> None:
        if self._task is None or self._task.done():
            self._task = asyncio.create_task(self._run(), name="league-lab")

    async def stop(self) -> None:
        self._cancel_auto_accept_waiter()
        if self._task is not None:
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
            self._task = None
        self._terminate_dodge_loop("service-stopped")
        if self._event_task:
            self._event_task.cancel()
            try:
                await self._event_task
            except asyncio.CancelledError:
                pass
        self._event_task = None

    def _cancel_auto_accept_waiter(self) -> None:
        waiter = self._auto_accept_waiter
        self._auto_accept_waiter = None
        if waiter is not None and not waiter.done():
            waiter.cancel()

    def _invalidate_snapshot_cache(self) -> None:
        """Invalidate the read-only status refresh marker.

        The generation protects an in-flight refresh from publishing a cache
        timestamp after an LCU event/write changed the underlying state.
        This stores no response body, token, player data, or other sensitive
        material.
        """
        self._snapshot_cache_at = 0.0
        self._snapshot_generation += 1

    def _start_auto_accept_waiter(self, delay: float) -> None:
        """Start a ReadyCheck timer immediately after the LCU event arrives.

        The regular loop still verifies the phase and deadline, so this is
        safe when the event stream drops.  A single sentinel assignment in
        ``_try_auto_accept`` prevents the event task and polling loop from
        submitting the same accept request concurrently.
        """
        self._cancel_auto_accept_waiter()
        self._auto_accept_waiter = asyncio.create_task(
            self._wait_and_auto_accept(
                delay,
                expected_generation=self._auto_accept_round_generation,
                expected_credentials=self._auto_accept_credentials,
            ),
            name="league-auto-accept",
        )

    def _ready_check_can_accept(self) -> bool:
        """Require the authoritative gameflow phase before an account write."""
        return self.phase == "ReadyCheck"

    def _reset_auto_accept_round(self, phase: str) -> None:
        """Reset per-ballot state only after authoritative round evidence."""
        self._auto_accept_round_generation += 1
        self._auto_accept_observed_phase = phase
        self._auto_accept_round_identity = None
        self._auto_accept_submitted = False
        self._auto_accept_terminal_response = False
        self._auto_accept_credentials = None
        self._accept_due_at = None
        self._cancel_auto_accept_waiter()

    def _observe_auto_accept_phase(self, phase: str) -> None:
        if phase != self._auto_accept_observed_phase:
            self._reset_auto_accept_round(phase)

    def _observe_ready_check_identity(self, ready_check: dict | None) -> None:
        if self.phase != "ReadyCheck" or not isinstance(ready_check, dict):
            return
        identity = str(ready_check.get("round_id") or "").strip() or None
        if identity is None:
            return
        if (
            self._auto_accept_round_identity is not None
            and identity != self._auto_accept_round_identity
        ):
            self._reset_auto_accept_round("ReadyCheck")
        self._auto_accept_round_identity = identity

    async def _ensure_auto_accept_scheduled(
        self,
        expected_event_sequence: int | None = None,
    ) -> None:
        """Create one deadline per ReadyCheck without restarting it on events."""
        if (
            (
                expected_event_sequence is not None
                and self._event_sequence != expected_event_sequence
            )
            or not self.settings.automation_enabled
            or not self.settings.auto_accept_enabled
            or not self._ready_check_can_accept()
            or self._auto_accept_terminal_response
            or self._auto_accept_submitted
        ):
            return

        if self._accept_due_at is None:
            delay = max(0.0, float(self.settings.auto_accept_delay_seconds))
            self._accept_due_at = time.monotonic() + delay
            self._auto_accept_credentials = self.credentials
            self._start_auto_accept_waiter(delay)

        if self._accept_due_at == float("inf"):
            return

        remaining = self._accept_due_at - time.monotonic()
        if remaining <= 0:
            if self._auto_accept_waiter is None or self._auto_accept_waiter.done():
                self._start_auto_accept_waiter(0.0)
            await asyncio.sleep(0)
        elif self._auto_accept_waiter is None or self._auto_accept_waiter.done():
            self._start_auto_accept_waiter(remaining)

    async def _wait_and_auto_accept(
        self,
        delay: float,
        *,
        expected_generation: int | None = None,
        expected_credentials: LcuCredentials | None = None,
    ) -> None:
        try:
            # ``asyncio.sleep`` may resume a fraction before the monotonic
            # deadline on Windows.  A one-shot sleep would then make
            # ``_try_auto_accept`` reject the action and clear the waiter,
            # leaving acceptance to the much slower polling fallback.  Wait
            # against the authoritative deadline until it is truly due.
            while True:
                due_at = self._accept_due_at
                if due_at is None or due_at == float("inf"):
                    return
                remaining = due_at - time.monotonic()
                if remaining <= 0:
                    break
                await asyncio.sleep(remaining)
            await self._try_auto_accept(
                expected_generation=expected_generation,
                expected_credentials=expected_credentials,
            )
        except asyncio.CancelledError:
            raise
        finally:
            if self._auto_accept_waiter is asyncio.current_task():
                self._auto_accept_waiter = None

    async def _try_auto_accept(
        self,
        *,
        expected_generation: int | None = None,
        expected_credentials: LcuCredentials | None = None,
    ) -> None:
        """Accept once when the event-driven ReadyCheck deadline is due."""
        if (
            not self._ready_check_can_accept()
            or not self.settings.automation_enabled
            or not self.settings.auto_accept_enabled
            or self._auto_accept_submitted
            or (
                expected_generation is not None
                and expected_generation != self._auto_accept_round_generation
            )
            or self._accept_due_at is None
            or self._accept_due_at == float("inf")
            or time.monotonic() < self._accept_due_at
        ):
            return
        credentials = (
            expected_credentials
            if expected_generation is not None
            else self._auto_accept_credentials
        )
        if credentials is not None and self.credentials != credentials:
            return
        ready_check = self.ready_check or {}
        response = str(
            ready_check.get("player_response") or ready_check.get("playerResponse") or ""
        ).upper()
        if response in {"ACCEPTED", "DECLINED"}:
            self._auto_accept_terminal_response = True
            self._accept_due_at = None
            return
        # Mark submitted before awaiting the LCU request. Settings toggles,
        # polling, duplicate events and uncertain timeouts must never submit
        # the same ReadyCheck twice.
        self._auto_accept_submitted = True
        self._accept_due_at = float("inf")
        try:
            if credentials is None:
                await self._record_action(
                    "已自动接受对局",
                    "POST",
                    "/lol-matchmaking/v1/ready-check/accept",
                )
            else:
                await self._record_action(
                    "已自动接受对局",
                    "POST",
                    "/lol-matchmaking/v1/ready-check/accept",
                    credentials=credentials,
                )
        except RuntimeError:
            # The request may have reached LCU even when its response timed
            # out.  Keep the per-ReadyCheck sentinel to guarantee at-most-once
            # submission; the next phase transition resets it.
            logger.info("League auto-accept request failed or timed out; not retrying this ReadyCheck")

    async def refresh_connection(self, *, force: bool = False) -> bool:
        now = time.monotonic()
        if not force and self.credentials and now - self._last_discovery_at < 5.0:
            return True
        if not force and now - self._last_discovery_at < 5.0:
            return False
        self._last_discovery_at = now
        clients = await discover_lcu_clients()
        self._available_clients = clients
        credentials = next((client for client in clients if client.pid == self._selected_client_pid), None)
        if credentials is None:
            credentials = clients[0] if clients else None
            self._selected_client_pid = credentials.pid if credentials else 0
        self._replace_credentials(credentials)
        return credentials is not None

    def _replace_credentials(self, credentials: LcuCredentials | None) -> None:
        if credentials != self.credentials:
            self._invalidate_snapshot_cache()
            self._terminate_dodge_loop("client-disconnected")
            if self._event_task:
                self._event_task.cancel()
                self._event_task = None
            self.credentials = credentials
            self._event_connected = False
            self.phase = ""
            self.game_mode = ""
            self.gameflow_session = {}
            self.summoner_name = ""
            self.current_summoner = {}
            self._acted_phase = ""
            self._phase_action_done = ""
            self._phase_action_due_at = None
            self._reset_auto_accept_round("")
            self._champion_action_due_at.clear()
            self._delayed_action_plan.clear()
            self._handled_champion_actions.clear()
            self._handled_trades.clear()
            self._trade_created_at.clear()
            self._bench_candidate_since.clear()
            self._active_auto_select_group = "default"
            self._active_auto_select_position = "default"
            self._active_auto_select_queue_type = ""
            self._honor_in_progress_game_id = ""
            self._matchmaking_due_at = None
            self._matchmaking_status = "idle"
            self._matchmaking_status_reason = None
            self._matchmaking_last_chat_key = None
            self._cancel_auto_accept_waiter()
            self.ready_check = None
            self.matchmaking_search = None
            self.ongoing_champion_swap = None
            if self.phase != "ChampSelect":
                self.champ_select = {}
                self.auto_select_temporarily_disabled = False
                self._active_auto_select_group = "default"
                self._active_auto_select_position = "default"
                self._active_auto_select_queue_type = ""
            self._reset_chat_ready_automation()
            if credentials:
                self._event_task = asyncio.create_task(self._run_event_stream(credentials), name="league-lcu-events")

    def _invalidate_connection(self) -> None:
        """Drop both credentials and cached gameflow state after an auth/IO failure.

        LeagueAkari clears its gameflow snapshot when the LCU connection is
        lost.  Keeping a stale Lobby/ChampSelect phase here would otherwise
        make the desktop lifecycle reopen auxiliary windows after disconnect.
        """
        if self.credentials is not None:
            self._replace_credentials(None)
            return
        # The caller may already have lost the credential object.  Recreate a
        # harmless sentinel so the canonical replacement path still performs
        # every lifecycle reset instead of maintaining a second reset list.
        self.credentials = LcuCredentials(port=0, token="")
        self._replace_credentials(None)

    async def select_client(self, pid: int) -> None:
        clients = await discover_lcu_clients()
        credentials = next((client for client in clients if client.pid == pid), None)
        if credentials is None:
            raise RuntimeError("所选 LeagueClientUx 已退出或无法读取认证信息")
        self._available_clients = clients
        self._selected_client_pid = pid
        self._last_discovery_at = time.monotonic()
        self._replace_credentials(credentials)
        await self._refresh_state()

    def _reset_chat_ready_automation(self) -> None:
        self._chat_ready_since = None
        self._chat_ready_automation_done = False

    def _interrupt_chat_ready_automation(self) -> None:
        self._chat_ready_since = None
        self._chat_ready_automation_done = True

    def _terminate_dodge_loop(self, reason: str) -> None:
        """Stop the local dodge-loop task without issuing an LCU request.

        This helper is intentionally synchronous so it can be used from
        connection/phase reset paths.  The loop task itself is cancelled as a
        best-effort local operation; every worker also observes the event and
        re-checks the account gate before its next write.
        """

        event = self._dodge_loop_cancel_event
        if event is not None:
            event.set()
        task = self._dodge_loop_task
        try:
            current = asyncio.current_task()
        except RuntimeError:
            current = None
        if task is not None and task is not current and not task.done():
            task.cancel()
        if self._dodge_loop_state.get("active") or task is not None:
            self._dodge_loop_state.update({
                "active": False,
                "stopped_at": time.time(),
                "stop_reason": reason,
            })

    async def _dodge_once_with_revalidation(self) -> tuple[bool, str | None]:
        """Perform one guarded dodge write, or return a terminal reason.

        The phase and account-write gate are checked immediately before every
        invoke request.  A worker never relies on the cached service phase.
        """

        if not self.settings.toolkit_account_actions_enabled:
            return False, "account-actions-disabled"
        try:
            phase = str(await self.request("GET", "/lol-gameflow/v1/gameflow-phase") or "")
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # pragma: no cover - concrete transport errors vary by client
            return False, f"phase-check-failed: {type(exc).__name__}"
        if phase != "ChampSelect":
            return False, "phase-exited"
        if not self.settings.toolkit_account_actions_enabled:
            return False, "account-actions-disabled"
        try:
            await self.request(
                "POST",
                "/lol-login/v1/session/invoke",
                params={
                    "destination": "lcdsServiceProxy",
                    "method": "call",
                    "args": '["", "teambuilder-draft", "quitV2", ""]',
                },
                json_body={"data": ["", "teambuilder-draft", "quitV2", ""]},
            )
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # pragma: no cover - concrete transport errors vary by client
            return False, f"write-failed: {type(exc).__name__}"
        return True, None

    async def _run_dodge_loop_worker(self, cancel_event: asyncio.Event) -> str:
        while not cancel_event.is_set():
            ok, reason = await self._dodge_once_with_revalidation()
            if ok:
                self._dodge_loop_state["attempts"] = int(self._dodge_loop_state.get("attempts") or 0) + 1
            elif reason in {"account-actions-disabled", "phase-exited"}:
                cancel_event.set()
                return reason
            else:
                self._dodge_loop_state["last_error"] = reason
            # Keep five workers responsive without turning an LCU error into a
            # tight CPU/network loop.  The upstream loop remains concurrent;
            # this small yield is only a local safety backoff.
            await asyncio.sleep(0.05)
        return "cancelled"

    async def _run_dodge_loop(self, cancel_event: asyncio.Event) -> None:
        workers = [
            asyncio.create_task(self._run_dodge_loop_worker(cancel_event), name=f"league-dodge-worker-{index}")
            for index in range(5)
        ]
        try:
            results = await asyncio.gather(*workers, return_exceptions=True)
            terminal = next(
                (result for result in results if isinstance(result, str) and result != "cancelled"),
                None,
            )
            if terminal and self._dodge_loop_state.get("stop_reason") is None:
                self._dodge_loop_state["stop_reason"] = terminal
        except asyncio.CancelledError:
            cancel_event.set()
            for worker in workers:
                worker.cancel()
            await asyncio.gather(*workers, return_exceptions=True)
            raise
        finally:
            self._dodge_loop_state["active"] = False
            self._dodge_loop_state["stopped_at"] = self._dodge_loop_state.get("stopped_at") or time.time()
            self._dodge_loop_state["stop_reason"] = self._dodge_loop_state.get("stop_reason") or "completed"
            if self._dodge_loop_task is asyncio.current_task():
                self._dodge_loop_task = None
                self._dodge_loop_cancel_event = None

    def start_dodge_loop(self) -> None:
        if self._dodge_loop_task is not None and not self._dodge_loop_task.done():
            raise RuntimeError("秒退循环已经在运行")
        cancel_event = asyncio.Event()
        self._dodge_loop_cancel_event = cancel_event
        self._dodge_loop_state = {
            "active": True,
            "attempts": 0,
            "concurrency": 5,
            "started_at": time.time(),
            "stopped_at": None,
            "stop_reason": None,
            "last_error": None,
        }
        self._dodge_loop_task = asyncio.create_task(
            self._run_dodge_loop(cancel_event),
            name="league-dodge-loop",
        )

    async def _prime_event_stream_state(
        self,
        credentials: LcuCredentials,
        expected_event_sequence: int | None = None,
    ) -> None:
        """Read the two time-sensitive states after subscribing.

        WebSocket events emitted during the handshake are not replayed by LCU.
        A concurrent read closes that gap without waiting for the full polling
        refresh (which also loads several unrelated endpoints).
        """
        headers = {"Authorization": credentials.auth_header}
        try:
            async with httpx.AsyncClient(verify=False, timeout=1.5) as client:
                phase_result, ready_result = await asyncio.gather(
                    client.get(f"{credentials.base_url}/lol-gameflow/v1/gameflow-phase", headers=headers),
                    client.get(f"{credentials.base_url}/lol-matchmaking/v1/ready-check", headers=headers),
                    return_exceptions=True,
                )
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            logger.debug("Unable to prime League LCU event state: %s", type(exc).__name__)
            return

        if (
            expected_event_sequence is not None
            and self._event_sequence != expected_event_sequence
        ):
            # A live event arrived while these GETs were in flight.  The event
            # is newer than this handshake snapshot, so never overwrite it.
            return

        ready_check = None
        if isinstance(ready_result, httpx.Response) and ready_result.is_success:
            try:
                ready_payload = ready_result.json()
            except ValueError:
                ready_payload = None
            ready_check = _normalize_ready_check(ready_payload)

        phase = None
        if isinstance(phase_result, httpx.Response) and phase_result.is_success:
            try:
                phase = phase_result.json()
            except ValueError:
                phase = None

        if (
            expected_event_sequence is not None
            and self._event_sequence != expected_event_sequence
        ):
            return
        if self.credentials != credentials:
            return

        # Apply the handshake snapshot without awaiting or invoking the normal
        # event handler.  This makes phase + response one atomic read-only state
        # update relative to newer WebSocket events.
        self._invalidate_snapshot_cache()
        if not isinstance(phase, str):
            return

        self.phase = phase
        self._observe_auto_accept_phase(phase)
        self._acted_phase = phase
        self._phase_action_done = ""
        incoming_round_identity = (
            str((ready_check or {}).get("round_id") or "").strip() or None
        )
        if (
            phase == "ReadyCheck"
            and ready_check is not None
            and self._auto_accept_terminal_response
            and (
                incoming_round_identity is None
                or self._auto_accept_round_identity is None
                or incoming_round_identity != self._auto_accept_round_identity
            )
            and str(ready_check.get("player_response") or "").upper()
            not in {"ACCEPTED", "DECLINED"}
        ):
            # A reconnect prime is an authoritative GET, not a delayed event.
            # If it observes a pending ballot after this process already saw a
            # terminal response, the intervening phase edge was missed while
            # the WebSocket was down. Start a new local generation. Ordinary
            # late Pending events never take this path and remain latched.
            self._reset_auto_accept_round("ReadyCheck")
        if ready_check is not None:
            self._observe_ready_check_identity(ready_check)
            self.ready_check = ready_check
        response = str((self.ready_check or {}).get("player_response") or "").upper()
        if phase != "ReadyCheck":
            self.ready_check = None
            return
        if response in {"ACCEPTED", "DECLINED"}:
            self._auto_accept_terminal_response = True
            self._accept_due_at = None
            self._cancel_auto_accept_waiter()
            return
        await self._ensure_auto_accept_scheduled(expected_event_sequence)

    async def _run_event_stream(self, credentials: LcuCredentials) -> None:
        """Keep one LCU event subscription alive; polling remains fallback."""
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        retry_count = 0

        while self.credentials == credentials:
            try:
                async with websockets.connect(
                    f"wss://127.0.0.1:{credentials.port}",
                    additional_headers={"Authorization": credentials.auth_header},
                    ssl=context,
                    open_timeout=3,
                    close_timeout=1,
                    ping_interval=20,
                ) as socket:
                    self._event_connected = True
                    retry_count = 0
                    await socket.send(json.dumps([5, "OnJsonApiEvent"]))
                    prime_task = asyncio.create_task(
                        self._prime_event_stream_state(credentials, self._event_sequence),
                        name="league-lcu-event-prime",
                    )
                    try:
                        async for raw in socket:
                            try:
                                event = json.loads(raw)
                            except (TypeError, ValueError):
                                continue
                            if isinstance(event, list) and len(event) >= 3 and event[0] == 8:
                                self._event_sequence += 1
                                self._last_event_at = time.time()
                                await self._handle_lcu_event(event[2] if isinstance(event[2], dict) else {})
                                self._event_wakeup.set()
                    finally:
                        if not prime_task.done():
                            prime_task.cancel()
                        await asyncio.gather(prime_task, return_exceptions=True)
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                log = logger.info if retry_count == 0 else logger.debug
                log(
                    "League LCU event stream disconnected; polling fallback remains active: %s",
                    type(exc).__name__,
                )
            finally:
                if self._event_task is asyncio.current_task():
                    self._event_connected = False

            if self.credentials != credentials:
                return
            retry_count += 1
            retry_delay = min(2.0, 0.25 * (2 ** min(retry_count - 1, 3)))
            await asyncio.sleep(retry_delay)

    async def _handle_lcu_event(self, event: dict) -> None:
        self._invalidate_snapshot_cache()
        uri = str(event.get("uri") or "")
        data = event.get("data") or {}
        if uri == "/lol-gameflow/v1/gameflow-phase":
            # LeagueAkari reacts to the gameflow observable immediately.  Do
            # the same here so ReadyCheck does not wait for a full state
            # refresh before its zero-delay action can run.
            next_phase = str(data or "")
            self.phase = next_phase
            self._observe_auto_accept_phase(next_phase)
            self._acted_phase = next_phase
            self._phase_action_done = ""
            if next_phase != "ReadyCheck":
                self.ready_check = None
            else:
                if self.settings.automation_enabled and self.settings.auto_accept_enabled:
                    await self._ensure_auto_accept_scheduled()

        if uri == "/lol-matchmaking/v1/ready-check":
            normalized = _normalize_ready_check(data)
            if normalized is not None:
                self._observe_ready_check_identity(normalized)
                self.ready_check = normalized
            response = str(
                (normalized or {}).get("player_response")
                or (normalized or {}).get("playerResponse")
                or ""
            ).upper()
            if response in {"ACCEPTED", "DECLINED"}:
                self._auto_accept_terminal_response = True
                self._accept_due_at = None
                self._cancel_auto_accept_waiter()
            elif self.settings.automation_enabled and self.settings.auto_accept_enabled:
                await self._ensure_auto_accept_scheduled()
        # LeagueAkari completes the pre-end mission celebration before its
        # play-again timer.  Without this event, clients that remain in
        # PreEndOfGame can wait on a ballot that never becomes actionable.
        if (
            uri == "/lol-pre-end-of-game/v1/currentSequenceEvent"
            and self.settings.automation_enabled
            and self.settings.play_again_enabled
            and isinstance(data, dict)
            and data.get("name") == "missions-celebration"
        ):
            try:
                await self.request(
                    "POST",
                    "/lol-pre-end-of-game/v1/complete/missions-celebration",
                )
            except RuntimeError:
                logger.debug("Unable to complete pre-end missions celebration", exc_info=True)
        # LeagueAkari reacts to the honor ballot event instead of relying on a
        # phase poll.  This matters because some client builds expose the
        # ballot only briefly while transitioning through WaitingForStats.
        # The handler remains opt-in and _run_auto_honor still validates the
        # phase/settings immediately before every account write.
        if (
            uri == "/lol-honor-v2/v1/ballot"
            and isinstance(data, dict)
            and data.get("gameId")
            and self.settings.automation_enabled
            and self.settings.auto_honor_enabled
        ):
            await self._run_auto_honor(data)
        message_match = re.fullmatch(r"/lol-chat/v1/conversations/([^/]+)/messages/([^/]+)", uri)
        if (
            message_match
            and self.settings.automation_enabled
            and self.settings.auto_reply_enabled
            and self.settings.auto_reply_text
        ):
            own_id = self.current_summoner.get("summoner_id")
            if (
                event.get("eventType") in {"Create", "Update"}
                and data.get("type") == "chat"
                and data.get("fromSummonerId") != own_id
                and not data.get("isHistorical")
            ):
                if self.settings.auto_reply_only_away:
                    try:
                        chat_me = await self.request("GET", "/lol-chat/v1/me")
                    except RuntimeError:
                        return
                    if not isinstance(chat_me, dict) or chat_me.get("availability") != "away":
                        return
                conversation_id = message_match.group(1)
                await self.request(
                    "POST",
                    f"/lol-chat/v1/conversations/{conversation_id}/messages",
                    json_body={"body": self.settings.auto_reply_text, "type": "chat"},
                )
                self.last_action = "已自动回复一条私聊"
                self.last_action_at = time.time()

        if uri == "/lol-chat/v1/me" and self.settings.automation_enabled and self.settings.lock_offline_status:
            availability = str(data.get("availability") or "")
            if availability in {"away", "chat", "online"}:
                await self.request("PUT", "/lol-chat/v1/me", json_body={"availability": "offline"})

        friend_match = re.fullmatch(r"/lol-chat/v1/friends/([^/]+)", uri)
        if friend_match and self.settings.automation_enabled and self.settings.auto_invite_friend_puuids:
            puuid = str(data.get("puuid") or "")
            if puuid in self.settings.auto_invite_friend_puuids and data.get("availability") == "chat":
                try:
                    lobby = await self.request("GET", "/lol-lobby/v2/lobby")
                except RuntimeError:
                    return
                if not isinstance(lobby, dict):
                    return
                local_member = lobby.get("localMember")
                if not isinstance(local_member, dict) or local_member.get("allowedInviteOthers") is not True:
                    return
                members = (lobby or {}).get("members") or []
                if any(str(member.get("puuid") or "") == puuid for member in members):
                    return
                summoner_id = data.get("summonerId")
                if summoner_id:
                    await self.request(
                        "POST",
                        "/lol-lobby/v2/lobby/invitations",
                        json_body=[{"toSummonerId": summoner_id}],
                    )
                    self.settings.auto_invite_friend_puuids = [
                        value for value in self.settings.auto_invite_friend_puuids if value != puuid
                    ]
                    self.update_settings(self.settings)
                    self.last_action = f"已自动邀请好友 {data.get('gameName') or puuid}"
                    self.last_action_at = time.time()

    async def request(
        self,
        method: str,
        path: str,
        *,
        json_body=None,
        params=None,
        credentials: LcuCredentials | None = None,
    ):
        if credentials is None:
            if not await self.refresh_connection():
                raise RuntimeError("未检测到正在运行的英雄联盟客户端")
            credentials = self.credentials
        elif self.credentials != credentials:
            raise RuntimeError("英雄联盟客户端已切换，已放弃过期自动操作")
        if credentials is None:
            raise RuntimeError("未检测到正在运行的英雄联盟客户端")
        try:
            async with httpx.AsyncClient(verify=False, timeout=3.0) as client:
                response = await client.request(
                    method,
                    f"{credentials.base_url}{path}",
                    headers={"Authorization": credentials.auth_header},
                    json=json_body,
                    params=params,
                )
            response.raise_for_status()
            if str(method).upper() != "GET":
                # A successful account/state write can invalidate the status
                # marker even when the endpoint returns an empty 204 body.
                self._invalidate_snapshot_cache()
            if response.status_code == 204 or not response.content:
                return None
            return response.json()
        except httpx.HTTPStatusError as exc:
            self.last_error = f"LCU 请求失败: {type(exc).__name__}"
            # Optional routes can legitimately be absent on some client builds.
            # A 404 does not invalidate the in-memory LCU credentials.
            if exc.response.status_code in {401, 403}:
                self._invalidate_connection()
            raise RuntimeError(self.last_error) from exc
        except httpx.RequestError as exc:
            self.last_error = f"LCU 请求失败: {type(exc).__name__}"
            self._invalidate_connection()
            raise RuntimeError(self.last_error) from exc
        except ValueError as exc:
            self.last_error = f"LCU 请求失败: {type(exc).__name__}"
            raise RuntimeError(self.last_error) from exc

    async def request_bytes(self, path: str) -> tuple[bytes, str]:
        if not await self.refresh_connection():
            raise RuntimeError("未检测到正在运行的英雄联盟客户端")
        assert self.credentials is not None
        try:
            async with httpx.AsyncClient(verify=False, timeout=3.0) as client:
                response = await client.get(
                    f"{self.credentials.base_url}{path}",
                    headers={"Authorization": self.credentials.auth_header},
                )
            response.raise_for_status()
            return response.content, response.headers.get("content-type", "image/png")
        except httpx.HTTPError as exc:
            self.last_error = f"LCU 资源请求失败: {type(exc).__name__}"
            raise RuntimeError(self.last_error) from exc

    async def riot_request(self, method: str, path: str, *, json_body=None, params=None):
        """Call the local Riot Client API without exposing its credentials outside this process."""
        if not await self.refresh_connection():
            raise RuntimeError("未检测到正在运行的英雄联盟客户端")
        credentials = self.credentials
        if not credentials or not credentials.riot_client_port or not credentials.riot_client_token:
            raise RuntimeError("英雄联盟客户端未提供 Riot Client 本地接口")
        try:
            async with httpx.AsyncClient(verify=False, timeout=5.0) as client:
                response = await client.request(
                    method,
                    f"{credentials.riot_client_base_url}{path}",
                    headers={"Authorization": credentials.riot_client_auth_header},
                    json=json_body,
                    params=params,
                )
            response.raise_for_status()
            if response.status_code == 204 or not response.content:
                return None
            return response.json()
        except (httpx.HTTPError, ValueError) as exc:
            raise RuntimeError(f"Riot Client 请求失败: {type(exc).__name__}") from exc

    async def snapshot(self) -> dict:
        now = time.monotonic()
        if self._snapshot_cache_at > 0 and now - self._snapshot_cache_at < self._SNAPSHOT_CACHE_TTL_SECONDS:
            return self.status()

        if self._snapshot_refresh_lock.locked():
            self._snapshot_coalesced_count += 1
        async with self._snapshot_refresh_lock:
            # Another caller may have completed the refresh while we waited
            # for the single-flight lock.
            now = time.monotonic()
            if self._snapshot_cache_at <= 0 or now - self._snapshot_cache_at >= self._SNAPSHOT_CACHE_TTL_SECONDS:
                started = time.perf_counter()
                self._snapshot_refresh_count += 1
                completed = False
                generation = self._snapshot_generation
                try:
                    connected = await self.refresh_connection(force=True)
                    # Client discovery can itself replace credentials.  That
                    # reset is part of this refresh, so use the post-discovery
                    # generation as the consistency point for the LCU reads.
                    generation = self._snapshot_generation
                    if connected:
                        await self._refresh_state()
                    completed = True
                finally:
                    self._snapshot_last_duration_ms = round(
                        max(0.0, (time.perf_counter() - started) * 1000.0),
                        2,
                    )
                    if completed and generation == self._snapshot_generation:
                        self._snapshot_cache_at = time.monotonic()
                    else:
                        self._snapshot_cache_at = 0.0
        return self.status()

    def status(self) -> dict:
        credentials = self.credentials
        client_window_detected = _league_client_window_is_present()
        now_mono = time.monotonic()
        snapshot_age_ms = (
            round(max(0.0, now_mono - self._snapshot_cache_at) * 1000.0, 2)
            if self._snapshot_cache_at > 0
            else None
        )

        def public_due(kind: str, label: str, due_at: float | None, *, action_id: str | None = None) -> dict | None:
            """Expose a monotonic scheduler deadline without leaking its clock."""
            if due_at is None or due_at == float("inf"):
                return None
            remaining = max(0.0, due_at - now_mono)
            payload = {
                "kind": kind,
                "label": label,
                "due_at": time.time() + remaining,
                "remaining_seconds": round(remaining, 2),
            }
            if action_id:
                payload["action_id"] = action_id
            return payload

        countdowns: list[tuple[str, str, float]] = []
        if self._accept_due_at is not None and self._accept_due_at != float("inf"):
            countdowns.append(("ready-check", "自动接受对局", self._accept_due_at))
        if self._champion_action_due_at:
            countdowns.append(("champion-action", "自动选择 / 禁用英雄", min(self._champion_action_due_at.values())))
        phase_action_enabled = (
            (self.phase == "Reconnect" and self.settings.auto_reconnect_enabled)
            or (self.phase in {"EndOfGame", "WaitingForStats", "PreEndOfGame"} and self.settings.play_again_enabled)
        )
        if self._phase_action_due_at is not None and not self._phase_action_done and phase_action_enabled:
            label = "自动重连" if self.phase == "Reconnect" else "自动返回房间"
            countdowns.append(("phase-action", label, self._phase_action_due_at))
        if self._matchmaking_due_at is not None:
            countdowns.append(("matchmaking", "自动开始匹配", self._matchmaking_due_at))
        next_countdown = min(countdowns, key=lambda item: item[2]) if countdowns else None
        action_countdown = None
        if next_countdown:
            kind, label, due_at = next_countdown
            action_countdown = {
                "kind": kind,
                "label": label,
                "due_at": time.time() + max(0.0, due_at - now_mono),
                "remaining_seconds": round(max(0.0, due_at - now_mono), 2),
            }

        phase_due = None
        if self._phase_action_due_at is not None and not self._phase_action_done and phase_action_enabled:
            phase_label = "自动重连" if self.phase == "Reconnect" else "自动返回房间"
            phase_due = public_due("phase-action", phase_label, self._phase_action_due_at)
        champion_due = [
            item
            for action_id, due_at in sorted(self._champion_action_due_at.items())
            if (item := public_due("champion-action", "自动选择 / 禁用英雄", due_at, action_id=action_id)) is not None
        ]
        for item in champion_due:
            metadata = self._delayed_action_plan.get(str(item.get("action_id")))
            if metadata:
                item.update({key: value for key, value in metadata.items() if key not in {"due_at", "remaining_seconds"}})
                item["kind"] = "champion-action"
                item["label"] = "自动选择 / 禁用英雄"
        # Preserve the original ``champion_due`` shape while enriching it with
        # LeagueAkari-style move/action metadata.  Bench swaps and champion
        # trades live in the same public delayed plan so the UI can explain a
        # pending operation without guessing from the countdown label.
        delayed_action_plan: list[dict] = []
        for action_id, due_at in sorted(self._champion_action_due_at.items()):
            entry = dict(self._delayed_action_plan.get(action_id) or {})
            entry.setdefault("kind", "champion-action")
            entry.setdefault("label", "自动选择 / 禁用英雄")
            entry.setdefault("action_id", action_id)
            entry["due_at"] = time.time() + max(0.0, due_at - now_mono)
            entry["remaining_seconds"] = round(max(0.0, due_at - now_mono), 2)
            delayed_action_plan.append(entry)
        for action_id, entry in sorted(self._delayed_action_plan.items()):
            if action_id in self._champion_action_due_at:
                continue
            due_at = entry.get("due_at")
            if not isinstance(due_at, (int, float)):
                delayed_action_plan.append(dict(entry))
                continue
            public_entry = dict(entry)
            public_entry["due_at"] = time.time() + max(0.0, float(due_at) - now_mono)
            public_entry["remaining_seconds"] = round(max(0.0, float(due_at) - now_mono), 2)
            delayed_action_plan.append(public_entry)

        champ_state = self.champ_select if isinstance(self.champ_select, dict) else {}
        current_pickable = list(champ_state.get("current_pickable_champion_ids") or [])
        current_bannable = list(champ_state.get("current_bannable_champion_ids") or [])
        pickable_available = bool(champ_state.get("current_pickable_ids_available"))
        bannable_available = bool(champ_state.get("current_bannable_ids_available"))
        default_profile = self.settings.auto_select_profiles.get("default") or AutoSelectProfile()
        active_group_id = str(self._active_auto_select_group or "default")
        if active_group_id == "default" and self.game_mode:
            guessed_group = self._mode_group(
                {"gameData": {"queue": {"gameMode": self.game_mode, "type": self._active_auto_select_queue_type}}},
                champ_state,
            )
            if guessed_group != "default":
                active_group_id = guessed_group
        active_profile = self.settings.auto_select_profiles.get(active_group_id) or default_profile
        local_cell = champ_state.get("local_player_cell_id")
        local_member = next(
            (
                member
                for member in (champ_state.get("my_team") or [])
                if str(member.get("cell_id")) == str(local_cell)
            ),
            {},
        )
        position = str(self._active_auto_select_position or local_member.get("assigned_position") or "default")
        pick_candidates = (
            self._profile_candidates(active_profile.pick.champions, position)
            if active_profile.pick.enabled
            else list(self.settings.auto_pick_champion_ids)
        )
        ban_candidates = (
            self._profile_candidates(active_profile.ban.champions, position)
            if active_profile.ban.enabled
            else list(self.settings.auto_ban_champion_ids)
        )

        allow_subset = bool(champ_state.get("allow_subset_champion_picks"))
        subset_available = bool(champ_state.get("subset_champion_ids_available"))
        subset_set = set(champ_state.get("subset_champion_ids") or [])
        grid_available = bool(champ_state.get("grid_champions_available"))
        grid_champions = champ_state.get("grid_champions") or {}
        game_mode = str(self.game_mode or "").upper()

        def expected_rows(candidates: list[int], available: list[int], available_flag: bool, action_type: str) -> list[dict]:
            available_set = set(available)
            if not available_flag:
                return [{"id": int(champion_id), "status": "unknown"} for champion_id in candidates]
            rows = []
            for champion_id in candidates:
                champion_id = int(champion_id)
                # LeagueAkari treats the Cherry "Bravery" pseudo champion as
                # available even though it is intentionally absent from the
                # normal LCU pickable-id array.
                if action_type == "pick" and champion_id == -3 and game_mode == "CHERRY":
                    status = "pickable"
                else:
                    grid = grid_champions.get(champion_id) if isinstance(grid_champions, dict) else None
                    selection = (grid or {}).get("selection_status") or {}
                    if action_type == "pick":
                        # When grid data is available, distinguish ownership
                        # and selection conflicts exactly as LeagueAkari does.
                        # A pickable-id list alone is not enough to establish
                        # ownership/selection status.  LeagueAkari treats a
                        # missing grid row as unknown even when the optional
                        # LCU pickable endpoint succeeded.
                        if not grid_available or grid is None:
                            status = "unknown"
                        elif champion_id not in available_set:
                            status = "not-owned" if not bool(grid.get("owned")) else "unpickable"
                        elif selection.get("is_banned"):
                            status = "banned"
                        elif not bool(champ_state.get("allow_duplicate_picks")):
                            if selection.get("pick_intented") and not selection.get("pick_intented_by_me"):
                                status = "pick-intented"
                            elif selection.get("picked_by_other_or_banned") and not selection.get("selected_by_me"):
                                status = "picked"
                            elif allow_subset:
                                status = "subset-pickable" if subset_available and champion_id in subset_set else (
                                    "unknown" if not subset_available else "unpickable"
                                )
                            else:
                                status = "pickable"
                        elif allow_subset:
                            status = "subset-pickable" if subset_available and champion_id in subset_set else (
                                "unknown" if not subset_available else "unpickable"
                            )
                        else:
                            status = "pickable"
                    else:
                        # Empty-ban (-1) has no grid row and may still be
                        # bannable; other banned champions are not actionable.
                        # For all ordinary champions the grid is still the
                        # source of truth, so no row means unknown.
                        if not grid_available or grid is None:
                            status = "unknown"
                        elif champion_id not in available_set:
                            status = "unbannable"
                        elif selection.get("is_banned") and champion_id != -1:
                            status = "banned"
                        elif selection.get("pick_intented") and not selection.get("pick_intented_by_me"):
                            status = "pick-intented"
                        else:
                            status = "bannable"
                rows.append({"id": champion_id, "status": status})
            return rows

        subset = list(champ_state.get("subset_champion_ids") or [])
        bench = list(champ_state.get("bench_champions") or [])
        scoped_bench = subset if subset and champ_state.get("timer_phase") == "BAN_PICK" else bench
        pickable_set = set(current_pickable)
        expected_swaps = []
        if champ_state.get("bench_enabled"):
            for champion_id in pick_candidates:
                champion_id = int(champion_id)
                if champion_id not in scoped_bench or champion_id not in pickable_set:
                    status = "unswappable"
                elif allow_subset and str(champ_state.get("timer_phase") or "") == "BAN_PICK":
                    status = (
                        "subset-swappable" if subset_available and champion_id in subset_set
                        else "unknown" if not subset_available
                        else "waiting-on-finalization"
                    )
                else:
                    status = "swappable"
                expected_swaps.append({"id": champion_id, "status": status})
        delayed_by_kind = {
            "delayed_pick": next((item for item in delayed_action_plan if item.get("action_type") == "pick"), None),
            "delayed_ban": next((item for item in delayed_action_plan if item.get("action_type") == "ban"), None),
            "delayed_bench_swap": next((item for item in delayed_action_plan if item.get("kind") == "bench-swap"), None),
            "delayed_trade": next((item for item in delayed_action_plan if item.get("kind") == "champion-swap"), None),
        }
        expected_picks = expected_rows(pick_candidates, current_pickable, pickable_available, "pick")
        expected_bans = expected_rows(ban_candidates, current_bannable, bannable_available, "ban")
        expected_pick = next((row for row in expected_picks if row.get("status") in {"pickable", "subset-pickable"}), None)
        expected_ban = next((row for row in expected_bans if row.get("status") == "bannable"), None)
        expected_swap = next((row for row in expected_swaps if row.get("status") in {"swappable", "subset-swappable"}), None)
        move = champ_state.get("auto_select_move")
        active_action = champ_state.get("active_action")
        pick_enabled = bool(active_profile.pick.enabled or self.settings.auto_pick_champion_ids)
        ban_enabled = bool(active_profile.ban.enabled or self.settings.auto_ban_champion_ids)
        trade_rows = list(champ_state.get("trades") or [])
        auto_select_public = {
            "schema_version": 1,
            "enabled": bool(self.settings.automation_enabled and self.settings.auto_select_enabled),
            "temporarily_disabled": bool(self.auto_select_temporarily_disabled),
            "active_group_id": active_group_id,
            "assigned_position": position,
            "game_mode": self.game_mode or None,
            "queue_type": self._active_auto_select_queue_type or None,
            "move": move,
            "current_action": active_action,
            "first_unfinished_pick_action": champ_state.get("first_unfinished_pick_action"),
            "expected_pick": expected_pick,
            "expected_picks": expected_picks,
            "expected_ban": expected_ban,
            "expected_bans": expected_bans,
            "expected_swap": expected_swap,
            "expected_swaps": expected_swaps,
            "current_pickable_ids": current_pickable,
            "current_bannable_ids": current_bannable,
            "subset_champion_ids": subset,
            "scoped_bench_champion_ids": scoped_bench,
            "subset": {
                "ids": subset,
                "available": bool(champ_state.get("subset_champion_ids_available")),
            },
            "scoped_bench": scoped_bench,
            "actionability": {
                "intent": bool(move == "pick-intent" and pick_enabled and active_profile.pick.show_intent),
                "show": bool(move in {"show-pick", "show-ban"} and (pick_enabled if move == "show-pick" else ban_enabled)),
                "complete": bool(move in {"complete-pick", "complete-subset-pick", "complete-ban"} and (pick_enabled if move != "complete-ban" else ban_enabled)),
                "vote": bool(move == "vote"),
                "subset_pick": bool(move in {"show-subset-pick", "complete-subset-pick"} and pick_enabled and subset_available),
                "bench_swap": bool(move in {"bench-swap", "subset-bench-swap"} and pick_enabled),
                "trade_accept": any(bool(item.get("can_accept")) for item in trade_rows),
                "trade_decline": any(bool(item.get("can_decline")) for item in trade_rows),
                "trades": trade_rows,
                "current_action": bool(active_action),
                "current_pickable_ids_available": pickable_available,
                "current_bannable_ids_available": bannable_available,
            },
            **delayed_by_kind,
            "config": {
                "master_enabled": bool(self.settings.automation_enabled),
                "feature_enabled": bool(self.settings.auto_select_enabled),
                "profiles": {key: value.model_dump() for key, value in self.settings.auto_select_profiles.items()},
                "pick_enabled": bool(active_profile.pick.enabled),
                "ban_enabled": bool(active_profile.ban.enabled),
                "show_intent": bool(active_profile.pick.show_intent),
                "ignore_intent": bool(active_profile.pick.ignore_intent),
                "pick_strategy": active_profile.pick.strategy,
                "ban_strategy": active_profile.ban.strategy,
                "bench_select_first_available_champion": bool(active_profile.pick.bench_select_first_available_champion),
                "bench_handle_trade_enabled": bool(active_profile.pick.bench_handle_trade_enabled),
                "pick_delay_seconds": float(active_profile.pick.delay_seconds),
                "ban_delay_seconds": float(active_profile.ban.delay_seconds),
                "bench_swap_accumulated_delay_seconds": float(active_profile.pick.bench_swap_accumulated_delay_seconds),
                "legacy_pick_ids": list(self.settings.auto_pick_champion_ids),
                "legacy_ban_ids": list(self.settings.auto_ban_champion_ids),
            },
        }
        return {
            "connected": credentials is not None,
            "client_window_detected": client_window_detected,
            "requires_elevation": credentials is None and client_window_detected,
            "phase": self.phase,
            "game_mode": self.game_mode,
            "gameflow_session": self.gameflow_session if isinstance(self.gameflow_session, dict) else {},
            "summoner_name": self.summoner_name,
            "current_summoner": self.current_summoner,
            "region": credentials.region if credentials else "",
            "platform_id": credentials.platform_id if credentials else "",
            "client_pid": credentials.pid if credentials else 0,
            "available_client_count": len(self._available_clients),
            "last_error": self.last_error,
            "last_action": self.last_action,
            "last_action_at": self.last_action_at or None,
            "ready_check": self.ready_check,
            "matchmaking_search": self.matchmaking_search,
            "ongoing_champion_swap": self.ongoing_champion_swap,
            "dodge_loop": dict(self._dodge_loop_state),
            "champ_select": self.champ_select,
            "auto_select": auto_select_public,
            "auto_select_temporarily_disabled": self.auto_select_temporarily_disabled,
            "event_stream_connected": self._event_connected,
            "last_event_at": self._last_event_at or None,
            # Local, aggregate-only diagnostics.  Deliberately exclude LCU
            # paths, credentials, response bodies, and player identifiers.
            "diagnostics": {
                "refresh_count": self._snapshot_refresh_count,
                "coalesced_count": self._snapshot_coalesced_count,
                "last_duration_ms": self._snapshot_last_duration_ms,
                "age_ms": snapshot_age_ms,
            },
            "matchmaking_status": self._matchmaking_status,
            "matchmaking_status_reason": self._matchmaking_status_reason,
            "matchmaking_due_at": (time.time() + max(0.0, self._matchmaking_due_at - time.monotonic())) if self._matchmaking_due_at else None,
            "action_countdown": action_countdown,
            "action_plan": {
                "accept_due": public_due("ready-check", "自动接受对局", self._accept_due_at)
                if self.phase == "ReadyCheck" else None,
                "phase_due": phase_due,
                "champion_due": champion_due,
                "delayed": delayed_action_plan,
            },
            "delayed_action_plan": delayed_action_plan,
            "respawn_timer": self.respawn_timer,
            "cooldown_timer_should_show": self.settings.cooldown_timer_enabled and self.phase == "InProgress" and self.game_mode in {"CLASSIC", "PRACTICETOOL", "ARAM", "URF", "ONEFORALL", "NEXUSBLITZ", "ULTBOOK", "KIWI"},
            "mini_should_show": bool(credentials) and self.settings.mini_enabled and self.settings.mini_auto_show and self.phase in {"Lobby", "Matchmaking", "ReadyCheck", "ChampSelect"} and not bool(self.champ_select.get("is_spectating")),
            "settings": self.settings.model_dump(),
        }

    async def _refresh_state(self) -> None:
        try:
            phase = await self.request("GET", "/lol-gameflow/v1/gameflow-phase")
            self.phase = str(phase or "")
            if self.phase != "ChampSelect":
                self._terminate_dodge_loop("phase-exited")
            self.ready_check = None
            self.matchmaking_search = None
            previous_ongoing_swap = self.ongoing_champion_swap
            self.ongoing_champion_swap = None
            if self.phase != "ChampSelect":
                self.champ_select = {}
                self.auto_select_temporarily_disabled = False
                self._active_auto_select_group = "default"
                self._active_auto_select_position = "default"
                self._active_auto_select_queue_type = ""
                self._trade_created_at.clear()

            async def optional(path: str):
                try:
                    return await self.request("GET", path)
                except RuntimeError:
                    return None

            if self.phase in {"Lobby", "Matchmaking", "ReadyCheck", "ChampSelect", "InProgress"}:
                # The session is a read-only snapshot.  It is available in
                # lobby/ready-check on the same client builds used by
                # LeagueAkari and contains the authoritative map icon/name and
                # queue mode used by the Mini Lounge.
                gameflow = await optional("/lol-gameflow/v1/session")
                self.gameflow_session = gameflow if isinstance(gameflow, dict) else {}
                self.game_mode = str((((gameflow or {}).get("gameData") or {}).get("queue") or {}).get("gameMode") or "").upper()
            else:
                gameflow = {}
                self.gameflow_session = {}
                self.game_mode = ""
            summoner = await self.request("GET", "/lol-summoner/v1/current-summoner")
            if isinstance(summoner, dict):
                self.summoner_name = str(summoner.get("gameName") or summoner.get("displayName") or "")
                self.current_summoner = {
                    "puuid": summoner.get("puuid"),
                    "game_name": summoner.get("gameName") or summoner.get("displayName"),
                    "tag_line": summoner.get("tagLine") or "",
                    "summoner_level": summoner.get("summonerLevel"),
                    "profile_icon_id": summoner.get("profileIconId"),
                    "summoner_id": summoner.get("summonerId"),
                }
            self.last_error = ""
            if self.phase == "ChampSelect":
                session = await self.request("GET", "/lol-champ-select/v1/session")
                if isinstance(session, dict):
                    self._active_auto_select_group = self._mode_group(
                        gameflow if isinstance(gameflow, dict) else {}, session
                    )
                    self._active_auto_select_position = self._position_for_session(session)
                    queue = ((gameflow or {}).get("gameData") or {}).get("queue") or {}
                    self._active_auto_select_queue_type = str(queue.get("type") or "")
                self.ongoing_champion_swap = await optional("/lol-champ-select/v1/ongoing-champion-swap")
                if isinstance(self.ongoing_champion_swap, dict):
                    trade_id = str(self.ongoing_champion_swap.get("id") or "")
                    trade_state = str(self.ongoing_champion_swap.get("state") or "").upper()
                    if trade_id and trade_state == "RECEIVED" and not bool(
                        self.ongoing_champion_swap.get("initiatedByLocalPlayer")
                        or self.ongoing_champion_swap.get("initiated_by_local_player")
                    ):
                        previous_id = str((previous_ongoing_swap or {}).get("id") or "")
                        previous_state = str((previous_ongoing_swap or {}).get("state") or "").upper()
                        if previous_id != trade_id or previous_state != "RECEIVED":
                            self._trade_created_at[trade_id] = time.monotonic()
                    else:
                        self._trade_created_at.clear()
                else:
                    self._trade_created_at.clear()
                # These are read-only snapshots.  Some client builds do not
                # expose one or more routes, so absence must degrade to an
                # explicit unavailable list rather than creating a write.
                pickable_ids = await optional("/lol-champ-select/v1/pickable-champion-ids")
                bannable_ids = await optional("/lol-champ-select/v1/bannable-champion-ids")
                subset_ids = None
                if isinstance(session, dict) and session.get("allowSubsetChampionPicks"):
                    subset_ids = await optional("/lol-lobby-team-builder/champ-select/v1/subset-champion-list")
                self.champ_select = self._normalize_champ_select(
                    session,
                    self.ongoing_champion_swap,
                    pickable_ids=pickable_ids,
                    bannable_ids=bannable_ids,
                    subset_ids=subset_ids,
                    grid_champions=(session.get("gridChampions") if isinstance(session, dict) else None),
                )
                try:
                    skin_info, skin_rows = await asyncio.gather(
                        self.request("GET", "/lol-champ-select/v1/skin-selector-info"),
                        self.request("GET", "/lol-champ-select/v1/skin-carousel-skins"),
                    )
                    self.champ_select["skin_selector"] = _normalize_skin_selector(skin_info, skin_rows)
                except RuntimeError:
                    self.champ_select["skin_selector"] = {"available": False, "skins": []}
            elif self.phase == "ReadyCheck":
                self.ready_check = _normalize_ready_check(await optional("/lol-matchmaking/v1/ready-check"))
            elif self.phase in {"Lobby", "Matchmaking"}:
                self.matchmaking_search = _normalize_matchmaking_search(await optional("/lol-matchmaking/v1/search"))
        except RuntimeError:
            return

    async def _refresh_respawn_timer(self) -> None:
        """Read the local in-game Live Client Data endpoint without external credentials."""
        if not self.settings.respawn_timer_enabled or self.phase != "InProgress":
            self.respawn_timer = {"available": False, "dead": False, "time_left": 0.0, "total_time": 0.0}
            return
        try:
            async with httpx.AsyncClient(verify=False, timeout=1.5) as client:
                response = await client.get("https://127.0.0.1:2999/liveclientdata/playerlist")
            response.raise_for_status()
            players = response.json()
            own_name = str(self.current_summoner.get("game_name") or "").casefold()
            own_riot_id = f"{self.current_summoner.get('game_name') or ''}#{self.current_summoner.get('tag_line') or ''}".casefold()
            player = next((row for row in players if isinstance(row, dict) and (
                str(row.get("riotId") or "").casefold() == own_riot_id
                or str(row.get("summonerName") or "").casefold() == own_name
            )), None)
            timer = float((player or {}).get("respawnTimer") or 0.0)
            dead = bool((player or {}).get("isDead")) and timer > 0
            previous_total = float(self.respawn_timer.get("total_time") or 0.0)
            self.respawn_timer = {
                "available": player is not None,
                "dead": dead,
                "time_left": round(timer if dead else 0.0, 1),
                "total_time": round(max(previous_total if dead else 0.0, timer), 1),
            }
        except (httpx.HTTPError, TypeError, ValueError):
            self.respawn_timer = {"available": False, "dead": False, "time_left": 0.0, "total_time": 0.0}

    async def _record_action(
        self,
        label: str,
        method: str,
        path: str,
        *,
        credentials: LcuCredentials | None = None,
    ) -> None:
        if credentials is None:
            await self.request(method, path)
        else:
            await self.request(method, path, credentials=credentials)
        self.last_action = label
        self.last_action_at = time.time()

    async def _send_lcu_phase_chat(self, message: str, *, phase: str, message_type: str = "celebration") -> bool:
        """Send a local automation notice to the matching LCU conversation.

        Chat feedback is deliberately best-effort.  It never turns an
        otherwise successful account action into a failure when a client
        build omits the conversation endpoint, and it remains unreachable on
        a fresh install because every caller is behind its feature switch.
        """

        conversation_types = {
            "ChampSelect": {"championselect", "champion-select"},
            "Lobby": {"customgame", "custom-game"},
        }.get(phase, set())
        if not conversation_types:
            return False
        try:
            conversations = await self.request("GET", "/lol-chat/v1/conversations")
            conversation = next(
                (
                    row
                    for row in conversations
                    if isinstance(row, dict)
                    and str(row.get("type") or "").lower() in conversation_types
                    and row.get("id")
                ),
                None,
            ) if isinstance(conversations, list) else None
            if not conversation:
                return False
            await self.request(
                "POST",
                f"/lol-chat/v1/conversations/{conversation['id']}/messages",
                json_body={"body": f"[Insight] {message}", "type": message_type},
            )
            return True
        except (RuntimeError, TypeError, ValueError):
            return False

    async def _send_champion_config_feedback(self, champion_id: int, message: str) -> None:
        await self._send_lcu_phase_chat(
            f"英雄 {champion_id}：{message}",
            phase="ChampSelect",
            message_type="celebration",
        )

    @staticmethod
    def _normalize_champ_select(
        session,
        ongoing_swap=None,
        *,
        pickable_ids=None,
        bannable_ids=None,
        subset_ids=None,
        grid_champions=None,
    ) -> dict:
        # Adapted from LeagueAkari's MIT-licensed auto-select computed state;
        # keep the upstream attribution in sync with THIRD_PARTY_LICENSES.md.
        if not isinstance(session, dict):
            return {}

        local_cell = session.get("localPlayerCellId")

        def same_cell(left, right) -> bool:
            return left is not None and right is not None and str(left) == str(right)

        def normalize_member(member, *, is_local: bool = False) -> dict | None:
            if not isinstance(member, dict):
                return None
            return {
                "cell_id": member.get("cellId"),
                # A pick intent is not the champion currently held by the
                # player.  LeagueAkari keeps these values separate because
                # the intent may be set during PLANNING before any champion
                # has been selected.
                "champion_id": member.get("championId"),
                "champion_pick_intent": member.get("championPickIntent"),
                "assigned_position": member.get("assignedPosition") or "",
                "summoner_id": member.get("summonerId"),
                "puuid": member.get("puuid"),
                "game_name": member.get("gameName") or member.get("summonerName") or member.get("displayName") or "",
                "tag_line": member.get("tagLine") or "",
                "profile_icon_id": member.get("profileIconId") or member.get("profileIcon"),
                "is_local": is_local or same_cell(member.get("cellId"), local_cell),
            }

        all_members = []
        member_by_cell: dict[str, dict] = {}
        for team_key in ("myTeam", "theirTeam"):
            for raw_member in session.get(team_key) or []:
                member = normalize_member(raw_member)
                if not member or member.get("cell_id") is None:
                    continue
                key = str(member["cell_id"])
                if key not in member_by_cell:
                    member_by_cell[key] = member
                    all_members.append(member)
        local_member = next((member for member in all_members if same_cell(member.get("cell_id"), local_cell)), None)
        if local_member is None:
            raw_local = next((item for item in session.get("myTeam") or [] if isinstance(item, dict) and same_cell(item.get("cellId"), local_cell)), {})
            local_member = normalize_member(raw_local, is_local=True) or {}

        timer = session.get("timer") or {}

        def normalize_ids(values) -> list[int]:
            if not isinstance(values, (list, tuple, set)):
                return []
            normalized: list[int] = []
            for value in values:
                try:
                    champion_id = int(value)
                except (TypeError, ValueError):
                    continue
                if champion_id not in normalized:
                    normalized.append(champion_id)
            return normalized

        current_pickable_ids = normalize_ids(pickable_ids)
        current_bannable_ids = normalize_ids(bannable_ids)
        subset_champion_ids = normalize_ids(subset_ids)

        def normalize_grid(values) -> dict[int, dict]:
            """Normalize LCU grid champion selection evidence.

            Different client builds expose this as either an id-keyed object
            or an array of records.  Only the fields used by LeagueAkari's
            expected-pick/ban computation are retained.
            """

            rows: list[tuple[object, object]] = []
            if isinstance(values, dict):
                rows = list(values.items())
            elif isinstance(values, list):
                rows = [
                    (
                        item.get("id") if isinstance(item, dict) else None,
                        item,
                    )
                    for item in values
                ]
            normalized_grid: dict[int, dict] = {}
            for key, raw in rows:
                if not isinstance(raw, dict):
                    continue
                champion_value = raw.get("championId") or raw.get("champion_id") or key
                try:
                    champion_id = int(champion_value)
                except (TypeError, ValueError):
                    continue
                selection = raw.get("selectionStatus") or raw.get("selection_status") or {}
                if not isinstance(selection, dict):
                    selection = {}
                normalized_grid[champion_id] = {
                    "owned": raw.get("owned"),
                    "selection_status": {
                        "is_banned": bool(selection.get("isBanned") if "isBanned" in selection else selection.get("is_banned")),
                        "pick_intented": bool(selection.get("pickIntented") if "pickIntented" in selection else selection.get("pick_intented")),
                        "pick_intented_by_me": bool(
                            selection.get("pickIntentedByMe")
                            if "pickIntentedByMe" in selection
                            else selection.get("pick_intented_by_me")
                        ),
                        "picked_by_other_or_banned": bool(
                            selection.get("pickedByOtherOrBanned")
                            if "pickedByOtherOrBanned" in selection
                            else selection.get("picked_by_other_or_banned")
                        ),
                        "selected_by_me": bool(
                            selection.get("selectedByMe")
                            if "selectedByMe" in selection
                            else selection.get("selected_by_me")
                        ),
                    },
                }
            return normalized_grid

        raw_grid_champions = grid_champions if grid_champions is not None else session.get("gridChampions")
        normalized_grid = normalize_grid(raw_grid_champions)

        # LeagueAkari's computed state deliberately scopes actions to the
        # first action group which is not completely finished.  Looking at all
        # ``isInProgress`` actions can make a stale pick/ban race a new LCU
        # phase, so retain both the scoped group and the first unfinished pick
        # for the read-only status surface and the guarded executor below.
        action_groups = [group for group in (session.get("actions") or []) if isinstance(group, list)]
        current_group_index = next(
            (
                index
                for index, group in enumerate(action_groups)
                if group and not all(bool(action.get("completed")) for action in group if isinstance(action, dict))
            ),
            -1,
        )

        def normalize_action(action) -> dict | None:
            if not isinstance(action, dict):
                return None
            try:
                champion_id = int(action.get("championId") or 0)
            except (TypeError, ValueError):
                champion_id = 0
            return {
                "id": action.get("id"),
                "type": action.get("type"),
                "champion_id": champion_id,
                "completed": bool(action.get("completed")),
                "in_progress": bool(action.get("isInProgress")),
                "actor_cell_id": action.get("actorCellId"),
                "pick_turn": action.get("pickTurn"),
                "is_local": same_cell(action.get("actorCellId"), local_cell),
            }

        scoped_group = action_groups[current_group_index] if current_group_index >= 0 else []
        current_actions = [
            normalized
            for action in scoped_group
            if (normalized := normalize_action(action)) is not None
            and normalized["is_local"]
        ]
        active_action = next((action for action in current_actions if not action["completed"]), None)
        first_unfinished_pick_action = next(
            (
                normalized
                for group in action_groups
                for action in group
                if (normalized := normalize_action(action)) is not None
                and normalized["is_local"]
                and normalized["type"] == "pick"
                and not normalized["completed"]
            ),
            None,
        )
        is_picking_now = active_action is not None and active_action.get("type") == "pick"
        is_banning_now = active_action is not None and active_action.get("type") == "ban"
        is_voting_now = active_action is not None and active_action.get("type") == "vote"
        is_pick_intenting = bool(
            str(timer.get("phase") or "") == "PLANNING"
            or (first_unfinished_pick_action and not is_picking_now and not is_banning_now)
        )
        champion_shown = bool(active_action and active_action.get("champion_id"))
        if is_pick_intenting:
            auto_select_move = "pick-intent"
        elif is_picking_now:
            auto_select_move = (
                "complete-subset-pick" if champion_shown else "show-subset-pick"
            ) if session.get("allowSubsetChampionPicks") else (
                "complete-pick" if champion_shown else "show-pick"
            )
        elif is_banning_now:
            auto_select_move = "complete-ban" if champion_shown else "show-ban"
        elif is_voting_now:
            auto_select_move = "vote"
        elif session.get("benchEnabled") and local_member.get("champion_id"):
            auto_select_move = (
                "subset-bench-swap"
                if session.get("allowSubsetChampionPicks") and str(timer.get("phase") or "") == "BAN_PICK"
                else "bench-swap"
            )
        else:
            auto_select_move = None
        my_actions = []
        for group in session.get("actions") or []:
            action = next((item for item in (group or []) if isinstance(item, dict) and same_cell(item.get("actorCellId"), local_cell)), None)
            if action:
                my_actions.append({
                    "id": action.get("id"),
                    "type": action.get("type"),
                    "champion_id": int(action.get("championId") or 0),
                    "completed": bool(action.get("completed")),
                    "in_progress": bool(action.get("isInProgress")),
                })

        # Keep this in lockstep with the existing automatic trade handler;
        # ``SENT`` is a locally initiated request and is not safe to accept or
        # decline through the incoming-trade endpoint.
        actionable_states = {"AVAILABLE", "PENDING", "RECEIVED"}
        trades = []
        for raw_trade in session.get("trades") or []:
            if not isinstance(raw_trade, dict) or raw_trade.get("id") is None:
                continue
            trade_id = raw_trade.get("id")
            state = str(raw_trade.get("state") or "")
            requester_cell = raw_trade.get("requesterCellId") or raw_trade.get("initiatorCellId")
            responder_cell = raw_trade.get("responderCellId") or raw_trade.get("otherCellId")
            explicit_local = raw_trade.get("initiatedByLocalPlayer")
            if explicit_local is not None:
                initiated_by_local = bool(explicit_local)
                initiator_cell = local_cell if initiated_by_local else requester_cell
                other_cell = responder_cell or raw_trade.get("cellId")
                if not initiated_by_local and initiator_cell is None:
                    initiator_cell = raw_trade.get("cellId")
                if initiated_by_local and other_cell is None and requester_cell is not None and not same_cell(requester_cell, local_cell):
                    other_cell = requester_cell
            elif requester_cell is not None:
                initiated_by_local = same_cell(requester_cell, local_cell)
                initiator_cell = requester_cell
                other_cell = responder_cell or (local_cell if not initiated_by_local else raw_trade.get("cellId"))
            else:
                # The current LCU session generally exposes only ``cellId``;
                # for an incoming request it identifies the remote player.
                other_cell = raw_trade.get("cellId") if not same_cell(raw_trade.get("cellId"), local_cell) else None
                initiated_by_local = same_cell(raw_trade.get("cellId"), local_cell)
                initiator_cell = local_cell if initiated_by_local else raw_trade.get("cellId")

            initiator = member_by_cell.get(str(initiator_cell)) if initiator_cell is not None else None
            other_player = member_by_cell.get(str(other_cell)) if other_cell is not None else None
            actionable = state.upper() in actionable_states and not initiated_by_local
            reason = "available" if actionable else ("missing-state" if not state else "state-not-actionable")
            trades.append({
                "id": trade_id,
                "state": state,
                "initiator_cell_id": initiator_cell,
                "initiator": initiator,
                "initiated_by_local_player": initiated_by_local,
                "other_player": other_player,
                "other_player_cell_id": other_cell,
                "actionable": actionable,
                "can_accept": actionable,
                "can_decline": actionable,
                "actionability": {
                    "actionable": actionable,
                    "can_accept": actionable,
                    "can_decline": actionable,
                    "reason": reason,
                },
            })

        normalized_ongoing_swap = None
        if isinstance(ongoing_swap, dict):
            ongoing_id = ongoing_swap.get("id")
            ongoing_state = str(ongoing_swap.get("state") or "")
            ongoing_actionable = ongoing_state.upper() == "RECEIVED" and ongoing_id is not None
            normalized_ongoing_swap = {
                "id": ongoing_id,
                "state": ongoing_swap.get("state"),
                "actionable": ongoing_actionable,
                "can_accept": ongoing_actionable,
                "can_decline": ongoing_actionable,
                "initiated_by_local_player": bool(ongoing_swap.get("initiatedByLocalPlayer")),
                "other_summoner_index": ongoing_swap.get("otherSummonerIndex"),
                "requester_champion_id": ongoing_swap.get("requesterChampionId"),
                "responder_champion_id": ongoing_swap.get("responderChampionId"),
                "requester_champion_name": ongoing_swap.get("requesterChampionName"),
                "responder_champion_name": ongoing_swap.get("responderChampionName"),
            }

        timer_is_infinite = bool(timer.get("isInfinite"))
        try:
            remaining_at_sample_ms = float(
                timer.get("adjustedTimeLeftInPhase") or timer.get("timeLeftInPhase") or 0
            )
        except (TypeError, ValueError):
            remaining_at_sample_ms = 0.0
        try:
            internal_now_ms = timer.get("internalNowInEpochMs")
            elapsed_since_sample_ms = (
                max(0.0, time.time() * 1000.0 - float(internal_now_ms))
                if internal_now_ms is not None
                else 0.0
            )
        except (TypeError, ValueError):
            internal_now_ms = None
            elapsed_since_sample_ms = 0.0
        remaining_ms = int(
            max(0.0, remaining_at_sample_ms - elapsed_since_sample_ms)
            if not timer_is_infinite
            else 0.0
        )
        try:
            total_time_ms = int(timer.get("totalTimeInPhase") or 0)
        except (TypeError, ValueError):
            total_time_ms = 0
        my_team_cells = {
            str(item.get("cell_id"))
            for raw_member in session.get("myTeam") or []
            for item in [normalize_member(raw_member)]
            if item and item.get("cell_id") is not None
        }
        return {
            "local_player_cell_id": session.get("localPlayerCellId"),
            "current_champion_id": local_member.get("champion_id"),
            "my_team": [member for member in all_members if str(member.get("cell_id")) in my_team_cells],
            "all_players": all_members,
            "bench_enabled": bool(session.get("benchEnabled")),
            "bench_champions": [int(item.get("championId")) for item in (session.get("benchChampions") or []) if isinstance(item, dict) and item.get("championId")],
            "rerolls_remaining": int(session.get("rerollsRemaining") or 0),
            # Keep support evidence strict: truthy strings or numeric values
            # from an incomplete/malformed payload must not make the Mini
            # render a write-capable reroll control.
            "allow_rerolling": session.get("allowRerolling") is True,
            "allow_subset_champion_picks": bool(session.get("allowSubsetChampionPicks")),
            "allow_duplicate_picks": bool(session.get("allowDuplicatePicks")),
            "is_custom_game": bool(session.get("isCustomGame")),
            "timer_phase": str(timer.get("phase") or ""),
            "timer_adjusted_time_left_ms": remaining_ms,
            "timer_total_time_ms": total_time_ms,
            "timer_is_infinite": timer_is_infinite,
            "timer_internal_now_ms": internal_now_ms,
            "timer_elapsed_ms": max(0, total_time_ms - remaining_ms) if total_time_ms else None,
            "timer_deadline_at": (
                time.time() + max(0, remaining_ms) / 1000
                if not timer_is_infinite and remaining_ms > 0
                else None
            ),
            "is_spectating": bool(session.get("isSpectating")),
            "my_actions": my_actions,
            "current_action_group_index": current_group_index,
            "current_actions": current_actions,
            "active_action": active_action,
            "first_unfinished_pick_action": first_unfinished_pick_action,
            "is_pick_intenting": is_pick_intenting,
            "is_picking_now": is_picking_now,
            "is_banning_now": is_banning_now,
            "is_voting_now": is_voting_now,
            "auto_select_move": auto_select_move,
            "current_session_champion_id": local_member.get("champion_id"),
            "current_pickable_champion_ids": current_pickable_ids,
            "current_bannable_champion_ids": current_bannable_ids,
            "current_pickable_ids_available": pickable_ids is not None,
            "current_bannable_ids_available": bannable_ids is not None,
            "subset_champion_ids": subset_champion_ids,
            "subset_champion_ids_available": subset_ids is not None,
            "grid_champions": normalized_grid,
            "grid_champions_available": raw_grid_champions is not None,
            "trades": trades,
            "ongoing_champion_swap": normalized_ongoing_swap,
        }

    @staticmethod
    def _mode_group(gameflow: dict | None, session: dict | None = None) -> str:
        game_data = (gameflow or {}).get("gameData") or {}
        queue = game_data.get("queue") or {}
        queue_id = int(queue.get("id") or game_data.get("queueId") or 0)
        mode = str(queue.get("gameMode") or game_data.get("gameMode") or "").upper()
        queue_type = str(queue.get("type") or "").upper()
        if bool(game_data.get("isCustomGame")) or bool((session or {}).get("isCustomGame")):
            return "custom"
        if bool((session or {}).get("benchEnabled")) or queue_id == 450 or mode in {"ARAM", "KIWI"}:
            return "aram"
        if queue_id in {420, 440} or "RANKED" in queue_type:
            return "ranked"
        if queue_id in {1700, 1710} or mode == "CHERRY":
            return "cherry"
        if mode == "ONEFORALL" or "ONEFORALL" in queue_type:
            return "oneforall"
        if mode in {"ULTBOOK", "ULTIMATE_SPELLBOOK"} or "ULTBOOK" in queue_type or "ULTIMATE_SPELLBOOK" in queue_type:
            return "ultbook"
        if "URF" in mode or "URF" in queue_type:
            return "urf"
        if queue_id in {830, 840, 850, 950, 960} or any(token in mode.replace("_", "") for token in ("DOOMBOT", "BOT")) or "BOT" in queue_type:
            return "bot"
        if queue_id == 700 or "CLASH" in queue_type:
            return "normal"
        if queue_id in {400, 430, 480, 490} or mode in {"CLASSIC", "SWIFTPLAY"}:
            return "normal"
        return "default"

    @staticmethod
    def _position_for_session(session: dict) -> str:
        local_cell = session.get("localPlayerCellId")
        member = next((item for item in (session.get("myTeam") or []) if item.get("cellId") == local_cell), {})
        value = str(member.get("assignedPosition") or "default").lower()
        aliases = {"mid": "middle", "adc": "bottom", "support": "utility"}
        value = aliases.get(value, value)
        return value if value in {"top", "jungle", "middle", "bottom", "utility"} else "default"

    @staticmethod
    def _profile_candidates(pool: dict[str, list[int]], position: str) -> list[int]:
        # LeagueAkari resolves exactly the active position page.  A role page
        # does not silently append the generic page, otherwise a champion
        # configured for ``default`` can unexpectedly override a role list.
        key = position if position in {"top", "jungle", "middle", "bottom", "utility"} else "default"
        return list(dict.fromkeys(pool.get(key) or []))

    def _set_delayed_action_plan(
        self,
        action_id: str,
        *,
        kind: str,
        label: str,
        due_at: float,
        **metadata,
    ) -> None:
        """Record a delayed action without making the plan executable by itself."""
        key = str(action_id)
        previous = self._delayed_action_plan.get(key) or {}
        now_wall = time.time()
        now_mono = time.monotonic()
        try:
            delay_seconds = max(0.0, float(metadata.get("delay_seconds")))
        except (TypeError, ValueError):
            delay_seconds = max(0.0, float(due_at) - now_mono)
        start_at = previous.get("start_at") or now_wall
        finish_at = previous.get("finish_at") or now_wall + max(0.0, float(due_at) - now_mono)
        action_type = str(metadata.get("action_type") or "")
        move = str(metadata.get("move") or "")
        self._delayed_action_plan[key] = {
            "action_id": key,
            "kind": kind,
            "label": label,
            "due_at": due_at,
            "start_at": start_at,
            "finish_at": finish_at,
            "delay_ms": int(round(delay_seconds * 1000.0)),
            "is_pick_intent": bool(action_type == "pick" and move == "pick-intent"),
            **metadata,
        }

    def _clear_delayed_action_plan(self, action_id: str) -> None:
        self._delayed_action_plan.pop(str(action_id), None)

    @staticmethod
    def _phase_calibrated_delay_seconds(
        session: dict,
        configured_seconds: float,
        *,
        target_offset_seconds: float | None = None,
    ) -> float:
        """Return LeagueAkari's phase-calibrated delay.

        LCU's ``adjustedTimeLeftInPhase`` is the value at
        ``internalNowInEpochMs`` rather than a continuously decreasing
        counter.  LeagueAkari subtracts the elapsed time and then caps the
        requested action at both the target offset (used by show-and-lock-in)
        and the real phase deadline.  Older client builds omit the epoch; in
        that case we retain the conservative remaining-time cap.
        """
        try:
            configured = max(0.0, float(configured_seconds))
        except (TypeError, ValueError):
            configured = 0.0
        timer = session.get("timer") if isinstance(session, dict) else None
        if not isinstance(timer, dict):
            return configured
        try:
            remaining_at_sample_ms = float(
                timer.get("adjustedTimeLeftInPhase") or timer.get("timeLeftInPhase") or 0
            )
        except (TypeError, ValueError):
            return configured
        if remaining_at_sample_ms <= 0:
            return configured
        internal_now = timer.get("internalNowInEpochMs")
        try:
            elapsed_since_sample_ms = (
                max(0.0, time.time() * 1000.0 - float(internal_now))
                if internal_now is not None
                else 0.0
            )
        except (TypeError, ValueError):
            elapsed_since_sample_ms = 0.0
        remaining_ms = max(0.0, remaining_at_sample_ms - elapsed_since_sample_ms)
        if target_offset_seconds is None:
            return max(0.0, min(configured, remaining_ms / 1000.0))
        target_ms = max(0.0, float(target_offset_seconds) * 1000.0)
        total_ms = timer.get("totalTimeInPhase")
        try:
            total_ms = float(total_ms) if total_ms is not None else target_ms
        except (TypeError, ValueError):
            total_ms = target_ms
        elapsed_ms = max(0.0, total_ms - remaining_ms)
        return max(0.0, min(configured, target_ms - elapsed_ms, remaining_ms) / 1000.0)

    async def _active_auto_select_profile(self, session: dict) -> tuple[str, AutoSelectProfile]:
        try:
            gameflow = await self.request("GET", "/lol-gameflow/v1/session")
        except RuntimeError:
            gameflow = {}
        group = self._mode_group(gameflow if isinstance(gameflow, dict) else {}, session)
        return group, self.settings.auto_select_profiles.get(group) or self.settings.auto_select_profiles.get("default") or AutoSelectProfile()

    async def _run_auto_select(self) -> None:
        # This method is intentionally safe when called directly by a test or
        # a future event handler: no automation master/feature/profile gate,
        # no ChampSelect phase, and no candidate ID means no LCU write.
        if (
            self.auto_select_temporarily_disabled
            or not self.settings.automation_enabled
            or not self.settings.auto_select_enabled
            or self.phase != "ChampSelect"
        ):
            return
        session = await self.request("GET", "/lol-champ-select/v1/session")
        if not isinstance(session, dict):
            return
        local_cell = session.get("localPlayerCellId")
        group, profile = await self._active_auto_select_profile(session)
        position = self._position_for_session(session)
        self._active_auto_select_group = group
        self._active_auto_select_position = position
        try:
            gameflow_queue = ((await self.request("GET", "/lol-gameflow/v1/session") or {}).get("gameData") or {}).get("queue") or {}
        except RuntimeError:
            gameflow_queue = {}
        self._active_auto_select_queue_type = str(gameflow_queue.get("type") or "")
        subset_champion_ids: set[int] = set()
        if session.get("allowSubsetChampionPicks"):
            try:
                subset = await self.request("GET", "/lol-lobby-team-builder/champ-select/v1/subset-champion-list")
                subset_champion_ids = {int(value) for value in (subset or [])}
            except (RuntimeError, TypeError, ValueError):
                subset_champion_ids = set()
        normalized = self._normalize_champ_select(
            session,
            self.ongoing_champion_swap,
            subset_ids=subset_champion_ids if session.get("allowSubsetChampionPicks") else None,
        )
        move = normalized.get("auto_select_move")
        active_action = normalized.get("active_action")
        first_unfinished_pick = normalized.get("first_unfinished_pick_action")
        action = active_action
        if move == "pick-intent" and first_unfinished_pick:
            action = first_unfinished_pick
        teammate_intents = {
            int(member.get("championPickIntent"))
            for member in (session.get("myTeam") or [])
            if member.get("cellId") != local_cell and int(member.get("championPickIntent") or 0) > 0
        }
        # LeagueAkari executes only the current local action.  A fixture may
        # expose a planning pick before the active pick group, hence the
        # separate ``action`` fallback above.
        action_type = action.get("type") if isinstance(action, dict) else None
        if action_type in {"pick", "ban"} and isinstance(action, dict):
            configured = profile.pick if action_type == "pick" else profile.ban
            legacy_candidates = self.settings.auto_pick_champion_ids if action_type == "pick" else self.settings.auto_ban_champion_ids
            candidates = self._profile_candidates(configured.champions, position) if configured.enabled else list(legacy_candidates)
            action_key = f"{action_type}:{action.get('id')}"
            allowed_moves = {
                "pick": {"pick-intent", "show-pick", "complete-pick", "show-subset-pick", "complete-subset-pick"},
                "ban": {"show-ban", "complete-ban"},
            }
            if candidates and action_key not in self._handled_champion_actions and move in allowed_moves[action_type]:
                available_path = "/lol-champ-select/v1/pickable-champion-ids" if action_type == "pick" else "/lol-champ-select/v1/bannable-champion-ids"
                available = await self.request("GET", available_path)
                if action_type == "pick" and configured.enabled and not configured.ignore_intent:
                    candidates = [value for value in candidates if value not in teammate_intents]
                try:
                    available_ids = {int(value) for value in (available or [])}
                except (TypeError, ValueError):
                    available_ids = set()
                if action_type == "pick" and session.get("allowSubsetChampionPicks"):
                    available_ids &= subset_champion_ids
                champion_id = next(
                    (
                        value
                        for value in candidates
                        if value in available_ids or (action_type == "pick" and group == "cherry" and value == -3)
                    ),
                    None,
                )
                if champion_id is not None:
                    delay_config = configured.delay_seconds if configured.enabled else self.settings.champion_action_delay_seconds
                    # LeagueAkari gives show-and-lock-in a two-stage budget:
                    # once the client has entered the complete-* move, the
                    # remaining delay is the configured offset minus elapsed
                    # phase time, capped by the phase deadline.
                    strategy = configured.strategy if configured.enabled else (
                        "show-and-lock-in" if self.settings.champion_lock_in else "just-show"
                    )
                    target_offset = delay_config
                    if strategy == "show-and-lock-in" and move in {"complete-pick", "complete-ban"}:
                        target_offset = delay_config * 2.0
                    delay = self._phase_calibrated_delay_seconds(
                        session,
                        delay_config,
                        target_offset_seconds=target_offset,
                    )
                    active_champion_id = int(action.get("champion_id") or 0)

                    # An already displayed champion is final for just-show;
                    # never replace a user's manual choice.
                    if move in {"complete-pick", "complete-subset-pick", "complete-ban"} and strategy == "just-show" and active_champion_id:
                        self._champion_action_due_at.pop(action_key, None)
                        self._clear_delayed_action_plan(action_key)
                        self._handled_champion_actions.add(action_key)
                    elif strategy == "lock-in-immediately" and move in {"complete-pick", "complete-subset-pick", "complete-ban"} and active_champion_id == champion_id and action_key in self._champion_action_due_at:
                        # Ignore the short LCU transition from shown -> locked.
                        return
                    else:
                        is_intent = move == "pick-intent"
                        if is_intent and (not configured.enabled or not configured.show_intent or active_champion_id):
                            self._champion_action_due_at.pop(action_key, None)
                            self._clear_delayed_action_plan(action_key)
                        else:
                            completed = (
                                not is_intent
                                and (
                                    # The legacy ID-list settings predate the
                                    # upstream two-step show/lock state.  Keep
                                    # their established one-call lock-in
                                    # behavior for backwards compatibility.
                                    (not configured.enabled and self.settings.champion_lock_in)
                                    or
                                    move in {"show-subset-pick", "complete-subset-pick"}
                                    or strategy == "lock-in-immediately"
                                    or (strategy == "show-and-lock-in" and move in {"complete-pick", "complete-ban"})
                                )
                            )
                            due_at = self._champion_action_due_at.get(action_key)
                            if due_at is None:
                                due_at = time.monotonic() + delay
                                self._champion_action_due_at[action_key] = due_at
                                self._set_delayed_action_plan(
                                    action_key,
                                    kind="champion-action",
                                    label="自动选择 / 禁用英雄",
                                    due_at=due_at,
                                    action_type=action_type,
                                    move=move,
                                    champion_id=champion_id,
                                    completed=completed,
                                    delay_seconds=round(delay, 3),
                                )
                            else:
                                plan = self._delayed_action_plan.get(action_key)
                                if plan:
                                    plan.update({"move": move, "champion_id": champion_id, "completed": completed})
                            if time.monotonic() >= due_at:
                                body = {"championId": champion_id}
                                if not is_intent:
                                    body.update({"type": action_type, "completed": completed})
                                await self.request(
                                    "PATCH",
                                    f"/lol-champ-select/v1/session/actions/{action.get('id')}",
                                    json_body=body,
                                )
                                self._champion_action_due_at.pop(action_key, None)
                                self._clear_delayed_action_plan(action_key)
                                if completed or strategy == "just-show":
                                    self._handled_champion_actions.add(action_key)
                                self.last_action = f"[{group}] 已自动{'预选' if is_intent else ('选择' if action_type == 'pick' else '禁用')}英雄 {champion_id}"
                                self.last_action_at = time.time()

        if profile.pick.enabled and profile.pick.bench_handle_trade_enabled:
            await self._run_trade_handling(session, self._profile_candidates(profile.pick.champions, position), profile.pick, normalized)
        if profile.pick.enabled and session.get("benchEnabled"):
            await self._run_bench_swap(session, profile.pick, position, subset_champion_ids, normalized)

    def _trade_received_at(self, trade_id: str, trade: dict | None = None) -> float:
        """Return the monotonic timestamp at which an incoming trade appeared."""

        key = str(trade_id)
        existing = self._trade_created_at.get(key)
        if existing is not None:
            return existing

        now_mono = time.monotonic()
        # A few LCU builds include an epoch timestamp; use it when present.
        # The normal build does not, so first observation is the safe fallback
        # and subsequent polls still subtract the already elapsed delay.
        raw_timestamp = None
        if isinstance(trade, dict):
            for name in ("createdAt", "created_at", "receivedAt", "received_at"):
                if trade.get(name) is not None:
                    raw_timestamp = trade.get(name)
                    break
        if raw_timestamp is not None:
            try:
                value = float(raw_timestamp)
                if value > 10_000_000_000:
                    value /= 1000.0
                if value > 1_000_000_000:
                    elapsed = max(0.0, time.time() - value)
                    existing = now_mono - elapsed
            except (TypeError, ValueError):
                existing = None
        if existing is None:
            existing = now_mono
        self._trade_created_at[key] = existing
        return existing

    async def _run_trade_handling(self, session: dict, expected: list[int], profile: PickProfile | None = None, normalized: dict | None = None) -> None:
        if self.phase != "ChampSelect":
            return
        if not self.settings.automation_enabled or not self.settings.auto_select_enabled:
            return
        profile = profile or PickProfile()
        if not profile.enabled or not profile.bench_handle_trade_enabled:
            return
        # The modern endpoint represents the incoming swap independently of
        # the session's legacy ``trades`` array.  Only RECEIVED swaps are
        # automatically actionable; other states remain status-only.
        ongoing = self.ongoing_champion_swap
        if not isinstance(ongoing, dict) and isinstance((normalized or {}).get("ongoing_champion_swap"), dict):
            ongoing = (normalized or {}).get("ongoing_champion_swap")
        if (
            isinstance(ongoing, dict)
            and str(ongoing.get("state") or "").upper() == "RECEIVED"
            and ongoing.get("id") is not None
            and not bool(ongoing.get("initiatedByLocalPlayer") or ongoing.get("initiated_by_local_player"))
        ):
            trade_id = str(ongoing.get("id"))
            requester_champion = int(ongoing.get("requesterChampionId") or ongoing.get("requester_champion_id") or 0)
            current_champion = int((normalized or {}).get("current_session_champion_id") or 0)
            try:
                requester_index = expected.index(requester_champion)
            except ValueError:
                requester_index = -1
            try:
                current_index = expected.index(current_champion)
            except ValueError:
                current_index = -1
            operation = "accept" if requester_index >= 0 and (current_index < 0 or (profile.bench_select_first_available_champion and requester_index < current_index)) else "decline"
            action_key = f"trade:{trade_id}:{operation}"
            if action_key not in self._handled_trades:
                due_at = self._delayed_action_plan.get(action_key, {}).get("due_at")
                if due_at is None:
                    received_at = self._trade_received_at(trade_id, ongoing)
                    elapsed = max(0.0, time.monotonic() - received_at)
                    # The configured trade delay is cumulative from the first
                    # RECEIVED observation, not a fresh delay on every poll.
                    delay = max(0.0, float(profile.delay_seconds) - elapsed)
                    delay = self._phase_calibrated_delay_seconds(session, delay)
                    due_at = time.monotonic() + delay
                    self._set_delayed_action_plan(action_key, kind="champion-swap", label="自动处理换英雄", due_at=due_at, trade_id=trade_id, operation=operation, champion_id=requester_champion, delay_seconds=round(delay, 3))
                if time.monotonic() >= due_at:
                    await self.request("POST", f"/lol-champ-select/v1/session/champion-swaps/{quote(trade_id, safe='')}/{operation}")
                    self._handled_trades.add(action_key)
                    self._clear_delayed_action_plan(action_key)
                    self._trade_created_at.pop(trade_id, None)
                    self.last_action = f"已{('接受' if operation == 'accept' else '拒绝')}英雄交换：{requester_champion}"
                    self.last_action_at = time.time()
            return
        for trade in session.get("trades") or []:
            trade_id = str(trade.get("id") or "") if isinstance(trade, dict) else ""
            if not trade_id or trade_id in self._handled_trades or trade.get("state") not in {"AVAILABLE", "PENDING", "RECEIVED"}:
                continue
            if bool(trade.get("initiatedByLocalPlayer")):
                continue
            requester_cell = trade.get("requesterCellId")
            requester = next((item for item in (session.get("myTeam") or []) if item.get("cellId") == requester_cell), {})
            requester_champion = int(requester.get("championId") or 0)
            if requester_champion not in expected:
                continue
            action_key = f"trade:{trade_id}:accept"
            if action_key in self._handled_trades:
                continue
            due_at = self._delayed_action_plan.get(action_key, {}).get("due_at")
            if due_at is None:
                received_at = self._trade_received_at(trade_id, trade)
                elapsed = max(0.0, time.monotonic() - received_at)
                delay = max(0.0, float(profile.delay_seconds) - elapsed)
                delay = self._phase_calibrated_delay_seconds(session, delay)
                due_at = time.monotonic() + delay
                self._set_delayed_action_plan(
                    action_key,
                    kind="champion-swap",
                    label="自动处理换英雄",
                    due_at=due_at,
                    trade_id=trade_id,
                    operation="accept",
                    champion_id=requester_champion,
                    delay_seconds=round(delay, 3),
                )
            if time.monotonic() < due_at:
                continue
            await self.request("POST", f"/lol-champ-select/v1/session/champion-swaps/{quote(trade_id, safe='')}/accept")
            self._handled_trades.add(action_key)
            self._trade_created_at.pop(trade_id, None)
            self._clear_delayed_action_plan(action_key)
            self.last_action = f"已接受英雄交换：{requester_champion}"
            self.last_action_at = time.time()

    async def _run_bench_swap(
        self,
        session: dict,
        profile: PickProfile,
        position: str,
        subset_champion_ids: set[int] | None = None,
        normalized: dict | None = None,
    ) -> None:
        if self.phase != "ChampSelect":
            return
        if not self.settings.automation_enabled or not self.settings.auto_select_enabled:
            return
        if not profile.enabled:
            return
        expected = self._profile_candidates(profile.champions, position)
        bench = [
            int(item.get("championId"))
            for item in (session.get("benchChampions") or [])
            if isinstance(item, dict) and item.get("championId")
        ]
        if session.get("allowSubsetChampionPicks") and str((session.get("timer") or {}).get("phase") or "") == "BAN_PICK":
            bench = [champion_id for champion_id in bench if champion_id in (subset_champion_ids or set())]
        current_champion = int((normalized or {}).get("current_session_champion_id") or 0)
        try:
            current_index = expected.index(current_champion)
        except ValueError:
            current_index = -1
        candidate = next((champion_id for champion_id in expected if champion_id in bench), None)
        try:
            candidate_index = expected.index(candidate) if candidate is not None else -1
        except ValueError:
            candidate_index = -1
        # LeagueAkari swaps only when the held champion is not in the expected
        # pool, or when the first available bench candidate has higher
        # priority. Never downgrade a configured current champion.
        if candidate is not None and current_index >= 0 and (
            not profile.bench_select_first_available_champion or candidate_index >= current_index
        ):
            candidate = None
        if candidate is None:
            self._bench_candidate_since.clear()
            self._clear_delayed_action_plan("bench-swap")
            return
        started = self._bench_candidate_since.setdefault(candidate, time.monotonic())
        delay = max(0.0, float(profile.bench_swap_accumulated_delay_seconds) - (time.monotonic() - started))
        # The bench cooldown is wall-clock based, but a phase can end before
        # it expires.  Match LeagueAkari by capping the pending task against
        # the corrected LCU phase timer and never scheduling past lock-in.
        delay = self._phase_calibrated_delay_seconds(session, delay)
        move = (normalized or {}).get("auto_select_move") or ("subset-bench-swap" if session.get("allowSubsetChampionPicks") else "bench-swap")
        due_at = time.monotonic() + delay
        self._set_delayed_action_plan("bench-swap", kind="bench-swap", label="自动从备战席换取英雄", due_at=due_at, move=move, champion_id=candidate, delay_seconds=round(delay, 3))
        if delay > 0:
            return
        await self.request("POST", f"/lol-champ-select/v1/session/bench/swap/{candidate}")
        self._bench_candidate_since.clear()
        self._clear_delayed_action_plan("bench-swap")
        self.last_action = f"已从备战席换取英雄 {candidate}"
        self.last_action_at = time.time()

    async def _run_champion_config(self) -> None:
        if (
            self.phase != "ChampSelect"
            or not self.settings.automation_enabled
            or not self.settings.auto_champion_config_enabled
        ):
            return
        champion_id = await self.request("GET", "/lol-champ-select/v1/current-champion")
        if not isinstance(champion_id, int) or champion_id <= 0 or champion_id == self._configured_champion_id:
            return
        session = await self.request("GET", "/lol-champ-select/v1/session")
        try:
            gameflow = await self.request("GET", "/lol-gameflow/v1/session")
        except RuntimeError:
            gameflow = {}
        game_data = (gameflow or {}).get("gameData") or {} if isinstance(gameflow, dict) else {}
        queue = game_data.get("queue") or {}
        game_mode = str(queue.get("gameMode") or "").upper()
        queue_type = str(queue.get("type") or "").upper()
        position = self._position_for_session(session if isinstance(session, dict) else {})
        if game_mode == "CLASSIC" and queue_type.startswith("RANKED_"):
            config_keys = [f"ranked-{position}", "ranked-default", "default"]
        elif game_mode == "CLASSIC":
            config_keys = ["normal", "default"]
        else:
            mapped = {"ARAM": "aram", "KIWI": "aram", "URF": "urf", "NEXUSBLITZ": "nexusblitz", "ULTBOOK": "ultbook"}.get(game_mode)
            config_keys = [mapped, "default"] if mapped else ["default"]
        # LeagueAkari stores runes and spells independently under the same
        # champion/mode page.  Prefer that canonical shape and only fall back
        # to the pre-1:1 flat loadout list for settings imported from older
        # Insight builds.
        rune_pages = self.settings.runes_v2.get(champion_id) or self.settings.runes_v2.get(str(champion_id)) or {}
        spell_pages = self.settings.summoner_spells.get(champion_id) or self.settings.summoner_spells.get(str(champion_id)) or {}
        rune_key = next((key for key in config_keys if rune_pages.get(key)), None)
        spell_key = next((key for key in config_keys if spell_pages.get(key)), None)
        rune_config = rune_pages.get(rune_key) if rune_key else None
        spell_config = spell_pages.get(spell_key) if spell_key else None
        legacy = next(
            (item for key in config_keys for item in self.settings.champion_loadouts if item.champion_id == champion_id and item.config_key == key),
            None,
        )
        if rune_config is None and legacy is not None:
            rune_config = ChampionRunesConfig(
                primaryStyleId=legacy.primary_style_id,
                subStyleId=legacy.sub_style_id,
                selectedPerkIds=legacy.selected_perk_ids,
            )
            rune_key = legacy.config_key
        if spell_config is None and legacy is not None:
            spell_config = SummonerSpellsConfig(spell1Id=legacy.spell1_id, spell2Id=legacy.spell2_id)
        if rune_config is None and spell_config is None:
            return
        applied_any = False
        if spell_config is not None:
            try:
                await self.request("PATCH", "/lol-champ-select/v1/session/my-selection", json_body={
                    "spell1Id": spell_config.spell1_id,
                    "spell2Id": spell_config.spell2_id,
                })
                applied_any = True
                await self._send_champion_config_feedback(
                    champion_id,
                    f"召唤师技能已应用（{spell_config.spell1_id}/{spell_config.spell2_id}）",
                )
            except RuntimeError as exc:
                await self._send_champion_config_feedback(
                    champion_id,
                    f"召唤师技能应用失败：{type(exc).__name__}",
                )
        if rune_config is not None:
            try:
                pages = await self.request("GET", "/lol-perks/v1/pages")
                editable = next((page for page in (pages or []) if isinstance(page, dict) and page.get("isEditable")), None)
                page_body = {
                    "name": f"[Insight] Champion {champion_id} - {rune_key or config_keys[0]}",
                    "primaryStyleId": rune_config.primary_style_id,
                    "subStyleId": rune_config.sub_style_id,
                    "selectedPerkIds": rune_config.selected_perk_ids,
                    "current": True,
                }
                if editable and editable.get("id") is not None:
                    page_id = editable["id"]
                    await self.request("PUT", f"/lol-perks/v1/pages/{page_id}", json_body=page_body)
                else:
                    created = await self.request("POST", "/lol-perks/v1/pages", json_body=page_body)
                    page_id = created.get("id") if isinstance(created, dict) else None
                if page_id is None:
                    raise RuntimeError("LCU 未返回可用符文页")
                await self.request("PUT", "/lol-perks/v1/currentpage", json_body=page_id)
                applied_any = True
                await self._send_champion_config_feedback(
                    champion_id,
                    f"符文页已应用（{rune_key or config_keys[0]}）",
                )
            except RuntimeError as exc:
                await self._send_champion_config_feedback(
                    champion_id,
                    f"符文页应用失败：{type(exc).__name__}",
                )
        if applied_any:
            self._configured_champion_id = champion_id
            self.last_action = f"已应用英雄 {champion_id} 的符文与召唤师技能"
            self.last_action_at = time.time()

    async def _run_auto_honor(self, ballot: dict | None = None) -> None:
        """Submit the end-of-game honor ballot once, then let play-again run.

        LeagueAkari watches the ballot itself, not only ``EndOfGame``.  In
        particular, Tencent/current client builds can expose the ballot while
        the gameflow phase is still ``WaitingForStats``.  The polling path
        keeps the phase guard, while the event path can pass the already
        received ballot as ``ballot``.

        The v2 ballot GET is optional on builds without honor support.  A 404
        must not abort the rest of ``_run_automation`` because doing so also
        prevents the independent play-again action from running.
        """
        if (
            self.phase not in {"WaitingForStats", "PreEndOfGame", "EndOfGame"}
            or not self.settings.automation_enabled
            or not self.settings.auto_honor_enabled
        ):
            return
        if ballot is None:
            try:
                ballot = await self.request("GET", "/lol-honor-v2/v1/ballot/")
            except RuntimeError:
                # This endpoint is absent on some client builds.  Treat it as
                # an unavailable optional snapshot, exactly like LeagueAkari's
                # 404 handling, and leave play-again untouched.
                logger.debug("Honor ballot is unavailable", exc_info=True)
                return
        if not isinstance(ballot, dict):
            return
        game_id = str(ballot.get("gameId") or "")
        if (
            not game_id
            or game_id == self._honored_game_id
            or game_id == self._honor_in_progress_game_id
        ):
            return

        self._honor_in_progress_game_id = game_id
        try:
            votes = int((ballot.get("votePool") or {}).get("votes") or 0)
            strategy = self.settings.auto_honor_strategy
            if strategy == "opt-out":
                candidates: list[dict] = []
            else:
                allies = [
                    item
                    for item in (ballot.get("eligibleAllies") or [])
                    if isinstance(item, dict) and not item.get("botPlayer") and item.get("puuid")
                ]
                opponents = [
                    item
                    for item in (ballot.get("eligibleOpponents") or [])
                    if isinstance(item, dict) and not item.get("botPlayer") and item.get("puuid")
                ]
                lobby_puuids: set[str] = set()
                try:
                    eog = await self.request("GET", "/lol-lobby/v2/eog-status")
                    for key in ("eogPlayers", "leftPlayers", "readyPlayers"):
                        lobby_puuids.update(str(value) for value in ((eog or {}).get(key) or []))
                except RuntimeError:
                    pass
                lobby_allies = [item for item in allies if str(item.get("puuid")) in lobby_puuids]
                other_allies = [item for item in allies if str(item.get("puuid")) not in lobby_puuids]
                if strategy == "only-lobby-member":
                    pools = (lobby_allies,)
                elif strategy == "all-member":
                    pools = (lobby_allies, other_allies)
                else:
                    # This is also the LeagueAkari default: lobby allies,
                    # other allies, then opponents, in that priority order.
                    pools = (lobby_allies, other_allies, opponents)

                candidates = []
                remaining_votes = max(votes, 0)
                for pool in pools:
                    if remaining_votes <= 0:
                        break
                    selected = random.sample(pool, min(remaining_votes, len(pool))) if pool else []
                    candidates.extend(selected)
                    remaining_votes -= len(selected)

            # Submit every selected honor, but always finish the ballot even
            # when an individual legacy honor write fails.  A stuck ballot is
            # what prevents the client from advancing and was the main reason
            # the subsequent play-again action appeared to do nothing.
            honor_error: RuntimeError | None = None
            for player in candidates:
                try:
                    await self.request(
                        "POST",
                        "/lol-honor/v1/honor",
                        json_body={
                            "honorType": "HEART",
                            "recipientPuuid": player.get("puuid"),
                        },
                    )
                except RuntimeError as exc:
                    honor_error = honor_error or exc
                    logger.warning("Unable to submit one honor vote", exc_info=True)
            try:
                await self.request("POST", "/lol-honor/v1/ballot")
            except RuntimeError:
                # Do not mark this game handled when the ballot cannot be
                # finished; the next event/poll can retry safely.
                logger.warning("Unable to finish honor ballot", exc_info=True)
                return

            self._honored_game_id = game_id
            if strategy == "opt-out":
                self.last_action = "已按设置跳过点赞"
            elif honor_error is not None:
                self.last_action = "自动点赞部分失败，已结束点赞流程"
            elif candidates:
                self.last_action = "已自动点赞队友"
            else:
                self.last_action = "已自动结束点赞流程"
            self.last_action_at = time.time()

            # A concrete ballot temporarily pauses play-again in LeagueAkari
            # so honor writes cannot race it.  Re-arm the independent action
            # immediately after the ballot is acknowledged; otherwise a
            # previous deadline cleared here would make return-to-lobby never
            # run when both switches are enabled.
            if (
                self.settings.play_again_enabled
                and self.phase in {"WaitingForStats", "PreEndOfGame", "EndOfGame"}
                and self._phase_action_done != "play-again"
            ):
                self._phase_action_due_at = time.monotonic() + 0.25
        finally:
            if self._honor_in_progress_game_id == game_id:
                self._honor_in_progress_game_id = ""

    async def _send_matchmaking_chat(self, message: str, *, key: str | None = None) -> bool:
        """Send a deduplicated matchmaking notice to the custom-game chat."""

        if not self.settings.auto_matchmaking_chat_countdown_enabled:
            return False
        if key and key == self._matchmaking_last_chat_key:
            return False
        sent = await self._send_lcu_phase_chat(message, phase="Lobby", message_type="celebration")
        if sent and key:
            self._matchmaking_last_chat_key = key
        return sent

    async def _notify_matchmaking_cancel(self, reason: str) -> None:
        """Expose the same cancellation reasons as LeagueAkari's controller."""

        messages = {
            "normal": "自动匹配已取消",
            "waiting-for-invitees": "自动匹配已取消，正在等待邀请的好友",
            # Keep the upstream singular reason accepted by callers while
            # the public status uses the more descriptive plural form.
            "waiting-for-invitee": "自动匹配已取消，正在等待邀请的好友",
            "not-the-leader": "自动匹配已取消，你不是房主",
            "waiting-for-penalty-time": "自动匹配已取消，等待惩罚计时结束",
            "insufficient-members": "自动匹配已取消，房间人数不足",
            "cannot-start-activity": "自动匹配已取消，当前房间无法开始该队列",
            "unsupported-lobby": "自动匹配已取消，当前房间不支持自动匹配",
            "lobby-unavailable": "自动匹配已取消，房间状态暂不可用",
            "rematch": "自动匹配已按重排策略取消",
        }
        await self._send_matchmaking_chat(
            messages.get(reason, f"自动匹配已取消：{reason}"),
            key=f"cancel:{reason}",
        )

    async def cancel_auto_matchmaking(self, reason: str = "normal") -> None:
        """Cancel only the local auto-matchmaking plan (never the LCU queue)."""

        had_plan = self._matchmaking_due_at is not None or self._matchmaking_status in {
            "countdown", "waiting-for-invitees", "not-leader", "insufficient-members",
        }
        self._matchmaking_due_at = None
        self._matchmaking_chat_countdown_second = None
        self._matchmaking_status = "idle"
        self._matchmaking_status_reason = reason
        if had_plan:
            await self._notify_matchmaking_cancel(reason)

    async def _run_auto_matchmaking(self) -> None:
        settings = self.settings
        had_local_plan = self._matchmaking_due_at is not None or self._matchmaking_status in {
            "countdown",
            "waiting-for-invitees",
            "not-leader",
            "insufficient-members",
            "waiting-for-penalty",
            "cannot-start",
        }
        if (
            not settings.automation_enabled
            or not settings.auto_matchmaking_enabled
            or self.phase not in {"Lobby", "Matchmaking"}
        ):
            self._matchmaking_due_at = None
            self._matchmaking_status = "idle"
            self._matchmaking_status_reason = None
            self._matchmaking_chat_countdown_second = None
            return
        try:
            lobby = await self.request("GET", "/lol-lobby/v2/lobby")
        except RuntimeError:
            self._matchmaking_status = "lobby-unavailable"
            self._matchmaking_status_reason = "lobby-unavailable"
            self._matchmaking_due_at = None
            self._matchmaking_chat_countdown_second = None
            if had_local_plan:
                await self._notify_matchmaking_cancel("lobby-unavailable")
            return
        if not isinstance(lobby, dict) or (lobby.get("gameConfig") or {}).get("isCustom"):
            self._matchmaking_status = "unsupported-lobby"
            self._matchmaking_status_reason = "unsupported-lobby"
            self._matchmaking_due_at = None
            if had_local_plan:
                await self._notify_matchmaking_cancel("unsupported-lobby")
            return
        local = lobby.get("localMember") or {}
        members = lobby.get("members") or []
        if not local.get("isLeader"):
            self._matchmaking_status = "not-leader"
            self._matchmaking_status_reason = "not-the-leader"
            self._matchmaking_due_at = None
            if had_local_plan:
                await self._notify_matchmaking_cancel("not-the-leader")
            return
        if len(members) < settings.auto_matchmaking_minimum_members:
            self._matchmaking_status = "insufficient-members"
            self._matchmaking_status_reason = "insufficient-members"
            self._matchmaking_due_at = None
            if had_local_plan:
                await self._notify_matchmaking_cancel("insufficient-members")
            return
        if settings.auto_matchmaking_wait_for_invitees and any(item.get("state") == "Pending" for item in (lobby.get("invitations") or [])):
            self._matchmaking_status = "waiting-for-invitees"
            self._matchmaking_status_reason = "waiting-for-invitees"
            self._matchmaking_due_at = None
            if had_local_plan:
                await self._notify_matchmaking_cancel("waiting-for-invitees")
            return
        try:
            search = await self.request("GET", "/lol-matchmaking/v1/search")
        except RuntimeError:
            search = None
        self.matchmaking_search = _normalize_matchmaking_search(search)
        if isinstance(search, dict) and (search.get("isCurrentlyInQueue") or search.get("searchState") == "Searching"):
            self._matchmaking_status = "searching"
            self._matchmaking_status_reason = None
            penalty = float(((search.get("lowPriorityData") or {}).get("penaltyTime")) or 0)
            elapsed = max(0.0, float(search.get("timeInQueue") or 0) - penalty)
            limit = settings.auto_matchmaking_rematch_fixed_duration
            if settings.auto_matchmaking_rematch_strategy == "estimated-duration":
                limit = float(search.get("estimatedQueueTime") or 0)
            if settings.auto_matchmaking_rematch_strategy != "never" and limit > 0 and elapsed >= limit:
                await self._record_action("已按重排策略取消匹配", "DELETE", "/lol-lobby/v2/lobby/matchmaking/search")
                self._matchmaking_status = "rematch-cancelled"
                self._matchmaking_status_reason = "rematch"
                await self._notify_matchmaking_cancel("rematch")
            return
        errors = ((search or {}).get("errors") or []) if isinstance(search, dict) else []
        low_priority = ((search or {}).get("lowPriorityData") or {}) if isinstance(search, dict) else {}
        if any(
            float(item.get("penaltyTimeRemaining") or 0) > 0
            for item in errors
            if isinstance(item, dict)
        ) or float(low_priority.get("penaltyTimeRemaining") or 0) > 0 or float(low_priority.get("penaltyTime") or 0) > 0:
            self._matchmaking_status = "waiting-for-penalty"
            self._matchmaking_status_reason = "waiting-for-penalty-time"
            self._matchmaking_due_at = None
            if had_local_plan:
                await self._notify_matchmaking_cancel("waiting-for-penalty-time")
            return
        if not lobby.get("canStartActivity", True):
            self._matchmaking_status = "cannot-start"
            self._matchmaking_status_reason = "cannot-start-activity"
            self._matchmaking_due_at = None
            if had_local_plan:
                await self._notify_matchmaking_cancel("cannot-start-activity")
            return
        if self._matchmaking_due_at is None:
            self._matchmaking_due_at = time.monotonic() + settings.auto_matchmaking_delay_seconds
            self._matchmaking_status = "countdown"
            self._matchmaking_status_reason = None
            self._matchmaking_last_chat_key = None
        if settings.auto_matchmaking_chat_countdown_enabled:
            remaining = max(0, int(math.ceil(self._matchmaking_due_at - time.monotonic())))
            if remaining != self._matchmaking_chat_countdown_second:
                sent = await self._send_matchmaking_chat(
                    f"将在 {remaining} 秒后自动开始匹配",
                    key=f"countdown:{remaining}",
                )
                if sent:
                    self._matchmaking_chat_countdown_second = remaining
        if time.monotonic() >= self._matchmaking_due_at:
            await self._record_action("已自动开始匹配", "POST", "/lol-lobby/v2/lobby/matchmaking/search")
            self._matchmaking_due_at = None
            self._matchmaking_chat_countdown_second = None
            self._matchmaking_status = "searching"
            self._matchmaking_status_reason = None

    async def _run_aram_team_side(self) -> None:
        if (
            not self.settings.automation_enabled
            or not self.settings.auto_send_aram_team_side_enabled
            or self.phase != "ChampSelect"
        ):
            if self.phase != "ChampSelect":
                self._aram_side_sent_context = ""
            return
        try:
            session = await self.request("GET", "/lol-champ-select/v1/session")
            gameflow = await self.request("GET", "/lol-gameflow/v1/session")
            conversations = await self.request("GET", "/lol-chat/v1/conversations")
        except RuntimeError:
            return
        if not isinstance(session, dict) or not session.get("benchEnabled"):
            return
        game_mode = str(((gameflow or {}).get("map") or {}).get("gameMode") or "").upper()
        if game_mode not in {"ARAM", "KIWI"}:
            return
        local_cell = session.get("localPlayerCellId")
        member = next((row for row in (session.get("myTeam") or []) if row.get("cellId") == local_cell), {})
        team = int(member.get("team") or 0)
        if team not in {1, 2}:
            return
        conversation_rows = conversations if isinstance(conversations, list) else []
        conversation = next(
            (
                row
                for row in conversation_rows
                if isinstance(row, dict) and str(row.get("type") or "").lower() in {"championselect", "champion-select"}
            ),
            None,
        )
        if not conversation or not conversation.get("id"):
            return
        context = f"{conversation['id']}:{team}"
        if context == self._aram_side_sent_context:
            return
        labels = {1: "本局位于左侧（蓝方）", 2: "本局位于右侧（红方）"}
        body = labels[team]
        await self.request(
            "POST",
            f"/lol-chat/v1/conversations/{conversation['id']}/messages",
            json_body={
                "body": body if self.settings.auto_send_aram_team_side_visible_to_team else f"[Insight] {body}",
                "type": "chat" if self.settings.auto_send_aram_team_side_visible_to_team else "celebration",
            },
        )
        self._aram_side_sent_context = context
        self.last_action = f"已发送大乱斗阵营：{labels[team]}"
        self.last_action_at = time.time()

    async def _run_lobby_automation(self) -> None:
        settings = self.settings
        if not settings.automation_enabled:
            return
        if settings.auto_skip_leader_enabled and self.phase == "Lobby":
            try:
                lobby = await self.request("GET", "/lol-lobby/v2/lobby")
            except RuntimeError:
                lobby = None
            if isinstance(lobby, dict):
                local_member = lobby.get("localMember") or {}
                lobby_id = str(lobby.get("partyId") or lobby.get("gameConfig", {}).get("gameId") or "lobby")
                if local_member.get("isLeader") and self._leader_handoff_lobby != lobby_id:
                    others = [member for member in (lobby.get("members") or []) if member.get("summonerId") != local_member.get("summonerId") and not member.get("isSpectator")]
                    ready = [member for member in others if member.get("ready")]
                    # LeagueAkari chooses a ready teammate first and randomizes
                    # within that tier so the same member is not always made
                    # leader when several invitees are available.
                    pool = ready or others
                    candidate = random.choice(pool) if pool else None
                    if candidate and candidate.get("summonerId"):
                        await self._record_action("已自动转交房主", "POST", f"/lol-lobby/v2/lobby/members/{candidate['summonerId']}/promote")
                        self._leader_handoff_lobby = lobby_id

        # LeagueAkari reacts to received invitations independently of the
        # current gameflow phase; only leader handoff is Lobby-specific.
        if not settings.auto_handle_invitations_enabled:
            return
        if settings.reject_invitation_when_away:
            try:
                chat_me = await self.request("GET", "/lol-chat/v1/me")
            except RuntimeError:
                chat_me = {}
            if isinstance(chat_me, dict) and chat_me.get("availability") == "away":
                return
        invitations = await self.request("GET", "/lol-lobby/v2/received-invitations")
        candidates = []
        for invitation in invitations if isinstance(invitations, list) else []:
            if not isinstance(invitation, dict) or invitation.get("state") != "Pending" or invitation.get("canAcceptInvitation") is False:
                continue
            invite_id = str(invitation.get("invitationId") or "")
            if not invite_id or invite_id in self._handled_invitations:
                continue
            invite_type = str((invitation.get("gameConfig") or {}).get("inviteGameType") or "<DEFAULT>")
            strategies = settings.invitation_handling_strategies
            action = strategies.get(invite_type) or strategies.get("<DEFAULT>") or settings.invitation_strategy
            candidates.append((0 if action == "accept" else 1 if action == "decline" else 2, invite_id, action, invite_type))
        if not candidates:
            return
        _, invite_id, action, invite_type = sorted(candidates)[0]
        if action == "ignore":
            return
        await self._record_action(
            f"已自动{'接受' if action == 'accept' else '拒绝'} {invite_type} 房间邀请",
            "POST", f"/lol-lobby/v2/received-invitations/{invite_id}/{action}",
        )
        self._handled_invitations.add(invite_id)

    async def apply_status_message(self, message: str, *, automated: bool = False) -> None:
        if automated and (
            not self.settings.automation_enabled
            or not self.settings.auto_set_status_message_enabled
        ):
            return
        if not automated:
            self._interrupt_chat_ready_automation()
        await self.request("PUT", "/lol-chat/v1/me", json_body={"statusMessage": message})
        self.last_action = "已自动恢复聊天状态签名" if automated else "已应用聊天状态签名"
        self.last_action_at = time.time()

    async def apply_ranked_status(self, ranked_status: RankedStatusUpdate, *, automated: bool = False) -> None:
        if automated and (
            not self.settings.automation_enabled
            or not self.settings.auto_set_ranked_status_enabled
        ):
            return
        if not automated:
            self._interrupt_chat_ready_automation()
        ranked = ranked_status.model_dump()
        if ranked_status.tier in {"MASTER", "GRANDMASTER", "CHALLENGER"}:
            ranked.pop("division", None)
        await self.request("PUT", "/lol-chat/v1/me", json_body={
            "lol": {
                "rankedLeagueQueue": ranked["queue"],
                "rankedLeagueTier": ranked["tier"],
                **({"rankedLeagueDivision": ranked["division"]} if "division" in ranked else {}),
            }
        })
        self.last_action = "已自动恢复排位展示" if automated else "已应用排位展示"
        self.last_action_at = time.time()

    async def _run_chat_ready_automation(self) -> None:
        """Apply LeagueAkari login automations once, after `/lol-chat/v1/me` settles for two seconds."""
        if self.credentials is None:
            self._chat_ready_since = None
            return
        if self._chat_ready_automation_done:
            return
        try:
            chat_me = await self.request("GET", "/lol-chat/v1/me")
        except RuntimeError:
            self._chat_ready_since = None
            return
        if not isinstance(chat_me, dict) or not chat_me:
            self._chat_ready_since = None
            return
        now = time.monotonic()
        if self._chat_ready_since is None:
            self._chat_ready_since = now
            return
        if now - self._chat_ready_since < 2.0:
            return
        self._chat_ready_automation_done = True
        if not self.settings.automation_enabled:
            return
        results = []
        if self.settings.auto_set_status_message_enabled:
            results.append(self.apply_status_message(self.settings.status_message, automated=True))
        if self.settings.auto_set_ranked_status_enabled:
            results.append(self.apply_ranked_status(self.settings.ranked_status, automated=True))
        if results:
            await asyncio.gather(*results, return_exceptions=True)

    async def _run_automation(self) -> None:
        settings = self.settings
        phase = self.phase
        self._observe_auto_accept_phase(phase)
        await self._run_chat_ready_automation()
        if not settings.automation_enabled:
            self._cancel_auto_accept_waiter()
            self._accept_due_at = None
            self._phase_action_due_at = None
            self._matchmaking_due_at = None
            self._matchmaking_status = "idle"
            self._matchmaking_status_reason = None
            self._matchmaking_chat_countdown_second = None
            self._matchmaking_last_chat_key = None
            self._champion_action_due_at.clear()
            self._delayed_action_plan.clear()
            self._trade_created_at.clear()
            self._phase_action_done = ""
            self._acted_phase = ""
            return

        if phase != self._acted_phase:
            self._acted_phase = phase
            self._phase_action_done = ""
            if not self._ready_check_can_accept():
                self._cancel_auto_accept_waiter()
                self._accept_due_at = None
            delay_by_phase = {
                "WaitingForStats": 10.0,
                "PreEndOfGame": 3.25,
                "EndOfGame": 1.575,
                "Reconnect": 10.0,
            }
            delay = delay_by_phase.get(phase)
            self._phase_action_due_at = time.monotonic() + delay if delay is not None else None

        if self._ready_check_can_accept() and settings.auto_accept_enabled:
            ready_check = self.ready_check or {}
            self._observe_ready_check_identity(ready_check)
            player_response = str(
                ready_check.get("player_response") or ready_check.get("playerResponse") or ""
            ).upper()
            if player_response in {"ACCEPTED", "DECLINED"}:
                self._auto_accept_terminal_response = True
                self._accept_due_at = None
            else:
                await self._ensure_auto_accept_scheduled()
        else:
            self._accept_due_at = None
            self._cancel_auto_accept_waiter()

        if phase == "ChampSelect":
            if settings.auto_select_enabled:
                await self._run_auto_select()
            if settings.auto_champion_config_enabled:
                await self._run_champion_config()
        else:
            self._handled_champion_actions.clear()
            self._champion_action_due_at.clear()
            self._delayed_action_plan.clear()
            self._handled_trades.clear()
            self._trade_created_at.clear()
            self._bench_candidate_since.clear()
            self._configured_champion_id = 0
            self._active_auto_select_group = "default"
            self._active_auto_select_position = "default"
            self._active_auto_select_queue_type = ""

        if phase in {"WaitingForStats", "PreEndOfGame", "EndOfGame"} and settings.auto_honor_enabled:
            # LeagueAkari watches the ballot independently of the exact
            # post-game phase.  WaitingForStats is a common phase while the
            # ballot is already actionable on current clients.
            await self._run_auto_honor()

        if phase in {"EndOfGame", "WaitingForStats", "PreEndOfGame"} and settings.play_again_enabled:
            if (
                self._phase_action_due_at is not None
                and self._phase_action_done != "play-again"
                and time.monotonic() >= self._phase_action_due_at
            ):
                await self._record_action("已自动返回房间", "POST", "/lol-lobby/v2/play-again")
                self._phase_action_done = "play-again"
        elif phase == "Reconnect" and settings.auto_reconnect_enabled:
            if (
                self._phase_action_due_at is not None
                and self._phase_action_done != "reconnect"
                and time.monotonic() >= self._phase_action_due_at
            ):
                await self._record_action("已自动重新连接", "POST", "/lol-gameflow/v1/reconnect")
                self._phase_action_done = "reconnect"

        await self._run_auto_matchmaking()
        await self._run_aram_team_side()
        await self._run_lobby_automation()

    async def _run(self) -> None:
        while True:
            try:
                if await self.refresh_connection():
                    await self._refresh_state()
                    await self._refresh_respawn_timer()
                    await self._run_automation()
            except asyncio.CancelledError:
                raise
            except Exception:
                logger.exception("League lab background loop failed")
            chat_settling = self.credentials and not self._chat_ready_automation_done and self._chat_ready_since is not None
            timeout = 1.0 if chat_settling or (self.credentials and self.settings.respawn_timer_enabled and self.phase == "InProgress") else 5.0
            # LeagueAkari starts a real timeout task when ReadyCheck begins.
            # Keep the polling fallback event-driven, but do not let its
            # five-second idle interval postpone a configured auto-accept
            # deadline (especially for sub-five-second delays).
            accept_due_at = self._accept_due_at
            if (
                self.phase == "ReadyCheck"
                and self.settings.automation_enabled
                and self.settings.auto_accept_enabled
                and accept_due_at is not None
                and accept_due_at != float("inf")
            ):
                timeout = min(timeout, max(0.0, accept_due_at - time.monotonic()))
            phase_due_at = self._phase_action_due_at
            if (
                phase_due_at is not None
                and phase_due_at != float("inf")
                and self._phase_action_done not in {"play-again", "reconnect"}
                and (
                    (self.phase in {"WaitingForStats", "PreEndOfGame", "EndOfGame"} and self.settings.play_again_enabled)
                    or (self.phase == "Reconnect" and self.settings.auto_reconnect_enabled)
                )
            ):
                timeout = min(timeout, max(0.0, phase_due_at - time.monotonic()))
            try:
                await asyncio.wait_for(self._event_wakeup.wait(), timeout=timeout)
                self._event_wakeup.clear()
            except asyncio.TimeoutError:
                pass


league_lab_service = LeagueLabService()


def _normalize_skin_selector(info, rows) -> dict:
    options = []
    for skin in rows or []:
        if not isinstance(skin, dict) or skin.get("disabled") or not skin.get("unlocked"):
            continue
        options.append({"id": int(skin.get("id") or 0), "name": skin.get("name") or str(skin.get("id")), "preview_path": skin.get("splashPath") or skin.get("tilePath") or "", "is_chroma": False})
        for child in skin.get("childSkins") or []:
            if isinstance(child, dict) and not child.get("disabled") and child.get("unlocked"):
                options.append({"id": int(child.get("id") or 0), "name": child.get("name") or str(child.get("id")), "preview_path": child.get("chromaPreviewPath") or child.get("splashPath") or "", "is_chroma": True})
    return {
        "available": bool(options) and bool((info or {}).get("showSkinSelector", True)),
        "disabled": bool((info or {}).get("skinSelectionDisabled")),
        "selected_skin_id": int((info or {}).get("selectedSkinId") or 0),
        "champion_id": int((info or {}).get("selectedChampionId") or 0),
        "skins": [row for row in options if row["id"] > 0],
    }


_READY_CHECK_ROUND_ID_KEYS = (
    "readyCheckId",
    "ready_check_id",
    "matchId",
    "match_id",
    "gameId",
    "game_id",
    "sessionId",
    "session_id",
    "id",
)


def _ready_check_round_identity(payload) -> str | None:
    """Extract a stable per-ballot id when an LCU build exposes one."""
    if not isinstance(payload, dict):
        return None
    for key in _READY_CHECK_ROUND_ID_KEYS:
        value = payload.get(key)
        if isinstance(value, (dict, list, tuple, set)):
            continue
        text = str(value or "").strip()
        if text:
            return f"{key}:{text}"
    return None


def _normalize_ready_check(payload) -> dict | None:
    """Keep useful, read-only ReadyCheck evidence in a stable shape."""
    if not isinstance(payload, dict):
        return None
    state = str(payload.get("state") or "")
    player_response = str(payload.get("playerResponse") or payload.get("player_response") or "")
    timer = payload.get("timer")
    normalized_timer = None
    if isinstance(timer, dict):
        normalized_timer = {
            "type": timer.get("type"),
            "total_seconds": timer.get("totalSeconds"),
            "remaining_seconds": timer.get("remainingSeconds"),
        }
    state_upper = state.upper()
    response_upper = player_response.upper()
    available = state_upper in {"INPROGRESS", "PENDING", "READY"}
    can_accept = available and response_upper != "ACCEPTED"
    can_decline = available and response_upper != "DECLINED"
    return {
        "round_id": _ready_check_round_identity(payload),
        "state": state,
        "player_response": player_response,
        "response_state": response_upper.lower() or "pending",
        "decliner_ids": [value for value in (payload.get("declinerIds") or payload.get("decliner_ids") or [])],
        "dodge_warning": payload.get("dodgeWarning") or payload.get("dodge_warning"),
        "suppress_ux": bool(payload.get("suppressUx") if "suppressUx" in payload else payload.get("suppress_ux")),
        "timer": normalized_timer,
        "available": available,
        # LeagueAkari lets the user reverse an already submitted response while
        # the ReadyCheck is still active: Accepted can decline, Declined can
        # accept again.  The endpoint still revalidates the live phase.
        "can_accept": can_accept,
        "can_decline": can_decline,
        "actionability": {"accept": can_accept, "decline": can_decline},
    }


def _normalize_matchmaking_search(payload) -> dict | None:
    """Expose queue/search evidence without coupling the UI to LCU casing."""
    if not isinstance(payload, dict):
        return None
    low_priority = payload.get("lowPriorityData") or payload.get("low_priority_data") or {}
    errors = []
    for item in payload.get("errors") or []:
        if not isinstance(item, dict):
            continue
        errors.append({
            "code": item.get("code"),
            "message": item.get("message"),
            "penalty_time_remaining": item.get("penaltyTimeRemaining"),
            "penalty_time": item.get("penaltyTime"),
        })
    return {
        "is_currently_in_queue": bool(payload.get("isCurrentlyInQueue") if "isCurrentlyInQueue" in payload else payload.get("is_currently_in_queue")),
        "search_state": str(payload.get("searchState") or payload.get("search_state") or ""),
        "time_in_queue": payload.get("timeInQueue") if "timeInQueue" in payload else payload.get("time_in_queue"),
        "estimated_queue_time": payload.get("estimatedQueueTime") if "estimatedQueueTime" in payload else payload.get("estimated_queue_time"),
        "queue_id": payload.get("queueId") if "queueId" in payload else payload.get("queue_id"),
        "lobby_id": payload.get("lobbyId") if "lobbyId" in payload else payload.get("lobby_id"),
        "errors": errors,
        "low_priority_data": {
            "penalty_time": low_priority.get("penaltyTime"),
            "penalty_time_remaining": low_priority.get("penaltyTimeRemaining"),
        },
        "ready_check": _normalize_ready_check(payload.get("readyCheck") or payload.get("ready_check")),
    }


# LeagueAkari's built-in SGP routing table, reduced to the match-history hosts used here.
# Tokens are fetched from the local LCU on demand and are never written to disk or returned to the UI.
_SGP_MATCH_HISTORY_HOSTS = {
    "TENCENT_HN1": "https://hn1-k8s-sgp.lol.qq.com:21019",
    "TENCENT_HN10": "https://hn10-k8s-sgp.lol.qq.com:21019",
    "TENCENT_TJ100": "https://tj100-sgp.lol.qq.com:21019",
    "TENCENT_TJ101": "https://tj101-sgp.lol.qq.com:21019",
    "TENCENT_NJ100": "https://nj100-sgp.lol.qq.com:21019",
    "TENCENT_GZ100": "https://gz100-sgp.lol.qq.com:21019",
    "TENCENT_CQ100": "https://cq100-sgp.lol.qq.com:21019",
    "TENCENT_BGP2": "https://bgp2-k8s-sgp.lol.qq.com:21019",
    "TENCENT_PBE": "https://pbe-sgp.lol.qq.com:21019",
    "TENCENT_PREPBE": "https://prepbe-sgp.lol.qq.com:21019",
    "TW2": "https://apse1-red.pp.sgp.pvp.net",
    "SG2": "https://apse1-red.pp.sgp.pvp.net",
    "PH2": "https://apse1-red.pp.sgp.pvp.net",
    "VN2": "https://apse1-red.pp.sgp.pvp.net",
    "PBE": "https://usw2-red.pp.sgp.pvp.net",
    "EUW": "https://euc1-red.pp.sgp.pvp.net",
    "JP": "https://apne1-red.pp.sgp.pvp.net",
    "RU": "https://euc1-red.pp.sgp.pvp.net",
    "BR1": "https://usw2-red.pp.sgp.pvp.net",
    "OC1": "https://apse1-red.pp.sgp.pvp.net",
    "TR1": "https://euc1-red.pp.sgp.pvp.net",
    "LA1": "https://usw2-red.pp.sgp.pvp.net",
    "LA2": "https://usw2-red.pp.sgp.pvp.net",
    "NA1": "https://usw2-red.pp.sgp.pvp.net",
    "TH2": "https://apse1-red.pp.sgp.pvp.net",
    "KR": "https://apne1-red.pp.sgp.pvp.net",
}

_SGP_COMMON_HOSTS = {
    **{key: value for key, value in _SGP_MATCH_HISTORY_HOSTS.items() if key.startswith("TENCENT_")},
    "TW2": "https://tw2-red.lol.sgp.pvp.net",
    "SG2": "https://sg2-red.lol.sgp.pvp.net",
    "PH2": "https://ph2-red.lol.sgp.pvp.net",
    "VN2": "https://vn2-red.lol.sgp.pvp.net",
    "PBE": "https://pbe-red.lol.sgp.pvp.net",
    "EUW": "https://euw-red.lol.sgp.pvp.net",
    "JP": "https://jp-red.lol.sgp.pvp.net",
    "RU": "https://ru-red.lol.sgp.pvp.net",
    "BR1": "https://br-red.lol.sgp.pvp.net",
    "OC1": "https://oce-red.lol.sgp.pvp.net",
    "TR1": "https://tr-red.lol.sgp.pvp.net",
    "LA1": "https://lan-red.lol.sgp.pvp.net",
    "LA2": "https://las-red.lol.sgp.pvp.net",
    "NA1": "https://na-red.lol.sgp.pvp.net",
    "TH2": "https://th2-red.lol.sgp.pvp.net",
    "KR": "https://kr-red.lol.sgp.pvp.net",
}

_SGP_SERVER_LABELS = {
    "TENCENT_HN1": "艾欧尼亚",
    "TENCENT_HN10": "黑色玫瑰",
    "TENCENT_TJ100": "峡谷之巅",
    "TENCENT_TJ101": "联盟一区",
    "TENCENT_NJ100": "联盟二区",
    "TENCENT_GZ100": "联盟三区",
    "TENCENT_CQ100": "联盟四区",
    "TENCENT_BGP2": "男爵领域",
    "TENCENT_PBE": "国服体验服",
    "TENCENT_PREPBE": "国服预发布服",
    "TW2": "中国台湾",
    "SG2": "新加坡",
    "PH2": "菲律宾",
    "VN2": "越南",
    "PBE": "PBE",
    "EUW": "欧洲西部",
    "JP": "日本",
    "RU": "俄罗斯",
    "BR1": "巴西",
    "OC1": "大洋洲",
    "TR1": "土耳其",
    "LA1": "拉丁美洲北部",
    "LA2": "拉丁美洲南部",
    "NA1": "北美",
    "TH2": "泰国",
    "KR": "韩国",
}


def _sgp_server_id(credentials: LcuCredentials | None) -> str:
    if not credentials:
        return ""
    region = credentials.region.upper()
    platform = credentials.platform_id.upper()
    if region in {"CN", "TENCENT"}:
        return f"TENCENT_{platform}" if platform else ""
    aliases = {"NA": "NA1", "BR": "BR1", "TR": "TR1", "LAN": "LA1", "LAS": "LA2", "OCE": "OC1", "EUW1": "EUW", "JP1": "JP"}
    return aliases.get(region, region)


def _normalize_sgp_server_id(server_id: str | None) -> str:
    value = str(server_id or "").strip().upper()
    aliases = {"EUW1": "EUW", "JP1": "JP", "NA": "NA1", "BR": "BR1", "TR": "TR1", "LAN": "LA1", "LAS": "LA2", "OCE": "OC1"}
    value = aliases.get(value, value)
    if value not in _SGP_COMMON_HOSTS or value not in _SGP_MATCH_HISTORY_HOSTS:
        raise RuntimeError(f"不支持的 SGP 区服: {value or '空'}")
    return value


def _sgp_region_path(credentials: LcuCredentials | None = None, server_id: str | None = None) -> str:
    server_id = _normalize_sgp_server_id(server_id) if server_id else _sgp_server_id(credentials)
    if server_id.startswith("TENCENT_"):
        return server_id.split("_", 1)[1]
    aliases = {"PBE": "PBE1", "EUW": "EUW1", "JP": "JP1"}
    return aliases.get(server_id, server_id)


async def _sgp_match_history(
    puuid: str,
    beg_index: int,
    count: int,
    server_id: str | None = None,
    access_token: str | None = None,
) -> dict:
    credentials = league_lab_service.credentials
    server_id = _normalize_sgp_server_id(server_id) if server_id else _sgp_server_id(credentials)
    host = _SGP_MATCH_HISTORY_HOSTS.get(server_id)
    if not host:
        raise RuntimeError(f"当前区服不支持 SGP 战绩源: {server_id or '未知区服'}")
    token = access_token
    if not token:
        token_payload = await league_lab_service.request("GET", "/entitlements/v1/token")
        token = token_payload.get("accessToken") if isinstance(token_payload, dict) else None
    if not token:
        raise RuntimeError("LCU 未返回 SGP 授权令牌")
    url = f"{host}/match-history-query/v1/products/lol/player/{puuid}/SUMMARY"
    try:
        async with httpx.AsyncClient(timeout=8.0) as client:
            response = await client.get(
                url,
                headers={"Authorization": f"Bearer {token}"},
                params={"startIndex": beg_index, "count": count},
            )
        response.raise_for_status()
        payload = response.json()
    except (httpx.HTTPError, ValueError) as exc:
        raise RuntimeError(f"SGP 战绩请求失败: {type(exc).__name__}") from exc
    if not isinstance(payload, dict):
        raise RuntimeError("SGP 战绩返回格式无效")
    return payload


async def _sgp_game_details(game_id: int, server_id: str | None = None) -> dict:
    credentials = league_lab_service.credentials
    server_id = _normalize_sgp_server_id(server_id) if server_id else _sgp_server_id(credentials)
    host = _SGP_MATCH_HISTORY_HOSTS.get(server_id)
    if not host:
        raise RuntimeError(f"当前区服不支持 SGP 时间线源: {server_id or '未知区服'}")
    token_payload = await league_lab_service.request("GET", "/entitlements/v1/token")
    token = token_payload.get("accessToken") if isinstance(token_payload, dict) else None
    if not token:
        raise RuntimeError("LCU 未返回 SGP 授权令牌")
    region_path = _sgp_region_path(credentials, server_id)
    url = f"{host}/match-history-query/v1/products/lol/{region_path}_{int(game_id)}/DETAILS"
    try:
        async with httpx.AsyncClient(timeout=8.0) as client:
            response = await client.get(url, headers={"Authorization": f"Bearer {token}"})
        response.raise_for_status()
        payload = response.json()
    except (httpx.HTTPError, ValueError) as exc:
        raise RuntimeError(f"SGP 时间线请求失败: {type(exc).__name__}") from exc
    body = payload.get("json") if isinstance(payload, dict) and isinstance(payload.get("json"), dict) else payload
    if not isinstance(body, dict) or not isinstance(body.get("frames"), list):
        raise RuntimeError("SGP 时间线返回格式无效")
    return body


async def _sgp_game_summary(game_id: int, server_id: str | None = None) -> dict:
    credentials = league_lab_service.credentials
    server_id = _normalize_sgp_server_id(server_id) if server_id else _sgp_server_id(credentials)
    host = _SGP_MATCH_HISTORY_HOSTS.get(server_id)
    if not host:
        raise RuntimeError(f"当前区服不支持 SGP 对局源: {server_id or '未知区服'}")
    token_payload = await league_lab_service.request("GET", "/entitlements/v1/token")
    token = token_payload.get("accessToken") if isinstance(token_payload, dict) else None
    if not token:
        raise RuntimeError("LCU 未返回 SGP 授权令牌")
    region_path = _sgp_region_path(credentials, server_id)
    url = f"{host}/match-history-query/v1/products/lol/{region_path}_{int(game_id)}/SUMMARY"
    try:
        async with httpx.AsyncClient(timeout=8.0) as client:
            response = await client.get(url, headers={"Authorization": f"Bearer {token}"})
        response.raise_for_status()
        payload = response.json()
    except (httpx.HTTPError, ValueError) as exc:
        raise RuntimeError(f"SGP 对局请求失败: {type(exc).__name__}") from exc
    body = payload.get("json") if isinstance(payload, dict) and isinstance(payload.get("json"), dict) else payload
    if not isinstance(body, dict) or not isinstance(body.get("participants"), list):
        raise RuntimeError("SGP 对局返回格式无效")
    return body


async def _sgp_common_request(method: str, path: str, *, json_body=None, server_id: str | None = None):
    """Call an SGP common service with an on-demand, memory-only League session token."""
    credentials = league_lab_service.credentials
    server_id = _normalize_sgp_server_id(server_id) if server_id else _sgp_server_id(credentials)
    host = _SGP_COMMON_HOSTS.get(server_id)
    if not host:
        raise RuntimeError(f"当前区服不支持 SGP 通用数据源: {server_id or '未知区服'}")
    token = await league_lab_service.request("GET", "/lol-league-session/v1/league-session-token")
    if not isinstance(token, str) or not token:
        raise RuntimeError("LCU 未返回 League Session 令牌")
    try:
        async with httpx.AsyncClient(timeout=8.0) as client:
            response = await client.request(
                method,
                f"{host}{path}",
                headers={"Authorization": f"Bearer {token}"},
                json=json_body,
            )
        response.raise_for_status()
        payload = response.json()
    except (httpx.HTTPError, ValueError) as exc:
        raise RuntimeError(f"SGP 通用数据请求失败: {type(exc).__name__}") from exc
    return payload


def _normalize_sgp_ranked(payload) -> dict:
    if not isinstance(payload, dict):
        return {}
    queues = []
    for row in payload.get("queues") or []:
        if not isinstance(row, dict):
            continue
        normalized = dict(row)
        normalized["division"] = row.get("division") or row.get("rank") or ""
        queues.append(normalized)
    result = dict(payload)
    result["queues"] = queues
    result["queueMap"] = {str(row.get("queueType")): row for row in queues if row.get("queueType")}
    result["source"] = "sgp"
    return result


async def _sgp_ranked_stats(puuid: str, server_id: str | None = None) -> dict:
    kwargs = {"server_id": server_id} if server_id else {}
    payload = await _sgp_common_request("GET", f"/leagues-ledge/v2/rankedStats/puuid/{puuid}", **kwargs)
    normalized = _normalize_sgp_ranked(payload)
    if not normalized:
        raise RuntimeError("SGP 排位数据返回格式无效")
    return normalized


async def _sgp_player_challenges(puuid: str, server_id: str | None = None) -> dict:
    kwargs = {"server_id": server_id} if server_id else {}
    payload = await _sgp_common_request("POST", f"/challenges-client/v2/all-player-data/?puuid={puuid}", json_body=[], **kwargs)
    if not isinstance(payload, dict):
        raise RuntimeError("SGP 挑战数据返回格式无效")
    return payload


async def _sgp_summoner_by_puuid(puuid: str, server_id: str | None = None) -> dict:
    region_path = _sgp_region_path(league_lab_service.credentials, server_id)
    kwargs = {"server_id": server_id} if server_id else {}
    payload = await _sgp_common_request(
        "POST",
        f"/summoner-ledge/v1/regions/{region_path}/summoners/puuids",
        json_body=[puuid],
        **kwargs,
    )
    row = payload[0] if isinstance(payload, list) and payload else None
    if not isinstance(row, dict):
        raise RuntimeError("SGP 召唤师数据返回格式无效")
    return {
        "puuid": row.get("puuid") or puuid,
        "summonerId": row.get("id"),
        "displayName": row.get("name") or "",
        "gameName": "",
        "tagLine": "",
        "summonerLevel": row.get("level"),
        "profileIconId": row.get("profileIconId"),
        "source": "sgp",
    }


async def _riot_player_account_aliases(game_name: str, tag_line: str) -> list[dict]:
    payload = await league_lab_service.riot_request(
        "GET",
        "/player-account/aliases/v1/lookup",
        params={"gameName": game_name, "tagLine": tag_line},
    )
    return [row for row in (payload or []) if isinstance(row, dict) and row.get("puuid")]


_champion_names_cache: tuple[float, dict[int, str]] | None = None


async def _champion_names() -> dict[int, str]:
    """Return the LCU champion catalog with a short in-memory TTL.

    The catalog is immutable for the lifetime of a client patch and is needed
    by several cards.  Avoiding a full catalog request on every 5-second live
    game refresh removes a surprisingly expensive LCU round trip while still
    allowing a new patch to be picked up after the TTL.
    """
    global _champion_names_cache
    if _champion_names_cache and time.monotonic() - _champion_names_cache[0] < 900:
        return _champion_names_cache[1]
    try:
        rows = await league_lab_service.request("GET", "/lol-game-data/assets/v1/champion-summary.json")
    except RuntimeError:
        return {}
    names = {int(row.get("id")): str(row.get("name") or row.get("alias") or row.get("id")) for row in (rows or []) if isinstance(row, dict) and row.get("id")}
    if names:
        _champion_names_cache = (time.monotonic(), names)
    return names


def _champion_catalog_path() -> Path:
    return get_data_dir() / "league-champion-catalog.json"


_ddragon_catalog_cache: tuple[float, str, list[dict]] | None = None


async def _ddragon_champion_catalog() -> tuple[str, list[dict]]:
    """Return Riot Data Dragon metadata when the local LCU is unavailable."""
    global _ddragon_catalog_cache
    if _ddragon_catalog_cache and time.monotonic() - _ddragon_catalog_cache[0] < 3600:
        return _ddragon_catalog_cache[1], _ddragon_catalog_cache[2]
    try:
        async with httpx.AsyncClient(timeout=12.0, follow_redirects=True) as client:
            versions_response = await client.get("https://ddragon.leagueoflegends.com/api/versions.json")
            versions_response.raise_for_status()
            versions = versions_response.json()
            version = str(versions[0]) if isinstance(versions, list) and versions else ""
            if not re.fullmatch(r"\d+\.\d+\.\d+", version):
                raise ValueError("invalid Data Dragon version")
            catalog_response = await client.get(
                f"https://ddragon.leagueoflegends.com/cdn/{version}/data/zh_CN/champion.json"
            )
            catalog_response.raise_for_status()
            payload = catalog_response.json()
    except (httpx.HTTPError, ValueError, IndexError, KeyError) as exc:
        raise RuntimeError(f"Data Dragon 英雄目录请求失败: {type(exc).__name__}") from exc
    rows = [
        {
            "id": int(row.get("key")),
            "name": str(row.get("name") or row.get("id") or row.get("key")),
            "alias": str(row.get("id") or ""),
            "roles": [str(role).lower() for role in (row.get("tags") or [])],
        }
        for row in (payload.get("data") or {}).values()
        if isinstance(row, dict) and str(row.get("key") or "").isdigit()
    ]
    _ddragon_catalog_cache = (time.monotonic(), version, rows)
    return version, rows


async def _ddragon_champion_icon(champion_id: int) -> tuple[bytes, str]:
    version, rows = await _ddragon_champion_catalog()
    champion = next((row for row in rows if int(row.get("id") or 0) == champion_id), None)
    alias = str((champion or {}).get("alias") or "")
    if not re.fullmatch(r"[A-Za-z0-9]+", alias):
        raise RuntimeError("Data Dragon 英雄头像不存在")
    try:
        async with httpx.AsyncClient(timeout=12.0, follow_redirects=True) as client:
            response = await client.get(
                f"https://ddragon.leagueoflegends.com/cdn/{version}/img/champion/{alias}.png"
            )
        response.raise_for_status()
    except httpx.HTTPError as exc:
        raise RuntimeError(f"Data Dragon 英雄头像请求失败: {type(exc).__name__}") from exc
    return response.content, response.headers.get("content-type") or "image/png"


async def _ddragon_item_icon(item_id: int) -> tuple[bytes, str]:
    """Fetch an item icon when the local LCU artwork endpoint is unavailable."""
    version, _ = await _ddragon_champion_catalog()
    try:
        async with httpx.AsyncClient(timeout=12.0, follow_redirects=True) as client:
            response = await client.get(
                f"https://ddragon.leagueoflegends.com/cdn/{version}/img/item/{int(item_id)}.png"
            )
        response.raise_for_status()
    except httpx.HTTPError as exc:
        raise RuntimeError(f"Data Dragon 装备图标请求失败: {type(exc).__name__}") from exc
    return response.content, response.headers.get("content-type") or "image/png"


async def _champion_catalog() -> list[dict]:
    rows = None
    try:
        rows = await league_lab_service.request("GET", "/lol-game-data/assets/v1/champion-summary.json")
    except RuntimeError:
        try:
            rows = json.loads(_champion_catalog_path().read_text(encoding="utf-8"))
        except (OSError, ValueError):
            rows = []
        if not rows:
            try:
                _, rows = await _ddragon_champion_catalog()
            except RuntimeError:
                rows = []
    normalized = [
        {
            "id": int(row.get("id")),
            "name": str(row.get("name") or row.get("alias") or row.get("id")),
            "alias": str(row.get("alias") or ""),
            "roles": [str(role).lower() for role in (row.get("roles") or [])],
        }
        for row in (rows or []) if isinstance(row, dict) and int(row.get("id") or 0) > 0
    ]
    if normalized:
        path = _champion_catalog_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(normalized, ensure_ascii=False), encoding="utf-8")
    return normalized


def _player_tags_path() -> Path:
    return get_data_dir() / "league-player-tags.json"


def _recent_players_path() -> Path:
    return get_data_dir() / "league-recent-players.json"


def _player_search_history_path() -> Path:
    return get_data_dir() / "league-player-search-history.json"


def _read_player_tags() -> dict[str, dict]:
    try:
        body = json.loads(_player_tags_path().read_text(encoding="utf-8"))
        return body if isinstance(body, dict) else {}
    except (OSError, ValueError):
        return {}


def _write_player_tags(body: dict[str, dict]) -> None:
    path = _player_tags_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(".tmp")
    temp.write_text(json.dumps(body, ensure_ascii=False, indent=2), encoding="utf-8")
    temp.replace(path)


def _player_tag_key(owner_puuid: str, puuid: str) -> str:
    owner = str(owner_puuid or "").strip()
    target = str(puuid or "").strip()
    return f"{owner}::{target}" if owner else target


def _split_player_tag_key(key: str, record: dict | None = None) -> tuple[str, str]:
    raw = str(key or "")
    if "::" in raw:
        owner, puuid = raw.split("::", 1)
        return owner, puuid
    body = record if isinstance(record, dict) else {}
    return str(body.get("_owner_puuid") or ""), raw


def _public_player_tag(record: dict | None) -> dict:
    body = record if isinstance(record, dict) else {}
    return {
        "label": str(body.get("label") or ""),
        "note": str(body.get("note") or ""),
        "color": str(body.get("color") or "emerald"),
    }


def _find_player_tag(tags: dict[str, dict], puuid: str, owner_puuid: str | None = None) -> dict:
    owner = str(owner_puuid if owner_puuid is not None else league_lab_service.current_summoner.get("puuid") or "")
    record = tags.get(_player_tag_key(owner, puuid)) or tags.get(str(puuid))
    return _public_player_tag(record) if record else {}


def _read_recent_players() -> list[dict]:
    try:
        body = json.loads(_recent_players_path().read_text(encoding="utf-8"))
        return body if isinstance(body, list) else []
    except (OSError, ValueError):
        return []


def _write_recent_players(rows: list[dict]) -> None:
    path = _recent_players_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(".tmp")
    temp.write_text(json.dumps(rows[:200], ensure_ascii=False, indent=2), encoding="utf-8")
    temp.replace(path)


def _read_player_search_history() -> list[dict]:
    try:
        body = json.loads(_player_search_history_path().read_text(encoding="utf-8"))
        return [row for row in body if isinstance(row, dict)] if isinstance(body, list) else []
    except (OSError, ValueError):
        return []


def _write_player_search_history(rows: list[dict]) -> None:
    path = _player_search_history_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(".tmp")
    temp.write_text(json.dumps(rows[:300], ensure_ascii=False, indent=2), encoding="utf-8")
    temp.replace(path)


def _remember_player_search(bundle: dict) -> None:
    summoner = bundle.get("summoner") if isinstance(bundle, dict) else None
    if not isinstance(summoner, dict) or not summoner.get("puuid"):
        return
    owner_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    puuid = str(summoner["puuid"])
    server_id = str(bundle.get("server_id") or "")
    rows = _read_player_search_history()
    existing = next((row for row in rows if (
        str(row.get("owner_puuid") or "") == owner_puuid
        and str(row.get("puuid") or "") == puuid
        and str(row.get("server_id") or "") == server_id
    )), None)
    record = {
        "owner_puuid": owner_puuid,
        "puuid": puuid,
        "server_id": server_id,
        "game_name": str(summoner.get("game_name") or ""),
        "tag_line": str(summoner.get("tag_line") or ""),
        "profile_icon_id": summoner.get("profile_icon_id"),
        "pinned": bool((existing or {}).get("pinned")),
        "visited_at": int(time.time() * 1000),
    }
    rows = [row for row in rows if not (
        str(row.get("owner_puuid") or "") == owner_puuid
        and str(row.get("puuid") or "") == puuid
        and str(row.get("server_id") or "") == server_id
    )]
    rows.append(record)
    rows.sort(key=lambda row: int(row.get("visited_at") or 0), reverse=True)
    _write_player_search_history(rows)


def _time_sort_value(value) -> int:
    try:
        return int(float(value or 0))
    except (TypeError, ValueError):
        return 0


def _scalar_match_stats(payload: dict) -> dict[str, bool | float | int | str | None]:
    """Keep the scalar match fields used by LeagueAkari's searchable ten-player matrix."""
    result: dict[str, bool | float | int | str | None] = {}
    for key, value in (payload or {}).items():
        if isinstance(value, (bool, int, float, str)) or value is None:
            result[str(key)] = value
    challenges = (payload or {}).get("challenges")
    if isinstance(challenges, dict):
        for key, value in challenges.items():
            if isinstance(value, (bool, int, float, str)) or value is None:
                result[f"challenge.{key}"] = value
    return result


_INVALID_MATCH_POSITIONS = {"", "NONE", "UNSELECTED", "INVALID", "UNKNOWN", "N/A"}


def _normalize_match_position(value) -> str | None:
    """Map LCU/SGP role sentinels to the same nullable value as LeagueAkari."""
    value = str(value or "").strip().upper()
    if value in _INVALID_MATCH_POSITIONS:
        return None
    return {
        "MID": "MIDDLE",
        "BOT": "BOTTOM",
        "ADC": "BOTTOM",
        "SUPPORT": "UTILITY",
        "SUP": "UTILITY",
    }.get(value, value)


def _normalize_riot_identity(participant: dict, player: dict) -> tuple[str, str, str]:
    """Return one canonical Riot ID without duplicating ``gameName#tagLine``."""
    game_name = str(
        participant.get("riotIdGameName")
        or participant.get("gameName")
        or player.get("gameName")
        or player.get("displayName")
        or player.get("summonerName")
        or ""
    ).strip()
    tag_line = str(
        participant.get("riotIdTagline")
        or participant.get("riotIdTagLine")
        or participant.get("tagLine")
        or player.get("tagLine")
        or ""
    ).strip().lstrip("#")
    if "#" in game_name:
        base, embedded_tag = game_name.rsplit("#", 1)
        if not tag_line or tag_line.casefold() == embedded_tag.casefold():
            game_name, tag_line = base.strip(), tag_line or embedded_tag.strip()
    if tag_line and game_name.casefold().endswith(f"#{tag_line}".casefold()):
        game_name = game_name[: -(len(tag_line) + 1)].rstrip()
    riot_id = f"{game_name}#{tag_line}" if tag_line else game_name
    return game_name, tag_line, riot_id


def _compute_match_win_result(end_of_game_result, stats: dict) -> tuple[bool, str]:
    """Mirror LeagueAkari's abort/remake/loss/win precedence."""
    if str(end_of_game_result or "").startswith("Abort_"):
        return False, "abort"
    if bool(stats.get("gameEndedInEarlySurrender")):
        return True, "remake"
    if bool(stats.get("teamEarlySurrendered")):
        return True, "loss"
    win = stats.get("win")
    if isinstance(win, str):
        win = win.strip().lower() in {"win", "won", "true", "1"}
    return bool(win), "win" if bool(win) else "loss"


def _lcu_perk_styles(stats: dict) -> list[dict]:
    def selection(index: int) -> dict:
        return {
            "perk": int(stats.get(f"perk{index}") or 0),
            "var1": stats.get(f"perk{index}Var1") or 0,
            "var2": stats.get(f"perk{index}Var2") or 0,
            "var3": stats.get(f"perk{index}Var3") or 0,
        }

    return [
        {
            "description": "primaryStyle",
            "style": int(stats.get("perkPrimaryStyle") or 0),
            "selections": [selection(index) for index in range(4)],
        },
        {
            "description": "subStyle",
            "style": int(stats.get("perkSubStyle") or 0),
            "selections": [selection(index) for index in range(4, 6)],
        },
    ]


def _lcu_team_identifier(game: dict, team_id: int, stats: dict) -> str:
    if str(game.get("gameMode") or "").upper() == "CHERRY":
        subteam = int(stats.get("playerSubteamId") or 0)
        if subteam:
            return f"CHERRY-{subteam}"
    return f"TEAM-{team_id}" if team_id else "TEAM-UNKNOWN"


def _normalize_item_id(value) -> int:
    """Return a usable League item id from any LCU/SGP item representation."""
    if isinstance(value, dict):
        value = (
            value.get("item_id")
            or value.get("itemId")
            or value.get("id")
            or value.get("itemID")
        )
    try:
        item_id = int(value or 0)
    except (TypeError, ValueError):
        return 0
    return item_id if item_id > 0 else 0


def _item_values(value) -> list:
    """Flatten list/dict item payloads while retaining their slot order."""
    if isinstance(value, (list, tuple)):
        return list(value)
    if isinstance(value, dict):
        indexed = []
        for key, item in value.items():
            match = re.fullmatch(r"(?:item|slot)?[_-]?(\d+)", str(key), re.IGNORECASE)
            if match:
                indexed.append((int(match.group(1)), item))
        if indexed:
            return [item for _, item in sorted(indexed)]
        return list(value.values())
    return []


def _normalize_item_slots(*sources) -> list[int]:
    """Normalize the seven end-of-game item slots from LCU and SGP shapes.

    LCU details normally expose ``stats.item0`` … ``stats.item6`` while SGP
    summaries may expose ``items``/``itemSlots`` arrays (and some Tencent
    builds wrap the same fields in ``stats``).  Keep seven slots for callers
    that need the original placement, then compact them separately below.
    """
    slots: list[int] = []
    for source in sources:
        if not isinstance(source, dict):
            continue
        for key in ("item_slots", "itemSlots", "items", "loadout"):
            values = _item_values(source.get(key))
            if values:
                slots = [_normalize_item_id(value) for value in values[:7]]
                break
        if slots:
            break
        flat = []
        for index in range(7):
            value = None
            for key in (f"item{index}", f"item_{index}", f"Item{index}"):
                if key in source:
                    value = source.get(key)
                    break
            flat.append(_normalize_item_id(value))
        if any(flat):
            slots = flat
            break
    return (slots + [0] * 7)[:7]


def _normalized_items(*sources) -> tuple[list[int], list[int]]:
    slots = _normalize_item_slots(*sources)
    compact = [item_id for item_id in slots if item_id]
    if not compact:
        for source in sources:
            if not isinstance(source, dict):
                continue
            for key in ("items", "item_ids", "itemIds", "loadout"):
                values = [_normalize_item_id(value) for value in _item_values(source.get(key))]
                compact = [item_id for item_id in values if item_id]
                if compact:
                    slots = (compact + [0] * 7)[:7]
                    break
            if compact:
                break
    return compact[:7], slots


def _normalize_lcu_participant(game: dict, row: dict, identities: dict[str, dict], names: dict[int, str]) -> dict:
    stats = row.get("stats") if isinstance(row.get("stats"), dict) else row
    participant_id = int(row.get("participantId") or stats.get("participantId") or 0)
    identity_row = identities.get(str(participant_id)) or {}
    player = identity_row.get("player") if isinstance(identity_row.get("player"), dict) else identity_row
    game_name, tag_line, riot_id = _normalize_riot_identity(row, player)
    champion_id = int(row.get("championId") or 0)
    team_id = int(row.get("teamId") or stats.get("teamId") or 0)
    team_identifier = _lcu_team_identifier(game, team_id, stats)
    styles = _lcu_perk_styles(stats)
    items, item_slots = _normalized_items(stats, row)
    augments = [int(stats.get(f"playerAugment{index}") or 0) for index in range(1, 7)]
    win, win_result = _compute_match_win_result(game.get("endOfGameResult"), stats)
    position = _normalize_match_position(
        row.get("teamPosition") or row.get("individualPosition") or (row.get("timeline") or {}).get("lane")
    )
    role = _normalize_match_position(row.get("individualPosition") or (row.get("timeline") or {}).get("role"))
    normalized = {
        "participant_id": participant_id,
        "puuid": player.get("puuid") or identity_row.get("puuid") or row.get("puuid") or "",
        "game_name": game_name,
        "tag_line": tag_line,
        "riot_id": riot_id,
        "profile_icon_id": row.get("profileIcon") or row.get("profileIconId") or player.get("profileIcon") or player.get("profileIconId"),
        "team_id": team_id,
        "team_identifier": team_identifier,
        "champion_id": champion_id,
        "champion_name": names.get(champion_id, str(champion_id)),
        "position": position,
        "role": role,
        "spell1_id": row.get("spell1Id") or row.get("summoner1Id") or stats.get("spell1Id") or stats.get("summoner1Id"),
        "spell2_id": row.get("spell2Id") or row.get("summoner2Id") or stats.get("spell2Id") or stats.get("summoner2Id"),
        "spells": [
            row.get("spell1Id") or row.get("summoner1Id") or stats.get("spell1Id") or stats.get("summoner1Id"),
            row.get("spell2Id") or row.get("summoner2Id") or stats.get("spell2Id") or stats.get("summoner2Id"),
        ],
        "kills": stats.get("kills", 0), "deaths": stats.get("deaths", 0), "assists": stats.get("assists", 0),
        "win": win, "win_result": win_result,
        "is_surrender": bool(stats.get("gameEndedInSurrender"))
        or bool(stats.get("teamEarlySurrendered"))
        or win_result == "remake",
        "gold": stats.get("goldEarned", 0), "level": stats.get("champLevel", 0), "gold_spent": stats.get("goldSpent", 0),
        "cs": int(stats.get("totalMinionsKilled", 0) or 0) + int(stats.get("neutralMinionsKilled", 0) or 0),
        "damage": stats.get("totalDamageDealtToChampions", 0), "damage_taken": stats.get("totalDamageTaken", 0),
        "healing": stats.get("totalHeal", 0), "time_ccing": stats.get("totalTimeCCDealt", 0),
        "tower_damage": stats.get("damageDealtToTurrets", 0), "vision_score": stats.get("visionScore", 0),
        "items": items, "item_slots": item_slots,
        "perks": [int(stats.get(f"perk{index}") or 0) for index in range(6) if stats.get(f"perk{index}")],
        "perk_styles": styles, "styles": styles,
        "augments": [augment for augment in augments if augment], "augment_slots": augments,
        "role_bound_item": int(stats.get("roleBoundItem") or 0),
        "subteam_placement": stats.get("subteamPlacement", 0),
        "game_ended_in_early_surrender": bool(stats.get("gameEndedInEarlySurrender")),
        "game_ended_in_surrender": bool(stats.get("gameEndedInSurrender")),
        "team_early_surrendered": bool(stats.get("teamEarlySurrendered")),
        "double_kills": stats.get("doubleKills", 0), "triple_kills": stats.get("tripleKills", 0),
        "quadra_kills": stats.get("quadraKills", 0), "penta_kills": stats.get("pentaKills", 0),
        "challenges": row.get("challenges") or stats.get("challenges") or {},
        "raw_stats": _scalar_match_stats({**stats, "challenges": row.get("challenges") or stats.get("challenges") or {}}),
    }
    normalized["teamIdentifier"] = team_identifier
    normalized["winResult"] = win_result
    normalized["isSurrender"] = normalized["is_surrender"]
    return normalized


def _normalize_lcu_team_stats(game: dict, participants: list[dict]) -> tuple[list[dict], dict[str, dict]]:
    grouped: dict[str, list[dict]] = {}
    for participant in participants:
        grouped.setdefault(str(participant.get("team_identifier") or "TEAM-UNKNOWN"), []).append(participant)
    raw_teams = {f"TEAM-{int(row.get('teamId') or 0)}": row for row in game.get("teams") or [] if isinstance(row, dict)}
    teams, team_stat_map = [], {}
    for identifier, rows in grouped.items():
        raw = raw_teams.get(identifier) or {}
        first = rows[0]
        result = first.get("win_result") or ("win" if first.get("win") else "loss")
        team = {
            "team_identifier": identifier, "teamIdentifier": identifier,
            "team_id": first.get("team_id"), "bans": raw.get("bans") or [],
            "win": bool(first.get("win")), "win_result": result,
            "winResult": result, "is_surrender": bool(first.get("is_surrender")),
            "isSurrender": bool(first.get("is_surrender")),
            "total_kills": sum(int(row.get("kills") or 0) for row in rows),
            "total_deaths": sum(int(row.get("deaths") or 0) for row in rows),
            "total_assists": sum(int(row.get("assists") or 0) for row in rows),
            "total_damage_dealt_to_champions": sum(float(row.get("damage") or 0) for row in rows),
            "total_damage_taken": sum(float(row.get("damage_taken") or 0) for row in rows),
            "total_gold_earned": sum(float(row.get("gold") or 0) for row in rows),
            "total_cs": sum(int(row.get("cs") or 0) for row in rows),
            "players": rows,
        }
        teams.append(team)
        team_stat_map[identifier] = team
    return teams, team_stat_map


async def _complete_lcu_match_history(payload: dict | None) -> dict | None:
    """Complete LCU summaries exactly like LeagueAkari's getGame queue."""
    if not isinstance(payload, dict):
        return payload
    games_container = payload.get("games")
    games = games_container.get("games") if isinstance(games_container, dict) else games_container
    if not isinstance(games, list):
        return payload
    semaphore = asyncio.Semaphore(10)

    async def complete(summary: dict) -> dict:
        game_id = summary.get("gameId") if isinstance(summary, dict) else None
        try:
            game_id = int(game_id)
        except (TypeError, ValueError):
            return summary
        if game_id <= 0:
            return summary
        try:
            async with semaphore:
                detail = await league_lab_service.request("GET", f"/lol-match-history/v1/games/{game_id}")
            detail = detail.get("json") if isinstance(detail, dict) and isinstance(detail.get("json"), dict) else detail
            if isinstance(detail, dict) and isinstance(detail.get("participants"), list):
                return detail
        except Exception as exc:  # optional completion must not hide an otherwise usable summary
            logger.debug("LCU match completion failed for %s: %s", game_id, exc)
        return summary

    completed = await asyncio.gather(*(complete(game) for game in games))
    if isinstance(games_container, dict):
        return {**payload, "games": {**games_container, "games": completed}}
    return {**payload, "games": completed}


def _remember_recent_players(players: list[dict], game_id=None) -> None:
    existing = {str(row.get("puuid")): row for row in _read_recent_players() if row.get("puuid")}
    now = int(time.time() * 1000)
    for player in players:
        puuid = str(player.get("puuid") or "")
        if not puuid:
            continue
        summoner = player.get("summoner") or {}
        previous = existing.get(puuid) or {}
        encounters = [row for row in (previous.get("encounters") or []) if isinstance(row, dict)]
        self_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
        if game_id and self_puuid and puuid != self_puuid:
            encounter = {
                "game_id": game_id,
                "self_puuid": self_puuid,
                "seen_at": now,
                "played_at": None,
                "game_mode": league_lab_service.game_mode,
                "target": {
                    "team_id": player.get("team"),
                    "champion_id": player.get("champion_id"),
                    "champion_name": player.get("champion_name"),
                },
            }
            encounters = [row for row in encounters if not (
                str(row.get("game_id")) == str(game_id) and str(row.get("self_puuid") or "") == self_puuid
            )]
            encounters.insert(0, encounter)
        existing[puuid] = {
            **previous,
            "puuid": puuid,
            "game_name": summoner.get("gameName") or summoner.get("displayName") or previous.get("game_name") or "",
            "tag_line": summoner.get("tagLine") or previous.get("tag_line") or "",
            "profile_icon_id": summoner.get("profileIconId") or previous.get("profile_icon_id"),
            "last_seen_at": now,
            "last_game_id": game_id or previous.get("last_game_id"),
            "team": player.get("team"),
            "champion_id": player.get("champion_id"),
            "champion_name": player.get("champion_name"),
            "encounters": encounters[:50],
        }
    _write_recent_players(sorted(existing.values(), key=lambda row: int(row.get("last_seen_at") or 0), reverse=True))


def _index_match_encounters(matches: list[dict], self_puuid: str) -> None:
    if not self_puuid:
        return
    existing = {str(row.get("puuid")): row for row in _read_recent_players() if row.get("puuid")}
    changed = False
    for match in matches:
        game_id = match.get("game_id")
        if not game_id:
            continue
        participants = [row for row in (match.get("participants") or []) if isinstance(row, dict)]
        own = next((row for row in participants if str(row.get("puuid") or "") == self_puuid), None)
        for participant in participants:
            puuid = str(participant.get("puuid") or "")
            if not puuid or puuid == self_puuid:
                continue
            previous = existing.get(puuid) or {"puuid": puuid}
            encounters = [row for row in (previous.get("encounters") or []) if isinstance(row, dict)]
            entry = {
                "game_id": game_id,
                "self_puuid": self_puuid,
                "seen_at": int(time.time() * 1000),
                "played_at": match.get("played_at"),
                "duration_seconds": match.get("duration_seconds"),
                "game_mode": match.get("game_mode"),
                "game_type": match.get("game_type"),
                "queue_id": match.get("queue_id"),
                "target": participant,
                "self": own or {},
            }
            encounters = [row for row in encounters if not (
                str(row.get("game_id")) == str(game_id) and str(row.get("self_puuid") or "") == self_puuid
            )]
            encounters.append(entry)
            encounters.sort(key=lambda row: _time_sort_value(row.get("played_at") or row.get("seen_at")), reverse=True)
            existing[puuid] = {**previous, "puuid": puuid, "encounters": encounters[:50]}
            changed = True
    if changed:
        _write_recent_players(sorted(existing.values(), key=lambda row: int(row.get("last_seen_at") or 0), reverse=True))


def _normalize_match_rows(payload: dict, names: dict[int, str], puuid: str = "") -> list[dict]:
    games = ((payload or {}).get("games") or {}).get("games") or []
    normalized = []
    for game in games:
        if not isinstance(game, dict):
            continue
        identities = game.get("participantIdentities") or []
        identity = next(
            (
                row
                for row in identities
                if not puuid or str((row.get("player") or {}).get("puuid") or row.get("puuid") or "") == puuid
            ),
            identities[0] if identities else None,
        )
        participant_id = identity.get("participantId") if identity else None
        participant = next(
            (p for p in (game.get("participants") or []) if p.get("participantId") == participant_id), None
        )
        if not participant:
            continue
        stats = participant.get("stats") or {}
        match_items, match_item_slots = _normalized_items(stats, participant)
        champion_id = int(participant.get("championId") or 0)
        identity_by_id = {str(row.get("participantId")): row for row in identities if isinstance(row, dict)}
        scoped_participants = [
            _normalize_lcu_participant(game, row, identity_by_id, names)
            for row in game.get("participants") or []
            if isinstance(row, dict)
        ]
        teams, team_stat_map = _normalize_lcu_team_stats(game, scoped_participants)
        own = next((row for row in scoped_participants if str(row.get("puuid") or "") == puuid), None)
        if own is None:
            own = _normalize_lcu_participant(game, participant, identity_by_id, names)
        win_result = own.get("win_result") or ("win" if own.get("win") else "loss")
        normalized.append(
            {
                "game_id": game.get("gameId"),
                "played_at": game.get("gameCreationDate") or game.get("gameCreation"),
                "duration_seconds": game.get("gameDuration"),
                "game_mode": game.get("gameMode"),
                "game_type": game.get("gameType"),
                "game_version": game.get("gameVersion"),
                "map_id": game.get("mapId"),
                "queue_id": game.get("queueId"),
                "participant_puuid": own.get("puuid") or ((identity.get("player") or {}).get("puuid") if identity else None),
                "team_id": participant.get("teamId"),
                "participants": scoped_participants,
                "team_identifier": own.get("team_identifier"),
                "teamIdentifier": own.get("team_identifier"),
                "game_name": own.get("game_name"),
                "tag_line": own.get("tag_line"),
                "riot_id": own.get("riot_id"),
                "profile_icon_id": own.get("profile_icon_id"),
                "teams": teams,
                "team_stats": team_stat_map,
                "team_stat_map": team_stat_map,
                "position": own.get("position"),
                "role": own.get("role"),
                "champion_id": champion_id,
                "champion_name": names.get(champion_id, str(champion_id)),
                "spell1_id": participant.get("spell1Id") or participant.get("summoner1Id") or stats.get("spell1Id") or stats.get("summoner1Id"),
                "spell2_id": participant.get("spell2Id") or participant.get("summoner2Id") or stats.get("spell2Id") or stats.get("summoner2Id"),
                "spells": [
                    participant.get("spell1Id") or participant.get("summoner1Id") or stats.get("spell1Id") or stats.get("summoner1Id"),
                    participant.get("spell2Id") or participant.get("summoner2Id") or stats.get("spell2Id") or stats.get("summoner2Id"),
                ],
                "kills": stats.get("kills", 0),
                "deaths": stats.get("deaths", 0),
                "assists": stats.get("assists", 0),
                "win": own.get("win"),
                "cs": int(stats.get("totalMinionsKilled", 0)) + int(stats.get("neutralMinionsKilled", 0)),
                "gold": stats.get("goldEarned", 0),
                "damage": stats.get("totalDamageDealtToChampions", 0),
                "damage_taken": stats.get("totalDamageTaken", 0),
                "gold_spent": stats.get("goldSpent", 0),
                "tower_damage": stats.get("damageDealtToTurrets", 0),
                "healing": stats.get("totalHeal", 0),
                "time_ccing": stats.get("totalTimeCCDealt", 0),
                "vision_score": stats.get("visionScore", 0),
                "level": stats.get("champLevel", 0),
                "double_kills": stats.get("doubleKills", 0),
                "triple_kills": stats.get("tripleKills", 0),
                "quadra_kills": stats.get("quadraKills", 0),
                "penta_kills": stats.get("pentaKills", 0),
                "items": match_items,
                "item_slots": match_item_slots,
                "perks": [stats.get(f"perk{i}") for i in range(6) if stats.get(f"perk{i}")],
                "perk_styles": _lcu_perk_styles(stats),
                "styles": _lcu_perk_styles(stats),
                "augments": [stats.get(f"playerAugment{i}") for i in range(1, 7) if stats.get(f"playerAugment{i}")],
                "augment_slots": [int(stats.get(f"playerAugment{i}") or 0) for i in range(1, 7)],
                "challenges": participant.get("challenges") or stats.get("challenges") or {},
                "win_result": win_result,
                "winResult": win_result,
                "is_surrender": bool(stats.get("gameEndedInSurrender"))
                or bool(stats.get("teamEarlySurrendered"))
                or win_result == "remake",
                "isSurrender": bool(stats.get("gameEndedInSurrender"))
                or bool(stats.get("teamEarlySurrendered"))
                or win_result == "remake",
            }
        )
    return normalized


_MATCH_RULE_NUMERIC_FIELDS = {
    "kills", "deaths", "assists", "kda", "damage", "gold", "cs", "queue_id",
    "champion_id", "duration_seconds", "spell1_id", "spell2_id", "kill_participation",
    "vision_score", "damage_taken", "gold_spent", "tower_damage", "healing", "time_ccing",
    "solo_kills", "double_kills", "triple_kills", "quadra_kills", "penta_kills", "level",
    "played_at",
}

_MATCH_RULE_CHALLENGE_FIELDS = {
    "kill_participation": "killParticipation",
    "vision_score": "visionScore",
    "damage_taken": "damageTakenOnTeamPercentage",
    "gold_spent": "goldPerMinute",
    "tower_damage": "damageToTurrets",
    "healing": "effectiveHealAndShielding",
    "time_ccing": "enemyChampionImmobilizations",
    "solo_kills": "soloKills",
}


def _match_rule_value(match: dict, field: str):
    """Read the same normalized fields exposed by the React filter tree."""
    aliases = {
        "champion_name": ("champion_name", "championName"),
        "champion_id": ("champion_id", "championId"),
        "game_mode": ("game_mode", "gameMode"),
        "game_type": ("game_type", "gameType"),
        "queue_id": ("queue_id", "queueId"),
        "duration_seconds": ("duration_seconds", "gameDuration"),
        "played_at": ("played_at", "playedAt", "gameCreation"),
        "spell1_id": ("spell1_id", "spell1Id", "summoner1Id"),
        "spell2_id": ("spell2_id", "spell2Id", "summoner2Id"),
    }
    if field == "kda":
        return (float(match.get("kills") or 0) + float(match.get("assists") or 0)) / max(float(match.get("deaths") or 0), 1.0)
    if field == "has_item":
        return match.get("items") or match.get("item_ids") or match.get("itemIds") or []
    if field == "has_spell":
        return [match.get("spell1_id", match.get("spell1Id")), match.get("spell2_id", match.get("spell2Id"))]
    if field == "has_perk":
        return match.get("perks") or match.get("runes") or []
    if field == "has_augment":
        return match.get("augments") or []
    if field == "is_remake":
        duration = float(match.get("duration_seconds") or match.get("gameDuration") or 0)
        return 0 < duration <= 300
    if field == "is_matched_game":
        return str(match.get("game_type") or match.get("gameType") or "").upper() == "MATCHED_GAME"
    if field == "is_pve_game":
        return bool(re.search(r"PVE|BOT", f"{match.get('game_type', '')} {match.get('game_mode', '')}".upper()))
    if field in _MATCH_RULE_CHALLENGE_FIELDS:
        direct = match.get(field)
        if direct is not None:
            return direct
        challenges = match.get("challenges") if isinstance(match.get("challenges"), dict) else {}
        return challenges.get(_MATCH_RULE_CHALLENGE_FIELDS[field])
    for key in aliases.get(field, (field,)):
        if key in match:
            return match.get(key)
    return None


def _match_rule_participants(match: dict, scope: str) -> list[dict]:
    participants = [row for row in (match.get("participants") or []) if isinstance(row, dict)]
    if scope == "self":
        return [match]
    self_puuid = str(match.get("participant_puuid") or match.get("participantPuuid") or "")
    self_team = str(match.get("team_id") if match.get("team_id") is not None else match.get("teamId") or "")
    _, group = str(scope).split("-", 1) if "-" in str(scope) else ("any", "all")
    if group == "allies":
        return [
            row for row in participants
            if str(row.get("team_id") if row.get("team_id") is not None else row.get("teamId") or "") == self_team
            and (not self_puuid or str(row.get("puuid") or row.get("participant_puuid") or "") != self_puuid)
        ]
    if group == "enemies":
        return [
            row for row in participants
            if str(row.get("team_id") if row.get("team_id") is not None else row.get("teamId") or "") != self_team
        ]
    return [
        row for row in participants
        if not self_puuid or str(row.get("puuid") or row.get("participant_puuid") or "") != self_puuid
    ]


def _matches_league_rule(match: dict, rule: dict) -> bool:
    if not isinstance(rule, dict) or not rule.get("field") or rule.get("value") == "":
        return True
    field = str(rule.get("field"))
    operator = str(rule.get("operator") or "eq")
    scope = str(rule.get("scope") or "self")
    rows = _match_rule_participants(match, scope)
    if scope != "self" and not rows:
        return False

    def compare(row: dict) -> bool:
        actual = _match_rule_value(row, field)
        expected = str(rule.get("value") or "")
        if isinstance(actual, (list, tuple, set)):
            contains = any(str(item) == expected for item in actual if item not in (None, ""))
            return not contains if operator == "neq" else contains
        if isinstance(actual, bool):
            truthy = expected.strip().lower() in {"true", "1", "yes", "是", "胜利"}
            return actual != truthy if operator == "neq" else actual == truthy
        if field in _MATCH_RULE_NUMERIC_FIELDS:
            try:
                left, right = float(actual or 0), float(expected)
            except (TypeError, ValueError):
                return True
            if operator == "gte":
                return left >= right
            if operator == "lte":
                return left <= right
            if operator == "neq":
                return left != right
            return left == right
        left = str(actual or "").casefold()
        right = expected.casefold()
        if operator == "neq":
            return left != right
        if operator == "contains":
            return right in left
        return left == right

    quantifier = scope.split("-", 1)[0] if scope != "self" else "any"
    return all(compare(row) for row in rows) if quantifier == "every" else any(compare(row) for row in rows)


def _matches_league_rule_tree(match: dict, node: dict | None) -> bool:
    if not node:
        return True
    if node.get("type") == "rule":
        return _matches_league_rule(match, node)
    children = node.get("children") if isinstance(node.get("children"), list) else []
    if not children:
        return True
    values = [_matches_league_rule_tree(match, child) for child in children if isinstance(child, dict)]
    if not values:
        return True
    combined = any(values) if str(node.get("logic") or "and") == "or" else all(values)
    return not combined if bool(node.get("negate")) else combined


def _matches_league_collect_request(match: dict, body: LeagueMatchCollectRequest) -> bool:
    if body.queue_id is not None and int(match.get("queue_id") or 0) != int(body.queue_id):
        return False
    if body.result != "all":
        own_win = bool(match.get("win"))
        if body.result == "win" and not own_win:
            return False
        if body.result == "loss" and own_win:
            return False
    needle = body.query.strip().casefold()
    if needle:
        haystack = " ".join(
            str(value or "")
            for value in (
                match.get("game_id"), match.get("champion_name"), match.get("game_mode"),
                match.get("game_type"), match.get("game_name"), match.get("tag_line"),
                *[
                    value
                    for participant in match.get("participants") or []
                    for value in (participant.get("game_name"), participant.get("tag_line"), participant.get("champion_name"))
                    if isinstance(participant, dict)
                ],
            )
        ).casefold()
        if needle not in haystack:
            return False
    return _matches_league_rule_tree(match, body.filter_tree)


def _normalize_sgp_match_rows(payload: dict, names: dict[int, str], puuid: str) -> list[dict]:
    normalized = []
    for wrapper in (payload or {}).get("games") or []:
        game = wrapper.get("json") if isinstance(wrapper, dict) and isinstance(wrapper.get("json"), dict) else wrapper
        if not isinstance(game, dict):
            continue
        participant = next(
            (row for row in (game.get("participants") or []) if str(row.get("puuid") or "") == puuid),
            None,
        )
        if not participant:
            continue
        participant_stats = participant.get("stats") if isinstance(participant.get("stats"), dict) else {}
        participant_items, participant_item_slots = _normalized_items(participant, participant_stats)
        champion_id = int(participant.get("championId") or 0)
        scoped_participants = []
        for row in game.get("participants") or []:
            row_stats = row.get("stats") if isinstance(row.get("stats"), dict) else {}
            row_champion_id = int(row.get("championId") or 0)
            row_items, row_item_slots = _normalized_items(row, row_stats)
            row_spell1_id = row.get("summoner1Id") or row.get("spell1Id") or row_stats.get("summoner1Id") or row_stats.get("spell1Id")
            row_spell2_id = row.get("summoner2Id") or row.get("spell2Id") or row_stats.get("summoner2Id") or row_stats.get("spell2Id")
            scoped_participants.append({
                "participant_id": row.get("participantId"), "puuid": row.get("puuid"),
                "game_name": row.get("riotIdGameName") or row.get("gameName") or row.get("summonerName") or "",
                "tag_line": row.get("riotIdTagline") or row.get("tagLine") or "",
                "profile_icon_id": row.get("profileIcon") or row.get("profileIconId"), "team_id": row.get("teamId"),
                "champion_id": row_champion_id, "champion_name": names.get(row_champion_id, row.get("championName") or str(row_champion_id)),
                "position": row.get("teamPosition"), "role": row.get("individualPosition"),
                "spell1_id": row_spell1_id, "spell2_id": row_spell2_id, "spells": [row_spell1_id, row_spell2_id],
                "kills": row.get("kills", 0), "deaths": row.get("deaths", 0), "assists": row.get("assists", 0), "win": bool(row.get("win")),
                "gold": row.get("goldEarned", 0), "gold_spent": row.get("goldSpent", 0), "level": row.get("champLevel", 0),
                "cs": int(row.get("totalMinionsKilled", 0)) + int(row.get("neutralMinionsKilled", 0)),
                "damage": row.get("totalDamageDealtToChampions", 0), "damage_taken": row.get("totalDamageTaken", 0),
                "healing": row.get("totalHeal", 0), "time_ccing": row.get("totalTimeCCDealt", 0),
                "tower_damage": row.get("damageDealtToTurrets", 0), "vision_score": row.get("visionScore", 0),
                "items": row_items, "item_slots": row_item_slots,
                "perks": [row.get(f"perk{i}") for i in range(6) if row.get(f"perk{i}")],
                "augments": [row.get(f"playerAugment{i}") for i in range(1, 7) if row.get(f"playerAugment{i}")], "challenges": row.get("challenges") or {},
                "raw_stats": _scalar_match_stats(row),
            })
        participant_spell1_id = participant.get("summoner1Id") or participant.get("spell1Id") or participant_stats.get("summoner1Id") or participant_stats.get("spell1Id")
        participant_spell2_id = participant.get("summoner2Id") or participant.get("spell2Id") or participant_stats.get("summoner2Id") or participant_stats.get("spell2Id")
        normalized.append(
            {
                "game_id": game.get("gameId"),
                "played_at": game.get("gameCreation") or game.get("gameStartTimestamp"),
                "duration_seconds": game.get("gameDuration"),
                "game_mode": game.get("gameMode"),
                "game_type": game.get("gameType"),
                "game_version": game.get("gameVersion"),
                "map_id": game.get("mapId"),
                "queue_id": game.get("queueId"),
                "participant_puuid": participant.get("puuid"),
                "team_id": participant.get("teamId"),
                "participants": scoped_participants,
                "position": participant.get("teamPosition"),
                "role": participant.get("individualPosition"),
                "champion_id": champion_id,
                "champion_name": names.get(champion_id, participant.get("championName") or str(champion_id)),
                "spell1_id": participant_spell1_id,
                "spell2_id": participant_spell2_id,
                "spells": [participant_spell1_id, participant_spell2_id],
                "kills": participant.get("kills", 0),
                "deaths": participant.get("deaths", 0),
                "assists": participant.get("assists", 0),
                "win": bool(participant.get("win")),
                "cs": int(participant.get("totalMinionsKilled", 0)) + int(participant.get("neutralMinionsKilled", 0)),
                "gold": participant.get("goldEarned", 0),
                "damage": participant.get("totalDamageDealtToChampions", 0),
                "damage_taken": participant.get("totalDamageTaken", 0),
                "gold_spent": participant.get("goldSpent", 0),
                "tower_damage": participant.get("damageDealtToTurrets", 0),
                "healing": participant.get("totalHeal", 0),
                "time_ccing": participant.get("totalTimeCCDealt", 0),
                "vision_score": participant.get("visionScore", 0),
                "level": participant.get("champLevel", 0),
                "double_kills": participant.get("doubleKills", 0),
                "triple_kills": participant.get("tripleKills", 0),
                "quadra_kills": participant.get("quadraKills", 0),
                "penta_kills": participant.get("pentaKills", 0),
                "items": participant_items, "item_slots": participant_item_slots,
                "perks": [participant.get(f"perk{i}") for i in range(6) if participant.get(f"perk{i}")],
                "augments": [participant.get(f"playerAugment{i}") for i in range(1, 7) if participant.get(f"playerAugment{i}")],
                "challenges": participant.get("challenges") or {},
                "source": "sgp",
            }
        )
    return normalized


def _preview_win(value) -> bool | None:
    if isinstance(value, bool):
        return value
    normalized = str(value or "").strip().lower()
    if normalized in {"win", "won", "true", "1"}:
        return True
    if normalized in {"fail", "failed", "loss", "lose", "lost", "false", "0"}:
        return False
    return None


def _normalize_game_preview(game: dict, names: dict[int, str], source: str, timeline: dict | None = None) -> dict:
    identities: dict[int, dict] = {}
    for row in game.get("participantIdentities") or []:
        if not isinstance(row, dict):
            continue
        participant_id = int(row.get("participantId") or 0)
        if participant_id:
            identities[participant_id] = row.get("player") if isinstance(row.get("player"), dict) else row
    team_wins: dict[int, bool | None] = {}
    for team in game.get("teams") or []:
        if not isinstance(team, dict):
            continue
        team_id = int(team.get("teamId") or 0)
        if team_id:
            team_wins[team_id] = _preview_win(team.get("win"))
    tags = _read_player_tags()
    players = []
    for index, participant in enumerate(game.get("participants") or []):
        if not isinstance(participant, dict):
            continue
        participant_id = int(participant.get("participantId") or index + 1)
        identity = identities.get(participant_id) or {}
        stats = participant.get("stats") if isinstance(participant.get("stats"), dict) else participant
        champion_id = int(participant.get("championId") or stats.get("championId") or 0)
        team_id = int(participant.get("teamId") or stats.get("teamId") or 0)
        puuid = str(
            participant.get("puuid")
            or identity.get("puuid")
            or identity.get("playerPuuid")
            or ""
        )
        game_name = str(
            participant.get("riotIdGameName")
            or participant.get("gameName")
            or identity.get("gameName")
            or identity.get("displayName")
            or identity.get("summonerName")
            or participant.get("summonerName")
            or f"玩家 {participant_id}"
        )
        tag_line = str(participant.get("riotIdTagline") or participant.get("tagLine") or identity.get("tagLine") or "")
        win = _preview_win(stats.get("win"))
        if win is None:
            win = team_wins.get(team_id)
        kills = int(stats.get("kills") or 0)
        deaths = int(stats.get("deaths") or 0)
        assists = int(stats.get("assists") or 0)
        cs = int(stats.get("totalMinionsKilled") or 0) + int(stats.get("neutralMinionsKilled") or 0)
        players.append(
            {
                "participant_id": participant_id,
                "puuid": puuid,
                "team": team_id,
                "champion_id": champion_id,
                "champion_name": names.get(champion_id, participant.get("championName") or str(champion_id)),
                "position": participant.get("teamPosition") or participant.get("individualPosition") or (participant.get("timeline") or {}).get("lane") or "",
                "summoner": {
                    "gameName": game_name,
                    "tagLine": tag_line,
                    "profileIconId": participant.get("profileIcon") or participant.get("profileIconId") or identity.get("profileIcon") or identity.get("profileIconId"),
                },
                "tag": _find_player_tag(tags, puuid),
                "premade_group": None,
                "recent": {"matches": 0, "wins": 0},
                "champion_usage": {"matches": 0, "wins": 0, "average_kda": 0},
                "match_stats": {
                    "kills": kills,
                    "deaths": deaths,
                    "assists": assists,
                    "kda": round((kills + assists) / max(1, deaths), 2),
                    "damage": int(stats.get("totalDamageDealtToChampions") or 0),
                    "gold": int(stats.get("goldEarned") or 0),
                    "cs": cs,
                    "items": [int(stats.get(f"item{i}")) for i in range(7) if stats.get(f"item{i}")],
                    "win": win,
                },
            }
        )
    frames = timeline.get("frames") if isinstance(timeline, dict) else []
    frames = frames if isinstance(frames, list) else []
    timeline_summary = {
        "loaded": isinstance(timeline, dict),
        "frame_count": len(frames),
        "event_count": sum(len(frame.get("events") or []) for frame in frames if isinstance(frame, dict)),
    }
    teams = []
    for team_id in sorted({int(player.get("team") or 0) for player in players}):
        if not team_id:
            continue
        team_players = [player for player in players if int(player.get("team") or 0) == team_id]
        team_win = next((player["match_stats"]["win"] for player in team_players if player["match_stats"]["win"] is not None), team_wins.get(team_id))
        teams.append({"team_id": team_id, "win": team_win, "players": team_players})
    metadata = {
        "game_id": game.get("gameId"),
        "played_at": game.get("gameCreationDate") or game.get("gameCreation") or game.get("gameStartTimestamp"),
        "duration_seconds": game.get("gameDuration"),
        "game_mode": game.get("gameMode"),
        "game_type": game.get("gameType"),
        "queue_id": game.get("queueId"),
        "platform_id": game.get("platformId") or game.get("platformID"),
    }
    ongoing_preview = {
        "phase": "HistoricalPreview",
        "queue": {"id": metadata["queue_id"], "gameMode": metadata["game_mode"]},
        "game_id": metadata["game_id"],
        "players": players,
        "available": bool(players),
        "historical_preview": True,
        "source": source,
        "metadata": metadata,
    }
    return {
        "source": source,
        "metadata": metadata,
        "teams": teams,
        "timeline": timeline_summary,
        "ongoing_preview": ongoing_preview,
    }


def _infer_premade_groups(histories: dict[str, dict], active_puuids: set[str], threshold: int = 3) -> dict[str, int]:
    """Infer premade groups using LeagueAkari's all-pairs rule.

    A player pair is evidence-backed only when they appeared on the same team
    in ``threshold`` distinct games.  A group is valid only when *every* pair
    in that group has reached the threshold.  This intentionally rejects the
    tempting connected-components shortcut (A-B and B-C must not imply an
    A/B/C premade).  Overlapping candidates are merged only when their union
    remains a valid all-pairs group.
    """
    threshold = max(1, int(threshold or 1))
    game_teams: dict[str, list[set[str]]] = {}
    for payload in histories.values():
        for game in (((payload or {}).get("games") or {}).get("games") or []):
            if not isinstance(game, dict):
                continue
            game_id = str(game.get("gameId") or "")
            if not game_id or game_id in game_teams:
                continue
            team_members: dict[int, set[str]] = {}
            participants = {
                row.get("participantId"): row
                for row in (game.get("participants") or [])
                if isinstance(row, dict)
            }
            identities = game.get("participantIdentities") or []
            # Some LCU/SGP payloads already put puuid on participants.  Keep
            # that form as a fallback instead of dropping otherwise valid
            # teammate evidence.
            for identity in identities:
                if not isinstance(identity, dict):
                    continue
                player = identity.get("player") or {}
                puuid = str(player.get("puuid") or identity.get("puuid") or "")
                participant = participants.get(identity.get("participantId")) or {}
                team_id = int(participant.get("teamId") or 0)
                if puuid and team_id:
                    team_members.setdefault(team_id, set()).add(puuid)
            for participant_id, participant in participants.items():
                if not isinstance(participant, dict):
                    continue
                puuid = str(participant.get("puuid") or participant.get("playerPuuid") or "")
                team_id = int(participant.get("teamId") or 0)
                if puuid and team_id:
                    team_members.setdefault(team_id, set()).add(puuid)
            game_teams[game_id] = list(team_members.values())
    together: dict[tuple[str, str], int] = {}
    for teams in game_teams.values():
        for team in teams:
            visible = sorted(team & active_puuids)
            for index, first in enumerate(visible):
                for second in visible[index + 1:]:
                    together[(first, second)] = together.get((first, second), 0) + 1
    players = sorted(active_puuids)

    def is_valid_group(group: tuple[str, ...]) -> bool:
        return all(
            together.get((first, second), 0) >= threshold
            for first, second in combinations(group, 2)
        )

    # Enumerating subsets is bounded by the ten players in a live game and
    # mirrors LeagueAkari's combinations -> removeSubsets pipeline.
    candidates = [
        group
        for size in range(2, len(players) + 1)
        for group in combinations(players, size)
        if is_valid_group(group)
    ]
    maximal = [
        group for group in candidates
        if not any(set(group) < set(other) for other in candidates)
    ]
    maximal.sort(key=lambda group: (-len(group), group))

    # Only merge overlapping sets if doing so does not turn a pairwise-valid
    # collection into the connected-components false positive described above.
    merged: list[tuple[str, ...]] = []
    for candidate in maximal:
        current = set(candidate)
        changed = True
        while changed:
            changed = False
            for index, existing in enumerate(merged):
                union = tuple(sorted(current | set(existing)))
                if current.intersection(existing) and is_valid_group(union):
                    current = set(union)
                    merged.pop(index)
                    changed = True
                    break
        merged.append(tuple(sorted(current)))
    merged.sort()

    # A player can occur in two overlapping, incompatible cliques.  The
    # deterministic first assignment keeps the public legacy map shape while
    # never labelling a non-clique as one premade team.
    groups: dict[str, int] = {}
    assigned: set[str] = set()
    for group_id, group in enumerate(merged, start=1):
        if len(group) < 2:
            continue
        # Incompatible overlapping cliques (for example AB and BC) are not a
        # single premade.  Keep the strongest deterministic candidate and do
        # not assign a second label to an already assigned player.
        if assigned.intersection(group):
            continue
        for puuid in group:
            groups[puuid] = group_id
        assigned.update(group)
    return groups


def _ongoing_performance_tags(
    matches: list[dict],
    *,
    show_streaks: bool = True,
    show_performance: bool = True,
    tag_settings: dict[str, bool] | None = None,
) -> list[dict]:
    """Build read-only player-card tags from the already loaded match sample."""
    tags: list[dict] = []
    if not matches:
        return tags
    enabled = tag_settings or {}

    def known_result(value):
        if isinstance(value, bool):
            return value
        if isinstance(value, str):
            normalized = value.strip().lower()
            if normalized in {"true", "win", "won", "victory", "胜利", "胜"}:
                return True
            if normalized in {"false", "loss", "lost", "defeat", "失败", "负"}:
                return False
        return None

    def is_enabled(tag_id: str, default: bool = True) -> bool:
        return bool(enabled.get(tag_id, default))

    def add(tag_id: str, label: str, tone: str, title: str) -> None:
        if is_enabled(tag_id):
            tags.append({"id": tag_id, "label": label, "tone": tone, "title": title})

    if show_streaks:
        first_result = next((known_result(match.get("win")) for match in matches if known_result(match.get("win")) is not None), None)
        if first_result is None:
            first_result = None
        streak = 0
        for match in matches:
            result = known_result(match.get("win"))
            if result is None or result != first_result:
                break
            streak += 1
        if first_result is not None and streak >= 3:
            tag_id = "winning-streak" if first_result else "losing-streak"
            add(tag_id, f"{streak} 连{'胜' if first_result else '败'}", "positive" if first_result else "negative", f"最近 {streak} 场结果连续为{'胜利' if first_result else '失败'}。")
    if not show_performance:
        return tags

    sample = len(matches)
    known_results = [known_result(match.get("win")) for match in matches]
    known_results = [result for result in known_results if result is not None]
    win_rate = sum(1 for result in known_results if result) / len(known_results) if known_results else None
    if len(known_results) >= 5 and win_rate is not None and win_rate >= 0.6:
        add("high-win-rate", f"近况强势 {win_rate:.0%}", "positive", f"最近 {sample} 场赢下 {sum(1 for match in matches if match.get('win'))} 场。")
    elif len(known_results) >= 5 and win_rate is not None and win_rate <= 0.4:
        add("high-win-rate", f"近况低迷 {win_rate:.0%}", "negative", f"最近 {sample} 场胜率偏低；标签只描述样本，不代表账号水平。")

    def average(key: str) -> float:
        return sum(float(match.get(key) or 0) for match in matches) / sample

    avg_kda = sum(
        (float(match.get("kills") or 0) + float(match.get("assists") or 0))
        / max(1.0, float(match.get("deaths") or 0))
        for match in matches
    ) / sample
    if avg_kda >= 4:
        add("great-kda", f"高 KDA {avg_kda:.1f}", "positive", f"最近 {sample} 场平均 (击杀+助攻)/死亡为 {avg_kda:.2f}。")
    elif avg_kda <= 1.5:
        add("low-kda", f"KDA {avg_kda:.1f}", "negative", f"最近 {sample} 场平均 (击杀+助攻)/死亡为 {avg_kda:.2f}。")

    cs_minutes = [
        float(match.get("cs") or 0) / max(1.0, float(match.get("duration_seconds") or 0) / 60)
        for match in matches
        if float(match.get("duration_seconds") or 0) > 0
    ]
    avg_cs = sum(cs_minutes) / len(cs_minutes) if cs_minutes else 0
    if avg_cs >= 7.5 and is_enabled("average-cs-per-minute", False):
        add("average-cs-per-minute", f"补刀 {avg_cs:.1f}/分", "info", f"按 {len(cs_minutes)} 场有效时长样本计算，平均每分钟补刀 {avg_cs:.2f}。")
    avg_vision = average("vision_score")
    if avg_vision >= 35 and is_enabled("average-vision-score", False):
        add("average-vision-score", f"视野 {avg_vision:.0f}", "info", f"最近 {sample} 场平均视野得分 {avg_vision:.2f}。")
    solo_kills = [float((match.get("challenges") or {}).get("soloKills") or 0) for match in matches]
    if sum(solo_kills) / sample >= 1:
        avg_solo_kills = sum(solo_kills) / sample
        add("solo-kills", f"场均单杀 {avg_solo_kills:.1f}", "warning", f"最近 {sample} 场的单杀字段平均为 {avg_solo_kills:.2f}。")

    team_rows = []
    for match in matches:
        team = [row for row in (match.get("participants") or []) if str(row.get("team_id")) == str(match.get("team_id"))]
        if not team:
            continue
        def share(key: str) -> float:
            total = sum(float(row.get(key) or 0) for row in team)
            return float(match.get(key) or 0) / total if total > 0 else 0
        damage_share, taken_share, gold_share = share("damage"), share("damage_taken"), share("gold")
        team_kills = sum(float(row.get("kills") or 0) for row in team)
        kill_share = float(match.get("kills") or 0) / team_kills if team_kills > 0 else 0
        team_rows.append({
            "damage": damage_share,
            "taken": taken_share,
            "gold": gold_share,
            "damage_gold": float(match.get("damage") or 0) / max(1.0, float(match.get("gold") or 0)),
            "kill_damage": kill_share / damage_share if damage_share > 0 else 1,
        })
    if team_rows:
        mean = lambda key: sum(row[key] for row in team_rows) / len(team_rows)
        metric_tags = (
            ("average-team-damage", f"团队输出 {mean('damage'):.0%}", "warning", f"{len(team_rows)} 场中平均承担团队英雄伤害的 {mean('damage'):.2%}。"),
            ("average-team-damage-taken", f"团队承伤 {mean('taken'):.0%}", "positive", f"{len(team_rows)} 场中平均承担团队承伤的 {mean('taken'):.2%}。"),
            ("average-team-gold", f"团队经济 {mean('gold'):.0%}", "warning", f"{len(team_rows)} 场中平均获得团队经济的 {mean('gold'):.2%}。"),
            ("average-damage-gold-efficiency", f"伤害/经济 {mean('damage_gold'):.2f}", "warning", f"每 1 点金币对应 {mean('damage_gold'):.3f} 点英雄伤害。"),
        )
        for tag_id, label, tone, title in metric_tags:
            if is_enabled(tag_id, False):
                add(tag_id, label, tone, title)
        kde = mean("kill_damage")
        if is_enabled("average-kill-damage-efficiency") and (kde > 1.35 or kde < .65):
            add("average-kill-damage-efficiency", "收割效率高" if kde > 1.35 else "收割效率低", "positive" if kde > 1.35 else "negative", f"击杀占比 ÷ 伤害占比为 {kde:.2f}；高值偏收割，低值偏消耗。")

    flash_d = sum(1 for match in matches if int(match.get("spell1_id") or 0) == 4)
    flash_f = sum(1 for match in matches if int(match.get("spell2_id") or 0) == 4)
    if flash_d and flash_f:
        add("suspicious-flash-position", "闪现键位不固定", "warning", f"样本中闪现位于 D 键 {flash_d} 场、F 键 {flash_f} 场。")

    akari_score = _ongoing_akari_score(matches)
    if sample >= 8 and akari_score >= 8:
        add("great-performance", "超凡发挥", "positive", f"{sample} 场 Akari Score 为 {akari_score:.2f}，达到超凡阈值 8.00。")
    elif sample >= 5 and akari_score >= 6.5:
        add("great-performance", "亮眼发挥", "positive", f"{sample} 场 Akari Score 为 {akari_score:.2f}，达到亮眼阈值 6.50。")
    if is_enabled("akari-score", False):
        add("akari-score", f"Akari {akari_score:.2f}", "info", f"基于 KDA、胜率、输出、承伤、治疗、补刀、经济、参团和视野的聚合评分，样本 {sample} 场。")
    return tags[:12]


def _ongoing_rating_summary(matches: list[dict]) -> dict:
    """Aggregate the evidence used by LeagueAkari's rating message preset.

    Metrics that require complete team rows are omitted when those rows are not
    available. Solo kills stay ``None`` unless every sampled match exposes the
    challenge field, matching upstream's non-fabrication rule.
    """
    if not matches:
        return {
            "sample_count": 0,
            "win_rate": None,
            "avg_kda": None,
            "avg_solo_kills": None,
            "avg_vision_score": None,
            "avg_champion_damage_percentage_of_team": None,
            "avg_damage_taken_percentage_of_team": None,
            "avg_gold_percentage_of_team": None,
            "avg_cs_per_minute": None,
            "avg_kill_participation": None,
            "avg_damage_gold_efficiency": None,
            "main_champions": [],
            "main_positions": [],
        }

    def number(value) -> float:
        try:
            return float(value or 0)
        except (TypeError, ValueError):
            return 0.0

    sample = len(matches)
    timed = [match for match in matches if number(match.get("duration_seconds")) > 0]
    team_metrics = []
    for match in matches:
        team = [
            row for row in (match.get("participants") or [])
            if str(row.get("team_id")) == str(match.get("team_id"))
        ]
        if not team:
            continue
        team_damage = sum(number(row.get("damage")) for row in team)
        team_taken = sum(number(row.get("damage_taken")) for row in team)
        team_gold = sum(number(row.get("gold")) for row in team)
        team_kills = sum(number(row.get("kills")) for row in team)
        team_metrics.append({
            "damage": number(match.get("damage")) / team_damage if team_damage > 0 else None,
            "taken": number(match.get("damage_taken")) / team_taken if team_taken > 0 else None,
            "gold": number(match.get("gold")) / team_gold if team_gold > 0 else None,
            "kill_participation": (number(match.get("kills")) + number(match.get("assists"))) / team_kills if team_kills > 0 else None,
        })

    def average(values):
        finite = [number(value) for value in values if value is not None]
        return sum(finite) / len(finite) if finite else None

    solo_values = []
    solo_complete = True
    for match in matches:
        challenges = match.get("challenges") or {}
        if "soloKills" not in challenges:
            solo_complete = False
            break
        solo_values.append(number(challenges.get("soloKills")))

    champion_counts: dict[int, int] = {}
    champion_names: dict[int, str] = {}
    position_counts: dict[str, int] = {}
    for match in matches:
        champion_id = int(match.get("champion_id") or 0)
        if champion_id > 0:
            champion_counts[champion_id] = champion_counts.get(champion_id, 0) + 1
            champion_names[champion_id] = str(match.get("champion_name") or champion_id)
        position = str(match.get("position") or "").upper()
        if position:
            position_counts[position] = position_counts.get(position, 0) + 1

    main_champions = [
        {"champion_id": champion_id, "champion_name": champion_names[champion_id], "count": count}
        for champion_id, count in sorted(champion_counts.items(), key=lambda item: (-item[1], item[0]))[:3]
    ]
    position_order = {"TOP": 0, "JUNGLE": 1, "MIDDLE": 2, "MID": 2, "BOTTOM": 3, "UTILITY": 4}
    main_positions = [
        {"position": position, "count": count}
        for position, count in sorted(position_counts.items(), key=lambda item: (-item[1], position_order.get(item[0], 99)))[:2]
    ]
    known_results = [
        match.get("win")
        for match in matches
        if isinstance(match.get("win"), bool)
    ]
    return {
        "sample_count": sample,
        "win_rate": (
            sum(1 for result in known_results if result) / len(known_results)
            if known_results else None
        ),
        "known_result_count": len(known_results),
        "unknown_result_count": sample - len(known_results),
        "avg_kda": average([
            (number(match.get("kills")) + number(match.get("assists"))) / max(1.0, number(match.get("deaths")))
            for match in matches
        ]),
        "avg_solo_kills": average(solo_values) if solo_complete else None,
        "avg_vision_score": average([match.get("vision_score") for match in matches]),
        "avg_champion_damage_percentage_of_team": average([row["damage"] for row in team_metrics]),
        "avg_damage_taken_percentage_of_team": average([row["taken"] for row in team_metrics]),
        "avg_gold_percentage_of_team": average([row["gold"] for row in team_metrics]),
        "avg_cs_per_minute": average([
            number(match.get("cs")) / (number(match.get("duration_seconds")) / 60.0)
            for match in timed
        ]),
        "avg_kill_participation": average([row["kill_participation"] for row in team_metrics]),
        "avg_damage_gold_efficiency": average([
            number(match.get("damage")) / max(1.0, number(match.get("gold"))) for match in matches
        ]),
        "main_champions": main_champions,
        "main_positions": main_positions,
    }


def _ongoing_analysis_payload(players: list[dict], team_tags: list[dict] | None = None) -> dict:
    """Return the compact analysis contract consumed by LeagueAkari's cards.

    The upstream panel keeps the raw match rows separate from its aggregated
    ``analysis.players``/``analysis.teams`` objects.  The desktop port already
    exposes the raw rows as ``recent_matches``; this helper adds the same
    stable, read-only aggregate shape without making the React layer guess at
    missing data.  Values are derived only from rows which the LCU/SGP loader
    actually returned, so an empty history remains empty evidence.
    """
    analyses: dict[str, dict] = {}

    def known_result(value):
        if isinstance(value, bool):
            return value
        if isinstance(value, str):
            normalized = value.strip().lower()
            if normalized in {"true", "win", "won", "victory", "胜利", "胜"}:
                return True
            if normalized in {"false", "loss", "lost", "defeat", "失败", "负"}:
                return False
        return None

    for player in players:
        puuid = str(player.get("puuid") or "")
        if not puuid:
            continue
        recent = player.get("recent") if isinstance(player.get("recent"), dict) else {}
        summary = player.get("rating_summary") if isinstance(player.get("rating_summary"), dict) else {}
        matches = player.get("recent_matches") if isinstance(player.get("recent_matches"), list) else []
        count = int(recent.get("matches") or len(matches) or 0)
        match_results = [known_result(row.get("win")) for row in matches if isinstance(row, dict)]
        known_count = len([result for result in match_results if result is not None])
        wins = sum(1 for result in match_results if result is True)
        losses = sum(1 for result in match_results if result is False)
        # Preserve an aggregate supplied by the loader when no normalized rows
        # are available, but never turn an unknown result into a loss.
        if not match_results and count:
            wins = max(0, min(count, int(recent.get("wins") or 0)))
            losses = max(0, min(count - wins, int(recent.get("losses") or 0)))
            known_count = wins + losses
        result_streak = 0
        result_streak_value = None
        for row in matches:
            if not isinstance(row, dict):
                continue
            value = known_result(row.get("win"))
            if value is None:
                break
            if result_streak_value is None:
                result_streak_value = value
            if value != result_streak_value:
                break
            result_streak += 1
        winning_streak = result_streak if result_streak_value is True else 0
        losing_streak = result_streak if result_streak_value is False else 0
        win_rate = (wins / known_count) if known_count else None
        akari_score = float(recent.get("akari_score") or 0.0)
        analyses[puuid] = {
            "count": count,
            "knownResultCount": known_count,
            "unknownResultCount": max(0, count - known_count),
            "detailsCount": int(recent.get("details_analyzed") or len(matches) or 0),
            "summary": {
                "avgKda": float(recent.get("average_kda") or summary.get("avg_kda") or 0.0),
                "kills": sum(float(row.get("kills") or 0) for row in matches if isinstance(row, dict)),
                "deaths": sum(float(row.get("deaths") or 0) for row in matches if isinstance(row, dict)),
                "assists": sum(float(row.get("assists") or 0) for row in matches if isinstance(row, dict)),
                "avgSoloKills": summary.get("avg_solo_kills"),
                "avgVisionScore": summary.get("avg_vision_score"),
                "avgChampionDamagePercentageOfTeam": summary.get("avg_champion_damage_percentage_of_team"),
                "avgDamageTakenPercentageOfTeam": summary.get("avg_damage_taken_percentage_of_team"),
                "avgGoldPercentageOfTeam": summary.get("avg_gold_percentage_of_team"),
                "avgCsPerMinute": summary.get("avg_cs_per_minute"),
                "avgKillParticipation": summary.get("avg_kill_participation"),
                "avgDamageGoldEfficiency": summary.get("avg_damage_gold_efficiency"),
            },
            "winLoss": {
                "all": {
                    "count": count,
                    "wins": wins,
                    "losses": losses,
                    "winRate": win_rate,
                    "winningStreak": winning_streak,
                    "losingStreak": losing_streak,
                }
            },
            "akariScore": {
                "total": akari_score,
                "outstanding": akari_score >= 6.5,
                "extraordinary": akari_score >= 8.0,
            },
            "champions": {},
            "positions": {},
            "details": {},
        }
        for row in matches:
            if not isinstance(row, dict):
                continue
            champion_id = int(row.get("champion_id") or 0)
            position = str(row.get("position") or "").upper()
            if champion_id:
                champion = analyses[puuid]["champions"].setdefault(
                    str(champion_id),
                    {"championId": champion_id, "count": 0, "wins": 0},
                )
                champion["count"] += 1
                champion["wins"] += int(known_result(row.get("win")) is True)
            if position:
                analyses[puuid]["positions"][position] = analyses[puuid]["positions"].get(position, 0) + 1

    teams: dict[str, dict] = {}
    for player in players:
        team_key = str(player.get("team") or "UNKNOWN")
        puuid = str(player.get("puuid") or "")
        analysis = analyses.get(puuid)
        if not analysis:
            continue
        bucket = teams.setdefault(team_key, {"players": [], "wins": 0, "games": 0, "knownGames": 0, "unknownGames": 0, "kills": 0, "deaths": 0, "assists": 0})
        bucket["players"].append(puuid)
        all_stats = analysis["winLoss"]["all"]
        bucket["wins"] += int(all_stats["wins"])
        bucket["games"] += int(all_stats["count"])
        bucket["knownGames"] += int(analysis.get("knownResultCount") or 0)
        bucket["unknownGames"] += int(analysis.get("unknownResultCount") or 0)
        bucket["kills"] += float(analysis["summary"]["kills"])
        bucket["deaths"] += float(analysis["summary"]["deaths"])
        bucket["assists"] += float(analysis["summary"]["assists"])
    for bucket in teams.values():
        bucket["avgWinRate"] = bucket["wins"] / bucket["knownGames"] if bucket["knownGames"] else None
        bucket["winRateEvidence"] = {
            "wins": bucket["wins"],
            "knownGames": bucket["knownGames"],
            "unknownGames": bucket["unknownGames"],
            "sampleSufficient": bucket["knownGames"] >= 5,
        }
        bucket["avgKda"] = (bucket["kills"] + bucket["assists"]) / max(1.0, bucket["deaths"])

    for tag in team_tags or []:
        group_players = set(tag.get("players") or [])
        for team in teams.values():
            if group_players.intersection(team["players"]):
                team.setdefault("winRateTeams", []).append(tag)

    return {"players": analyses, "teams": teams}


def _ongoing_win_rate_team_tags(players: list[dict]) -> list[dict]:
    """Return LeagueAkari's evidence-backed premade win/loss team tags.

    The thresholds intentionally match the upstream TeamTagsArea constants:
    a win-rate team needs at least three premade members, one member with at
    least 13 known games and >=90% wins, and an average winning streak of four
    for the remaining members.  A loss-rate team needs at least two members,
    each with at least two known games and <=25% wins.  Missing/unknown
    results never qualify a tag.
    """
    by_group: dict[int, list[dict]] = {}
    for player in players:
        group_id = player.get("premade_group")
        if group_id is None:
            continue
        by_group.setdefault(int(group_id), []).append(player)

    result: list[dict] = []
    for group_id, members in sorted(by_group.items()):
        analyses = []
        for player in members:
            recent = player.get("recent") if isinstance(player.get("recent"), dict) else {}
            summary = player.get("rating_summary") if isinstance(player.get("rating_summary"), dict) else {}
            matches = player.get("recent_matches") if isinstance(player.get("recent_matches"), list) else []
            known = [row.get("win") for row in matches if isinstance(row, dict) and isinstance(row.get("win"), bool)]
            known_count = len(known)
            wins = sum(1 for value in known if value)
            # Preserve a loader aggregate only when it explicitly provides a
            # known sample count.  Do not infer unknown rows as losses.
            if not known and int(summary.get("known_result_count") or 0) > 0:
                known_count = int(summary["known_result_count"])
                wins = int(round(float(summary.get("win_rate") or 0) * known_count))
            rate = wins / known_count if known_count else None
            streak = 0
            for row in matches:
                if not isinstance(row, dict) or not isinstance(row.get("win"), bool):
                    break
                if row["win"]:
                    streak += 1
                else:
                    break
            analyses.append({"puuid": player.get("puuid"), "known": known_count, "win_rate": rate, "winning_streak": streak})
        if len(analyses) >= 3:
            high = next((row for row in analyses if row["known"] >= 13 and row["win_rate"] is not None and row["win_rate"] >= 0.9), None)
            if high:
                others = [row for row in analyses if row is not high]
                average_streak = sum(row["winning_streak"] for row in others) / len(others) if others else 0
                if average_streak >= 4:
                    result.append({
                        "premade_id": group_id,
                        "type": "win-rate-team",
                        "players": [row["puuid"] for row in analyses],
                        "evidence": {
                            "min_size": 3,
                            "high_win_rate_member": {"puuid": high["puuid"], "known_matches": high["known"], "win_rate": high["win_rate"]},
                            "other_members_average_winning_streak": average_streak,
                        },
                    })
        if len(analyses) >= 2 and all(
            row["known"] >= 2 and row["win_rate"] is not None and row["win_rate"] <= 0.25
            for row in analyses
        ):
            result.append({
                "premade_id": group_id,
                "type": "loss-rate-team",
                "players": [row["puuid"] for row in analyses],
                "evidence": {
                    "min_size": 2,
                    "members": [{"puuid": row["puuid"], "known_matches": row["known"], "win_rate": row["win_rate"]} for row in analyses],
                },
            })
    return result


def _ongoing_jungle_main_champions(matches: list[dict], names: dict[int, str]) -> list[dict]:
    counts: dict[int, int] = {}
    for match in matches:
        champion_id = int(match.get("champion_id") or 0)
        position = str(match.get("position") or "").upper()
        spells = {int(match.get("spell1_id") or 0), int(match.get("spell2_id") or 0)}
        if champion_id > 0 and (position == "JUNGLE" or 11 in spells):
            counts[champion_id] = counts.get(champion_id, 0) + 1
    return [
        {"champion_id": champion_id, "champion_name": names.get(champion_id, str(champion_id)), "count": count}
        for champion_id, count in sorted(counts.items(), key=lambda item: (-item[1], item[0]))[:3]
    ]


def _ongoing_akari_score(matches: list[dict]) -> float:
    """LeagueAkari MIT scoring model adapted for live-card ordering."""
    if not matches:
        return 0.0

    def number(value) -> float:
        try:
            return float(value or 0)
        except (TypeError, ValueError):
            return 0.0

    def clamp(value: float, minimum: float, maximum: float) -> float:
        return max(minimum, min(maximum, value))

    def linear(value: float, minimum: float, maximum: float, cap: float) -> float:
        return (clamp(value, minimum, maximum) - minimum) / (maximum - minimum) * cap

    part_rows = []
    for match in matches:
        team = [
            row for row in (match.get("participants") or [])
            if str(row.get("team_id")) == str(match.get("team_id"))
        ]
        count = len(team)
        def total(key: str) -> float:
            return sum(number(row.get(key)) for row in team)

        def expected(value: float, team_value: float, full: float, cap: float) -> float:
            if count <= 1 or team_value <= 0:
                return 0.0
            return linear(value / team_value / (1 / count), 1, full, cap)
        duration_minutes = max(1.0, number(match.get("duration_seconds")) / 60)
        team_taken = total("damage_taken")
        part_rows.append({
            "damage": expected(number(match.get("damage")), total("damage"), 2, 3),
            "damage_taken": expected(number(match.get("damage_taken")), team_taken, 2, 2),
            "healing": linear(
                number(match.get("healing")) / max(1.0, team_taken / max(1, count)),
                0.2,
                1 if count == 1 else 1.4,
                2,
            ),
            "cs": linear(number(match.get("cs")) / duration_minutes, 5, 10, 2),
            "gold": expected(number(match.get("gold")), total("gold"), 1.5, 2),
            "participation": linear(
                (number(match.get("kills")) + number(match.get("assists"))) / max(1.0, total("kills")),
                0.3,
                1,
                2,
            ),
            "vision": expected(number(match.get("vision_score")), total("vision_score"), 2, 2),
        })
    kills = sum(number(row.get("kills")) for row in matches)
    deaths = sum(number(row.get("deaths")) for row in matches)
    assists = sum(number(row.get("assists")) for row in matches)
    kda = (kills + assists) / max(1.0, deaths)
    known_results = [row.get("win") for row in matches if isinstance(row.get("win"), bool)]
    win_rate = sum(1 for result in known_results if result) / len(known_results) if known_results else 0.5
    def mean(key: str) -> float:
        return sum(row[key] for row in part_rows) / len(part_rows)
    score = (
        clamp(max(kda - 2, 0) ** 0.5 * (3 / 7), 0, 1)
        + linear(win_rate, 0.5, 1, 1)
        + sum(mean(key) for key in ("damage", "damage_taken", "healing", "cs", "gold", "participation", "vision"))
    )
    return round(score, 2)


_JUNGLE_ANALYSIS_MINUTES = 14
_JUNGLE_KILL_WEIGHT = 5
_JUNGLE_CAMPS = (
    {"x": 3830, "y": 7880, "camp": "blue", "side": "blue"},
    {"x": 3800, "y": 6440, "camp": "wolves", "side": "blue"},
    {"x": 7760, "y": 4010, "camp": "red", "side": "blue"},
    {"x": 6970, "y": 5460, "camp": "raptors", "side": "blue"},
    {"x": 10990, "y": 7000, "camp": "blue", "side": "red"},
    {"x": 11020, "y": 8440, "camp": "wolves", "side": "red"},
    {"x": 7060, "y": 10870, "camp": "red", "side": "red"},
    {"x": 7850, "y": 9420, "camp": "raptors", "side": "red"},
)


def _classify_jungle_map_zone(x: float, y: float) -> str:
    if x < 5000 and y > 9000:
        return "top"
    if x > 9000 and y < 5000:
        return "bot"
    if abs(y - x) <= 3500:
        return "mid"
    return "top" if y > x else "bot"


def _classify_jungle_gank_lane(x: float, y: float) -> str | None:
    if x < 5000 and y > 9000:
        return "top"
    if x > 9000 and y < 5000:
        return "bot"
    midpoint = (x + y) / 2
    if abs(y - x) < 4000 and 3000 < midpoint < 12000:
        return "mid"
    return None


def _detect_jungle_start_camp(x: float, y: float) -> dict:
    nearest = min(_JUNGLE_CAMPS, key=lambda camp: (x - camp["x"]) ** 2 + (y - camp["y"]) ** 2)
    return {"camp": nearest["camp"], "side": nearest["side"]}


def _timeline_participant_frame(frame: dict, participant_id: int) -> dict:
    rows = (frame or {}).get("participantFrames") or {}
    if isinstance(rows, dict):
        row = rows.get(str(participant_id), rows.get(participant_id))
        return row if isinstance(row, dict) else {}
    return {}


def _compute_single_jungle_analysis(frames: list[dict], participant_id: int, team_id: int | None = None) -> dict:
    zone_weights = {"top": 0, "mid": 0, "bot": 0}
    kill_zone_weights = {"top": 0, "mid": 0, "bot": 0}
    minute_positions = []
    total_frames = 0
    for minute, frame in enumerate(frames[1 : _JUNGLE_ANALYSIS_MINUTES + 1], start=1):
        participant_frame = _timeline_participant_frame(frame, participant_id)
        position = participant_frame.get("position") or {}
        if position.get("x") is None or position.get("y") is None:
            continue
        x, y = float(position["x"]), float(position["y"])
        lane = _classify_jungle_map_zone(x, y)
        zone_weights[lane] += 1
        total_frames += 1
        minute_positions.append({"x": x, "y": y, "lane": lane, "minute": minute})

    ganks = {"top": 0, "mid": 0, "bot": 0}
    gank_positions, level3_positions, level4_positions = [], [], []
    kill_weight_total = 0
    objectives = {"dragons": 0, "voidgrubs": 0, "heralds": 0, "barons": 0, "first_dragon": None, "first_dragon_time_ms": None}
    first_dragon_seen = False
    for frame in frames:
        for event in (frame or {}).get("events") or []:
            if not isinstance(event, dict):
                continue
            if event.get("type") == "ELITE_MONSTER_KILL":
                monster = str(event.get("monsterType") or "").upper()
                subtype = str(event.get("monsterSubType") or "").upper()
                killer = int(event.get("killerId") or 0)
                killer_team = int(event.get("killerTeamId") or (100 if 1 <= killer <= 5 else 200))
                own_team = int(team_id or (100 if 1 <= participant_id <= 5 else 200))
                involved = killer_team == own_team
                if monster == "DRAGON":
                    if not first_dragon_seen:
                        first_dragon_seen = True
                        objectives["first_dragon"] = involved
                    if involved:
                        objectives["dragons"] += 1
                        if objectives["first_dragon_time_ms"] is None:
                            objectives["first_dragon_time_ms"] = int(event.get("timestamp") or 0)
                elif involved and (monster in {"HORDE", "VOID_GRUB", "VOIDGRUB"} or "HORDE" in subtype or "VOID" in subtype):
                    objectives["voidgrubs"] += 1
                elif involved and monster in {"RIFTHERALD", "RIFT_HERALD"}:
                    objectives["heralds"] += 1
                elif involved and monster in {"BARON_NASHOR", "BARON"}:
                    objectives["barons"] += 1
                continue
            if event.get("type") != "CHAMPION_KILL":
                continue
            timestamp = int(event.get("timestamp") or 0)
            if timestamp > _JUNGLE_ANALYSIS_MINUTES * 60 * 1000:
                continue
            assists = event.get("assistingParticipantIds") or []
            if event.get("killerId") != participant_id and participant_id not in assists:
                continue
            position = event.get("position") or {}
            if position.get("x") is None or position.get("y") is None:
                continue
            x, y = float(position["x"]), float(position["y"])
            zone = _classify_jungle_map_zone(x, y)
            kill_zone_weights[zone] += _JUNGLE_KILL_WEIGHT
            kill_weight_total += _JUNGLE_KILL_WEIGHT
            lane = _classify_jungle_gank_lane(x, y)
            point = {"x": x, "y": y, "lane": lane or zone, "timestamp": timestamp}
            if lane:
                ganks[lane] += 1
                gank_positions.append(point)
            if timestamp <= 180000:
                level3_positions.append(point)
            elif timestamp <= 240000:
                level4_positions.append(point)

    start_camp = None
    if len(frames) > 1:
        position = _timeline_participant_frame(frames[1], participant_id).get("position") or {}
        if position.get("x") is not None and position.get("y") is not None:
            start_camp = _detect_jungle_start_camp(float(position["x"]), float(position["y"]))

    frame3 = _timeline_participant_frame(frames[3], participant_id) if len(frames) > 3 else {}
    frame4 = _timeline_participant_frame(frames[4], participant_id) if len(frames) > 4 else {}
    damage3 = ((frame3.get("damageStats") or {}).get("totalDamageDoneToChampions"))
    damage4 = ((frame4.get("damageStats") or {}).get("totalDamageDoneToChampions"))
    cs3 = int(frame3.get("minionsKilled") or 0) + int(frame3.get("jungleMinionsKilled") or 0)
    level3_gank = 12 <= cs3 < 20 and int(frame3.get("level") or 0) == 3 and (
        (damage3 is not None and float(damage3) > 0) or bool(level3_positions)
    )
    level4_gank = bool(level4_positions) or (
        damage3 is not None and damage4 is not None and float(damage4) > float(damage3)
    )
    combined = {
        lane: zone_weights[lane] + kill_zone_weights[lane]
        for lane in ("top", "mid", "bot")
    }
    return {
        "zone_weights": combined,
        "total_zone_weight": total_frames + kill_weight_total,
        "ganks": ganks,
        "start_camp": start_camp,
        "level3_gank_detected": level3_gank,
        "level4_gank_detected": level4_gank,
        "level3_kill_positions": level3_positions,
        "level4_kill_positions": level4_positions,
        "gank_positions": gank_positions,
        "minute_positions": minute_positions,
        "objectives": objectives,
    }


def _aggregate_jungle_analyses(samples: list[dict]) -> dict | None:
    if not samples:
        return None
    total_weight = sum(int(sample.get("total_zone_weight") or 0) for sample in samples)
    zone_weights = {
        lane: sum(int((sample.get("zone_weights") or {}).get(lane) or 0) for sample in samples)
        for lane in ("top", "mid", "bot")
    }
    ganks = {
        lane: sum(int((sample.get("ganks") or {}).get(lane) or 0) for sample in samples)
        for lane in ("top", "mid", "bot")
    }
    camp_counts: dict[str, int] = {}
    for sample in samples:
        start = sample.get("start_camp") or {}
        if start.get("camp") and start.get("side"):
            key = f'{start["side"]}:{start["camp"]}'
            camp_counts[key] = camp_counts.get(key, 0) + 1
    games = len(samples)
    preferred_lane = max(zone_weights, key=zone_weights.get) if total_weight else "unknown"
    preferred_camp = max(camp_counts, key=camp_counts.get) if camp_counts else "unknown"
    zone_percentages = {
        lane: round(zone_weights[lane] / total_weight, 4) if total_weight else 0
        for lane in ("top", "mid", "bot")
    }
    average_ganks = {lane: round(ganks[lane] / games, 2) for lane in ("top", "mid", "bot")}
    objective_rows = [sample.get("objectives") or {} for sample in samples]
    known_first_dragons = [
        bool(row.get("first_dragon"))
        for row in objective_rows if row.get("first_dragon") is not None
    ]
    first_dragon_times = [
        float(row.get("first_dragon_time_ms")) / 1000.0
        for row in objective_rows if row.get("first_dragon_time_ms") is not None
    ]
    objectives = {
        "first_dragon_rate": (
            sum(known_first_dragons) / len(known_first_dragons)
            if known_first_dragons else None
        ),
        "avg_first_dragon_time_seconds": round(sum(first_dragon_times) / len(first_dragon_times), 1) if first_dragon_times else None,
        "avg_dragons": round(sum(int(row.get("dragons") or 0) for row in objective_rows) / games, 2),
        "avg_voidgrubs": round(sum(int(row.get("voidgrubs") or 0) for row in objective_rows) / games, 2),
        "avg_heralds": round(sum(int(row.get("heralds") or 0) for row in objective_rows) / games, 2),
        "avg_barons": round(sum(int(row.get("barons") or 0) for row in objective_rows) / games, 2),
    }
    level3_rate = round(sum(bool(sample.get("level3_gank_detected")) for sample in samples) / games, 4)
    level4_rate = round(sum(bool(sample.get("level4_gank_detected")) for sample in samples) / games, 4)
    lane_labels = {"top": "上半区", "mid": "中路", "bot": "下半区", "unknown": "未知区域"}
    camp_labels = {"blue": "蓝 BUFF", "red": "红 BUFF", "wolves": "三狼", "raptors": "F6", "unknown": "未知营地"}
    camp_side, _, camp_name = preferred_camp.partition(":")
    side_label = {"blue": "蓝色方野区", "red": "红色方野区"}.get(camp_side, "")
    draft = (
        f"近 {games} 场打野时间线：首开偏好 {side_label}{camp_labels.get(camp_name, camp_name or '未知营地')}；"
        f"前 14 分钟活动更偏 {lane_labels.get(preferred_lane, preferred_lane)}；"
        f"3 分钟内参与击杀率 {level3_rate * 100:.0f}%，4 分钟内新增参与率 {level4_rate * 100:.0f}%。"
    )
    return {
        "games_analyzed": games,
        "zone_percentages": zone_percentages,
        "average_ganks": average_ganks,
        "early_gank": {
            "level3_rate": level3_rate,
            "level4_rate": level4_rate,
        },
        "objectives": objectives,
        "start_camps": camp_counts,
        "preferred_lane": preferred_lane,
        "preferred_start_camp": preferred_camp,
        "draft": draft,
        "samples": samples,
    }


def _history_games(payload: dict) -> list[dict]:
    rows = (payload or {}).get("games") or []
    if isinstance(rows, dict):
        rows = rows.get("games") or []
    result = []
    for wrapper in rows if isinstance(rows, list) else []:
        game = wrapper.get("json") if isinstance(wrapper, dict) and isinstance(wrapper.get("json"), dict) else wrapper
        if isinstance(game, dict):
            result.append(game)
    return result


def _jungle_game_participant(game: dict, puuid: str) -> dict | None:
    identities = game.get("participantIdentities") or []
    participant_id = next(
        (
            row.get("participantId")
            for row in identities
            if str((row.get("player") or {}).get("puuid") or row.get("puuid") or "") == puuid
        ),
        None,
    )
    participant = next(
        (
            row
            for row in (game.get("participants") or [])
            if str(row.get("puuid") or "") == puuid
            or (participant_id is not None and row.get("participantId") == participant_id)
        ),
        None,
    )
    if not isinstance(participant, dict):
        return None
    position = str(
        participant.get("teamPosition")
        or participant.get("individualPosition")
        or (participant.get("timeline") or {}).get("lane")
        or ""
    ).upper()
    spells = {
        int(value)
        for value in (
            participant.get("spell1Id"),
            participant.get("spell2Id"),
            participant.get("summoner1Id"),
            participant.get("summoner2Id"),
        )
        if value is not None
    }
    if position != "JUNGLE" and 11 not in spells:
        return None
    return participant


async def _load_jungle_analysis(
    puuid: str,
    history: dict,
    *,
    limit: int = 6,
    server_id: str | None = None,
    prefer_sgp: bool = False,
    semaphore: asyncio.Semaphore | None = None,
) -> dict:
    candidates = []
    for game in _history_games(history):
        participant = _jungle_game_participant(game, puuid)
        game_id = game.get("gameId") or game.get("game_id")
        if participant and game_id:
            candidates.append((int(game_id), int(participant.get("participantId") or 0), game))
        if len(candidates) >= max(1, min(limit, 10)):
            break

    async def analyze(entry):
        game_id, participant_id, game = entry
        timeline = None
        source = "sgp" if prefer_sgp else "lcu"

        async def limited(coro):
            if semaphore is None:
                return await coro
            async with semaphore:
                return await coro

        if prefer_sgp:
            try:
                timeline = await limited(_sgp_game_details(game_id, server_id))
            except RuntimeError:
                return None
        else:
            try:
                timeline = await limited(
                    league_lab_service.request(
                        "GET", f"/lol-match-history/v1/game-timelines/{game_id}"
                    )
                )
            except RuntimeError:
                try:
                    timeline = await limited(_sgp_game_details(game_id, server_id))
                    source = "sgp"
                except RuntimeError:
                    return None
        if not participant_id:
            participant = next(
                (
                    row
                    for row in (timeline or {}).get("participants") or []
                    if str(row.get("puuid") or "") == puuid
                ),
                None,
            )
            participant_id = int((participant or {}).get("participantId") or 0)
        frames = (timeline or {}).get("frames") or []
        if not participant_id or not isinstance(frames, list) or not frames:
            return None
        game_participant = _jungle_game_participant(game, puuid) or {}
        team_id = int(game_participant.get("teamId") or (100 if 1 <= participant_id <= 5 else 200))
        sample = _compute_single_jungle_analysis(frames, participant_id, team_id)
        sample.update(
            {
                "game_id": game_id,
                "team_id": team_id,
                "source": source,
            }
        )
        return sample

    samples = [sample for sample in await asyncio.gather(*(analyze(row) for row in candidates)) if sample]
    aggregate = _aggregate_jungle_analyses(samples)
    return aggregate or {"games_analyzed": 0, "samples": [], "reason": "最近战绩中没有可用的打野时间线"}


@router.get("/status")
async def league_lab_status():
    return await league_lab_service.snapshot()


async def _lcu_client_public_identity(credentials: LcuCredentials) -> dict:
    async def read(path: str):
        try:
            async with httpx.AsyncClient(verify=False, timeout=2.0) as client:
                response = await client.get(
                    f"{credentials.base_url}{path}",
                    headers={"Authorization": credentials.auth_header},
                )
            response.raise_for_status()
            return response.json() if response.content else None
        except (httpx.HTTPError, ValueError):
            return None

    summoner, phase = await asyncio.gather(
        read("/lol-summoner/v1/current-summoner"),
        read("/lol-gameflow/v1/gameflow-phase"),
    )
    summoner = summoner if isinstance(summoner, dict) else {}
    return {
        "pid": credentials.pid,
        "selected": credentials.pid == league_lab_service._selected_client_pid,
        "game_name": summoner.get("gameName") or summoner.get("displayName") or "",
        "tag_line": summoner.get("tagLine") or "",
        "profile_icon_id": summoner.get("profileIconId"),
        "summoner_level": summoner.get("summonerLevel"),
        "phase": str(phase or ""),
        "region": credentials.region,
        "platform_id": credentials.platform_id,
        "has_riot_client": bool(credentials.riot_client_port and credentials.riot_client_token),
    }


@router.get("/clients")
async def league_clients():
    clients = await discover_lcu_clients()
    league_lab_service._available_clients = clients
    rows = await asyncio.gather(*(_lcu_client_public_identity(client) for client in clients))
    return {
        "clients": rows,
        "count": len(rows),
        "selected_pid": league_lab_service._selected_client_pid,
    }


@router.post("/clients/select")
async def select_league_client(body: LeagueClientSelect):
    try:
        await league_lab_service.select_client(body.pid)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return league_lab_service.status()


@router.get("/installations")
async def league_client_installations():
    rows = list((await detect_client_installations()).values())
    return {"installations": rows, "count": len(rows)}


@router.post("/installations/launch")
async def launch_league_client(body: LeagueClientLaunch):
    try:
        return await launch_detected_client(body.kind)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc


@router.put("/settings")
async def update_league_lab_settings(body: LeagueLabSettings):
    league_lab_service.update_settings(body)
    return league_lab_service.status()


@router.get("/matches")
async def league_match_history(limit: int = 20, beg_index: int = 0):
    requested_limit = max(1, min(limit, 40))
    requested_start = max(0, int(beg_index or 0))
    try:
        payload = await league_lab_service.request(
            "GET", "/lol-match-history/v1/products/lol/current-summoner/matches",
            params={"begIndex": requested_start, "endIndex": requested_start + requested_limit - 1},
        )
        names = await _champion_names()
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    payload = await _complete_lcu_match_history(payload)
    current_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    raw_games = (payload.get("games") or {}).get("games") if isinstance(payload, dict) and isinstance(payload.get("games"), dict) else (payload.get("games") if isinstance(payload, dict) else [])
    raw_games = raw_games if isinstance(raw_games, list) else []
    # Some Tencent LCU builds return one extra row even with an inclusive
    # endIndex. Enforce the user-visible limit after normalization as well.
    normalized = _normalize_match_rows(payload, names, current_puuid)[:requested_limit]
    _index_match_encounters(normalized, current_puuid)
    return {
        "matches": normalized,
        "count": len(normalized),
        "beg_index": requested_start,
        "page_size": requested_limit,
        "page": requested_start // requested_limit + 1,
        "has_more": len(raw_games) >= requested_limit,
        "source": "lcu",
    }


@router.post("/matches/collect")
async def collect_league_match_history(body: LeagueMatchCollectRequest):
    """Collect matching pages without performing any LCU write.

    This is the server-side counterpart of LeagueAkari's Collect mode: each
    iteration asks the LCU for one bounded page, normalizes it to the same
    ten-player matrix used by the cards, applies the nested predicate tree,
    de-duplicates Game IDs, and stops at the requested result or scan budget.
    """
    current_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    if not current_puuid:
        raise HTTPException(status_code=409, detail="当前客户端没有可用账号")
    try:
        names = await _champion_names()
        collected: list[dict] = []
        seen_game_ids: set[str] = set()
        scanned = 0
        iterations = 0
        for iteration in range(max(1, int(body.max_iteration))):
            payload = await league_lab_service.request(
                "GET",
                "/lol-match-history/v1/products/lol/current-summoner/matches",
                params={
                    "begIndex": iteration * int(body.count_per_iteration),
                    "endIndex": (iteration + 1) * int(body.count_per_iteration) - 1,
                },
            )
            payload = await _complete_lcu_match_history(payload)
            page_rows = _normalize_match_rows(payload, names, current_puuid)
            if not page_rows:
                break
            scanned += len(page_rows)
            for row in page_rows:
                game_id = str(row.get("game_id") or "")
                if not game_id or game_id in seen_game_ids or not _matches_league_collect_request(row, body):
                    continue
                seen_game_ids.add(game_id)
                collected.append(row)
                if len(collected) >= int(body.expected_count):
                    break
            iterations = iteration + 1
            if len(collected) >= int(body.expected_count) or len(page_rows) < int(body.count_per_iteration):
                break
        collected = collected[: int(body.expected_count)]
        stored_count = await _store_match_collection(current_puuid, collected) if collected else await _match_collection_count(current_puuid)
        _index_match_encounters(collected, current_puuid)
        return {
            "matches": collected,
            "count": len(collected),
            "expected_count": int(body.expected_count),
            "scanned_games_count": scanned,
            "iterations": iterations,
            "collection_count": stored_count,
            "source": "lcu",
            "complete": len(collected) >= int(body.expected_count),
        }
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc


@router.get("/replays/{game_id}")
async def league_replay_metadata(game_id: int):
    if game_id <= 0:
        raise HTTPException(status_code=422, detail="无效的游戏 ID")
    try:
        configuration = await league_lab_service.request("GET", "/lol-replays/v1/configuration")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    enabled = bool((configuration or {}).get("isReplaysEnabled")) if isinstance(configuration, dict) else False
    metadata = None
    if enabled:
        try:
            metadata = await league_lab_service.request("GET", f"/lol-replays/v1/metadata/{game_id}")
        except RuntimeError:
            metadata = None
    return {
        "enabled": enabled,
        "metadata": metadata if isinstance(metadata, dict) else {"gameId": game_id, "state": "download", "downloadProgress": 0},
        "configuration": {
            "game_version": (configuration or {}).get("gameVersion") if isinstance(configuration, dict) else "",
            "is_playing_game": bool((configuration or {}).get("isPlayingGame")) if isinstance(configuration, dict) else False,
            "is_playing_replay": bool((configuration or {}).get("isPlayingReplay")) if isinstance(configuration, dict) else False,
        },
    }


async def _prepare_league_replay(game_id: int, body: LeagueReplayPrepare) -> None:
    configuration = await league_lab_service.request("GET", "/lol-replays/v1/configuration")
    if not isinstance(configuration, dict) or not configuration.get("isReplaysEnabled"):
        raise RuntimeError("当前 League 客户端未启用比赛回放")
    game_version = body.game_version or str(configuration.get("gameVersion") or "")
    await league_lab_service.request(
        "POST",
        f"/lol-replays/v2/metadata/{game_id}/create",
        json_body={
            "gameVersion": game_version,
            "gameType": body.game_type,
            "queueId": body.queue_id,
            "gameEnd": body.game_end,
        },
    )


@router.post("/replays/{game_id}/download")
async def download_league_replay(game_id: int, body: LeagueReplayPrepare):
    if game_id <= 0:
        raise HTTPException(status_code=422, detail="无效的游戏 ID")
    try:
        await _prepare_league_replay(game_id, body)
        await league_lab_service.request(
            "POST",
            f"/lol-replays/v1/rofls/{game_id}/download",
            json_body={"componentType": "replay-button_match-history"},
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"game_id": game_id, "state": "downloading"}


@router.post("/replays/{game_id}/watch")
async def watch_league_replay(game_id: int):
    if game_id <= 0:
        raise HTTPException(status_code=422, detail="无效的游戏 ID")
    try:
        await league_lab_service.request(
            "POST",
            f"/lol-replays/v1/rofls/{game_id}/watch",
            json_body={"componentType": "replay-button_match-history"},
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"game_id": game_id, "state": "watching"}


@router.get("/players/current")
async def current_league_player():
    try:
        summoner = await league_lab_service.request("GET", "/lol-summoner/v1/current-summoner")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return await _load_player_bundle(summoner)


@router.get("/players/search")
async def search_league_player(game_name: str, tag_line: str, server_id: str = ""):
    if not game_name.strip() or not tag_line.strip():
        raise HTTPException(status_code=422, detail="请输入完整的游戏名称和标签")
    current_server_id = _sgp_server_id(league_lab_service.credentials)
    try:
        target_server_id = _normalize_sgp_server_id(server_id) if server_id.strip() else current_server_id
    except RuntimeError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc
    aliases = []
    try:
        aliases = await _riot_player_account_aliases(game_name.strip(), tag_line.strip())
    except RuntimeError:
        try:
            rows = await league_lab_service.request(
                "POST",
                "/lol-summoner/v1/summoners/aliases",
                json_body=[{"gameName": game_name.strip(), "tagLine": tag_line.strip()}],
            )
            aliases = [row for row in (rows or []) if isinstance(row, dict) and row.get("puuid")]
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
    alias = aliases[0] if aliases else None
    if not alias:
        raise HTTPException(status_code=404, detail="未找到该 Riot ID")
    puuid = str(alias["puuid"])
    alias_name = alias.get("alias") or {}
    prefer_sgp = bool(target_server_id and target_server_id != current_server_id)
    summoner = alias if not alias_name and (alias.get("gameName") or alias.get("displayName")) else None
    if not prefer_sgp:
        if not isinstance(summoner, dict):
            try:
                summoner = await league_lab_service.request("GET", f"/lol-summoner/v2/summoners/puuid/{puuid}")
            except RuntimeError:
                pass
    if not isinstance(summoner, dict) or not summoner.get("puuid"):
        try:
            summoner = await _sgp_summoner_by_puuid(puuid, target_server_id or None)
        except RuntimeError as exc:
            raise HTTPException(status_code=404, detail=f"该 Riot ID 在所选区服不存在: {exc}") from exc
    summoner = {
        **summoner,
        "gameName": summoner.get("gameName") or alias_name.get("game_name") or game_name.strip(),
        "tagLine": summoner.get("tagLine") or alias_name.get("tag_line") or tag_line.strip(),
    }
    if server_id.strip():
        bundle = await _load_player_bundle(
            summoner,
            sgp_server_id=target_server_id or None,
            prefer_sgp=prefer_sgp,
        )
    else:
        bundle = await _load_player_bundle(summoner)
    _remember_player_search(bundle)
    return bundle


@router.get("/players/search-servers")
async def league_player_search_servers():
    current = _sgp_server_id(league_lab_service.credentials)
    return {
        "current": current,
        "servers": [
            {"id": server_id, "label": _SGP_SERVER_LABELS.get(server_id, server_id), "current": server_id == current}
            for server_id in _SGP_COMMON_HOSTS
            if server_id in _SGP_MATCH_HISTORY_HOSTS
        ],
    }


@router.get("/players/recent")
async def recent_league_players(limit: int = 40):
    rows = _read_recent_players()[: max(1, min(limit, 200))]
    tags = _read_player_tags()
    public_rows = [{**{key: value for key, value in row.items() if key != "encounters"}, "tag": _find_player_tag(tags, str(row.get("puuid")))} for row in rows]
    return {"players": public_rows, "count": len(public_rows)}


@router.get("/players/search-history")
async def league_player_search_history(limit: int = 40):
    owner_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    rows = [
        {key: value for key, value in row.items() if key != "owner_puuid"}
        for row in _read_player_search_history()
        if str(row.get("owner_puuid") or "") == owner_puuid
    ]
    rows.sort(key=lambda row: (bool(row.get("pinned")), int(row.get("visited_at") or 0)), reverse=True)
    selected = rows[:max(1, min(limit, 100))]
    return {"players": selected, "count": len(selected)}


@router.put("/players/search-history/{puuid}/pin")
async def pin_league_player_search_history(puuid: str, body: SearchHistoryPinBody, server_id: str = ""):
    owner_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    rows = _read_player_search_history()
    found = False
    for row in rows:
        if (
            str(row.get("owner_puuid") or "") == owner_puuid
            and str(row.get("puuid") or "") == puuid
            and str(row.get("server_id") or "") == str(server_id or "")
        ):
            row["pinned"] = body.pinned
            found = True
            break
    if not found:
        raise HTTPException(status_code=404, detail="最近访问记录不存在")
    _write_player_search_history(rows)
    return {"puuid": puuid, "server_id": server_id, "pinned": body.pinned}


@router.delete("/players/search-history/{puuid}")
async def delete_league_player_search_history(puuid: str, server_id: str = ""):
    owner_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    rows = _read_player_search_history()
    remaining = [row for row in rows if not (
        str(row.get("owner_puuid") or "") == owner_puuid
        and str(row.get("puuid") or "") == puuid
        and str(row.get("server_id") or "") == str(server_id or "")
    )]
    removed = len(remaining) != len(rows)
    if removed:
        _write_player_search_history(remaining)
    return {"puuid": puuid, "server_id": server_id, "removed": removed}


def _public_league_friend(friend: dict) -> dict:
    lol = friend.get("lol") if isinstance(friend.get("lol"), dict) else {}
    puuid = str(friend.get("puuid") or lol.get("puuid") or "")
    availability = str(friend.get("availability") or "offline")
    game_status = str(lol.get("gameStatus") or "")
    return {
        "puuid": puuid,
        "game_name": str(friend.get("gameName") or friend.get("name") or ""),
        "tag_line": str(friend.get("gameTag") or ""),
        "profile_icon_id": friend.get("icon") or friend.get("profileIconId"),
        "availability": availability,
        "game_status": game_status,
        "spectatable": bool(
            availability == "dnd"
            and game_status.casefold() == "ingame"
            and puuid
            and lol.get("spectatorKey")
        ),
    }


@router.get("/players/friends")
async def league_player_friends():
    try:
        payload = await league_lab_service.request("GET", "/lol-chat/v1/friends")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    priority = {"dnd": 3, "chat": 2, "away": 1, "offline": 0}
    friends = [_public_league_friend(row) for row in (payload or []) if isinstance(row, dict)]
    friends.sort(key=lambda row: (priority.get(row["availability"], 0), row["game_name"].casefold()), reverse=True)
    return {"friends": friends, "count": len(friends)}


async def _load_auto_invite_friend_rows() -> list[dict]:
    """Return friend choices plus local invitation-schedule state.

    This is intentionally a read-only LCU call.  The caller may use the
    returned PUUIDs to update the local schedule, while the actual invitation
    remains gated by ``automation_enabled`` and the friend event handler.
    """

    payload = await league_lab_service.request("GET", "/lol-chat/v1/friends")
    scheduled = {str(value) for value in league_lab_service.settings.auto_invite_friend_puuids}
    priority = {"dnd": 3, "away": 2, "chat": 1, "offline": 0}
    rows: list[dict] = []
    for friend in payload if isinstance(payload, list) else []:
        if not isinstance(friend, dict):
            continue
        public = _public_league_friend(friend)
        puuid = public["puuid"]
        if not puuid:
            continue
        row = {
            **public,
            "summoner_id": friend.get("summonerId"),
            "scheduled": puuid in scheduled,
            "inviteable": public["availability"] == "chat",
        }
        rows.append(row)
    rows.sort(key=lambda row: (priority.get(row["availability"], 0), row["game_name"].casefold()), reverse=True)
    return rows


@router.get("/automation/invite-friends")
@router.get("/automation/invite-friends/schedule")
async def league_auto_invite_friends():
    """Read friend choices and the locally scheduled invitation PUUIDs."""

    try:
        friends = await _load_auto_invite_friend_rows()
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    scheduled = [row["puuid"] for row in friends if row.get("scheduled")]
    # Preserve scheduled PUUIDs for offline/deleted friends in the explicit
    # list, so a transient friend-list response cannot silently erase a user's
    # appointment.  They will be consumed only if a matching friend event
    # arrives later.
    known = set(scheduled)
    scheduled.extend(
        puuid for puuid in league_lab_service.settings.auto_invite_friend_puuids
        if str(puuid) not in known
    )
    return {
        "friends": friends,
        "count": len(friends),
        "scheduled_puuids": scheduled,
        "automation_enabled": bool(league_lab_service.settings.automation_enabled),
    }


@router.put("/automation/invite-friends")
@router.put("/automation/invite-friends/schedule")
async def schedule_league_auto_invite_friends(body: AutoInviteFriendSchedule):
    """Persist an invitation appointment without calling an LCU write route."""

    requested: list[str] = []
    for raw in body.puuids:
        value = str(raw or "").strip()
        if value and value not in requested:
            requested.append(value)
    if len(requested) > 20:
        raise HTTPException(status_code=422, detail="最多预约 20 位好友")
    try:
        friends = await _load_auto_invite_friend_rows()
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    available = {str(row["puuid"]) for row in friends}
    # A PUUID that was already scheduled may be temporarily absent (offline
    # or a partial LCU refresh); keep it rather than turning a selection UI
    # refresh into an implicit cancellation.
    current = {str(value) for value in league_lab_service.settings.auto_invite_friend_puuids}
    scheduled = [value for value in requested if value in available or value in current]
    league_lab_service.update_settings(
        league_lab_service.settings.model_copy(update={"auto_invite_friend_puuids": scheduled})
    )
    return {
        "scheduled_puuids": scheduled,
        "count": len(scheduled),
        "automation_enabled": bool(league_lab_service.settings.automation_enabled),
    }


@router.delete("/automation/invite-friends/{puuid}")
async def unschedule_league_auto_invite_friend(puuid: str):
    """Remove one local invitation appointment; never touches the LCU."""

    value = str(puuid or "").strip()
    scheduled = [
        str(item) for item in league_lab_service.settings.auto_invite_friend_puuids
        if str(item) != value
    ]
    league_lab_service.update_settings(
        league_lab_service.settings.model_copy(update={"auto_invite_friend_puuids": scheduled})
    )
    return {"puuid": value, "removed": value not in scheduled, "scheduled_puuids": scheduled}


@router.post("/players/friends/{puuid}/spectate")
async def spectate_league_friend(puuid: str):
    _require_toolkit_account_actions()
    try:
        payload = await league_lab_service.request("GET", "/lol-chat/v1/friends")
        friend = next((row for row in (payload or []) if (
            isinstance(row, dict) and _public_league_friend(row)["puuid"] == puuid
        )), None)
        public = _public_league_friend(friend or {})
        if not friend or not public["spectatable"]:
            raise HTTPException(status_code=409, detail="该好友当前不可观战")
        await _require_live_phase({
            "None", "Lobby", "Matchmaking", "ReadyCheck", "ChampSelect",
            "PreEndOfGame", "EndOfGame", "WaitingForStats", "Reconnect",
        })
        await league_lab_service.request(
            "POST",
            "/lol-spectator/v1/spectate/launch",
            json_body={"puuid": puuid, "spectatorKey": (friend.get("lol") or {}).get("spectatorKey")},
        )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"puuid": puuid, "launched": True}


@router.get("/players/{puuid}/encounters")
async def league_player_encounters(puuid: str, page: int = 1, page_size: int = 10):
    self_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    target = next((row for row in _read_recent_players() if str(row.get("puuid") or "") == puuid), None)
    rows = [
        row for row in ((target or {}).get("encounters") or [])
        if isinstance(row, dict) and (not self_puuid or str(row.get("self_puuid") or "") == self_puuid)
    ]
    rows.sort(key=lambda row: _time_sort_value(row.get("played_at") or row.get("seen_at")), reverse=True)
    size = max(1, min(page_size, 30))
    current_page = max(1, page)
    start = (current_page - 1) * size
    return {
        "puuid": puuid,
        "self_puuid": self_puuid,
        "games": rows[start:start + size],
        "page": current_page,
        "page_size": size,
        "total": len(rows),
    }


@router.delete("/players/{puuid}/encounters/{game_id}")
async def delete_league_player_encounter(puuid: str, game_id: str):
    self_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    rows = _read_recent_players()
    removed = False
    for player in rows:
        if str(player.get("puuid") or "") != puuid:
            continue
        before = [row for row in (player.get("encounters") or []) if isinstance(row, dict)]
        after = [row for row in before if not (
            str(row.get("game_id") or "") == game_id and (not self_puuid or str(row.get("self_puuid") or "") == self_puuid)
        )]
        player["encounters"] = after
        removed = len(after) != len(before)
        break
    if removed:
        _write_recent_players(rows)
    return {"removed": removed, "puuid": puuid, "game_id": game_id}


@router.get("/players/{puuid}")
async def league_player_bundle(puuid: str, match_limit: int = 20, beg_index: int = 0, server_id: str = ""):
    try:
        target_server_id = _normalize_sgp_server_id(server_id) if server_id.strip() else ""
    except RuntimeError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc
    prefer_sgp = bool(target_server_id and target_server_id != _sgp_server_id(league_lab_service.credentials))
    lcu_exc = RuntimeError("已选择跨区 SGP 数据源")
    summoner = None
    if not prefer_sgp:
        try:
            summoner = await league_lab_service.request("GET", f"/lol-summoner/v2/summoners/puuid/{puuid}")
        except RuntimeError as exc:
            lcu_exc = exc
    if not isinstance(summoner, dict):
        try:
            summoner = await _sgp_summoner_by_puuid(puuid, target_server_id or None)
        except RuntimeError as sgp_exc:
            raise HTTPException(status_code=409, detail=f"{lcu_exc}; {sgp_exc}") from sgp_exc
    bundle = await _load_player_bundle(
        summoner,
        match_limit=max(1, min(match_limit, 100)),
        beg_index=max(0, beg_index),
        sgp_server_id=target_server_id or None,
        prefer_sgp=prefer_sgp,
    )
    _remember_player_search(bundle)
    return bundle


async def _load_player_bundle(
    summoner,
    match_limit: int = 20,
    beg_index: int = 0,
    sgp_server_id: str | None = None,
    prefer_sgp: bool = False,
) -> dict:
    if not isinstance(summoner, dict) or not summoner.get("puuid"):
        raise HTTPException(status_code=404, detail="未找到召唤师")
    puuid = str(summoner["puuid"])

    async def optional(method: str, path: str, **kwargs):
        try:
            return await league_lab_service.request(method, path, **kwargs)
        except RuntimeError:
            return None

    ranked, mastery, history, names = await asyncio.gather(
        optional("GET", f"/lol-ranked/v1/ranked-stats/{puuid}"),
        optional("POST", f"/lol-champion-mastery/v1/{puuid}/champion-mastery/top", json_body={"skipCache": True}, params={"count": 10}),
        optional(
            "GET",
            f"/lol-match-history/v1/products/lol/{puuid}/matches",
            params={"begIndex": beg_index, "endIndex": beg_index + match_limit - 1},
        ),
        _champion_names(),
    )
    if prefer_sgp:
        ranked, mastery, history = None, None, None
    match_source = "lcu"
    ranked_source = "lcu" if ranked else "none"
    if not ranked:
        try:
            ranked = await (_sgp_ranked_stats(puuid, sgp_server_id) if sgp_server_id else _sgp_ranked_stats(puuid))
            ranked_source = "sgp"
        except RuntimeError:
            pass
    if not prefer_sgp and isinstance(history, dict):
        history = await _complete_lcu_match_history(history)
    matches = _normalize_match_rows(history or {}, names, puuid)
    if len(matches) < match_limit:
        try:
            sgp_history = await (
                _sgp_match_history(puuid, beg_index, match_limit, sgp_server_id)
                if sgp_server_id
                else _sgp_match_history(puuid, beg_index, match_limit)
            )
            sgp_matches = _normalize_sgp_match_rows(sgp_history, names, puuid)
            if sgp_matches:
                matches = sgp_matches
                match_source = "sgp"
        except RuntimeError:
            pass
    challenges = {}
    try:
        challenges = await (
            _sgp_player_challenges(puuid, sgp_server_id)
            if sgp_server_id
            else _sgp_player_challenges(puuid)
        )
    except RuntimeError:
        pass
    current_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    if puuid == current_puuid:
        _index_match_encounters(matches, current_puuid)
    tags = _find_player_tag(_read_player_tags(), puuid)
    if match_limit >= 100 and beg_index == 0 and matches:
        await _store_match_collection(puuid, matches)
    collection_count = await _match_collection_count(puuid)
    return {
        "summoner": {
            "puuid": puuid,
            "game_name": summoner.get("gameName") or summoner.get("displayName"),
            "tag_line": summoner.get("tagLine") or "",
            "summoner_level": summoner.get("summonerLevel"),
            "profile_icon_id": summoner.get("profileIconId"),
            "source": summoner.get("source") or "lcu",
        },
        "ranked": ranked or {},
        "ranked_source": ranked_source,
        "mastery": mastery or {},
        "player_challenges": challenges,
        "matches": matches,
        "match_source": match_source,
        "collection_count": collection_count,
        "server_id": sgp_server_id or _sgp_server_id(league_lab_service.credentials),
        "page": {
            "beg_index": beg_index,
            "end_index": beg_index + match_limit - 1,
            "has_more": len(matches) >= match_limit,
        },
        "tag": tags,
    }


def _league_collection_db_path() -> Path:
    return get_data_dir() / "cs2-insight.db"


async def _ensure_league_collection_table(conn: aiosqlite.Connection) -> None:
    await conn.execute(
        """
        CREATE TABLE IF NOT EXISTS league_match_collection (
            puuid TEXT NOT NULL,
            game_id TEXT NOT NULL,
            played_at INTEGER,
            payload_json TEXT NOT NULL,
            collected_at INTEGER NOT NULL,
            PRIMARY KEY (puuid, game_id)
        )
        """
    )


async def _store_match_collection(puuid: str, matches: list[dict]) -> int:
    path = _league_collection_db_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    async with aiosqlite.connect(path) as conn:
        await _ensure_league_collection_table(conn)
        now = int(time.time())
        for row in matches:
            game_id = str(row.get("game_id") or "")
            if not game_id:
                continue
            await conn.execute(
                """
                INSERT INTO league_match_collection (puuid, game_id, played_at, payload_json, collected_at)
                VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(puuid, game_id) DO UPDATE SET
                    played_at=excluded.played_at,
                    payload_json=excluded.payload_json,
                    collected_at=excluded.collected_at
                """,
                (puuid, game_id, row.get("played_at"), json.dumps(row, ensure_ascii=False), now),
            )
        await conn.commit()
        cursor = await conn.execute("SELECT COUNT(*) FROM league_match_collection WHERE puuid = ?", (puuid,))
        count = int((await cursor.fetchone())[0])
    return count


async def _match_collection_count(puuid: str) -> int:
    path = _league_collection_db_path()
    if not path.exists():
        return 0
    async with aiosqlite.connect(path) as conn:
        await _ensure_league_collection_table(conn)
        cursor = await conn.execute("SELECT COUNT(*) FROM league_match_collection WHERE puuid = ?", (puuid,))
        row = await cursor.fetchone()
    return int(row[0] if row else 0)


async def _read_match_collection(puuid: str, limit: int = 100) -> list[dict]:
    path = _league_collection_db_path()
    if not path.exists():
        return []
    async with aiosqlite.connect(path) as conn:
        await _ensure_league_collection_table(conn)
        cursor = await conn.execute(
            "SELECT payload_json FROM league_match_collection WHERE puuid = ? ORDER BY COALESCE(played_at, 0) DESC, collected_at DESC LIMIT ?",
            (puuid, max(1, min(limit, 500))),
        )
        rows = await cursor.fetchall()
    result = []
    for row in rows:
        try:
            payload = json.loads(row[0])
        except (TypeError, ValueError):
            continue
        if isinstance(payload, dict):
            result.append(payload)
    return result


@router.get("/players/{puuid}/collection")
async def league_player_collection(puuid: str, limit: int = 100):
    matches = await _read_match_collection(puuid, limit)
    return {"puuid": puuid, "matches": matches, "count": len(matches), "source": "sqlite"}


@router.get("/players/{puuid}/mastery")
async def league_player_mastery(puuid: str):
    if not puuid.strip():
        raise HTTPException(status_code=422, detail="缺少玩家 PUUID")
    try:
        rows, names = await asyncio.gather(
            league_lab_service.request("GET", f"/lol-champion-mastery/v1/{quote(puuid, safe='')}/champion-mastery"),
            _champion_names(),
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    normalized = []
    for row in rows if isinstance(rows, list) else []:
        if not isinstance(row, dict):
            continue
        champion_id = int(row.get("championId") or 0)
        normalized.append({
            **row,
            "championId": champion_id,
            "championName": names.get(champion_id, str(champion_id)),
        })
    normalized.sort(key=lambda row: int(row.get("championPoints") or 0), reverse=True)
    return {"mastery": normalized, "count": len(normalized)}


@router.get("/players/{puuid}/jungle-analysis")
async def league_player_jungle_analysis(puuid: str, limit: int = 6, server_id: str = ""):
    try:
        target_server_id = _normalize_sgp_server_id(server_id) if server_id.strip() else _sgp_server_id(league_lab_service.credentials)
    except RuntimeError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc
    prefer_sgp = bool(server_id.strip() and target_server_id != _sgp_server_id(league_lab_service.credentials))
    history = None
    source = "sgp" if prefer_sgp else "lcu"
    if not prefer_sgp:
        try:
            history = await league_lab_service.request(
                "GET",
                f"/lol-match-history/v1/products/lol/{puuid}/matches",
                params={"begIndex": 0, "endIndex": 29},
            )
        except RuntimeError:
            pass
    if not isinstance(history, dict):
        try:
            history = await _sgp_match_history(puuid, 0, 30, target_server_id or None)
            source = "sgp"
            prefer_sgp = True
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
    result = await _load_jungle_analysis(
        puuid,
        history,
        limit=max(1, min(limit, 10)),
        server_id=target_server_id or None,
        prefer_sgp=prefer_sgp,
    )
    return {**result, "puuid": puuid, "server_id": target_server_id, "history_source": source}


class PlayerTagBody(BaseModel):
    label: str = Field(default="", max_length=40)
    note: str = Field(default="", max_length=500)
    color: str = Field(default="emerald", max_length=24)


class PlayerTagImportRow(PlayerTagBody):
    puuid: str = Field(min_length=1, max_length=200)
    owner_puuid: str = Field(default="", max_length=200)


class PlayerTagsImportBody(BaseModel):
    rows: list[PlayerTagImportRow] = Field(default_factory=list, max_length=1000)


@router.get("/player-tags")
async def list_league_player_tags(page: int = 1, page_size: int = 20, query: str = "", current_account_only: bool = True):
    current_owner = str(league_lab_service.current_summoner.get("puuid") or "")
    needle = str(query or "").strip().casefold()
    known_players = {
        str(row.get("puuid")): row
        for row in _read_recent_players()
        if isinstance(row, dict) and row.get("puuid")
    }
    rows = []
    for key, record in _read_player_tags().items():
        if not isinstance(record, dict):
            continue
        owner_puuid, puuid = _split_player_tag_key(key, record)
        if not puuid or (current_account_only and owner_puuid and owner_puuid != current_owner):
            continue
        tag = _public_player_tag(record)
        player = known_players.get(puuid) or {}
        searchable = " ".join((
            puuid,
            owner_puuid,
            tag["label"],
            tag["note"],
            str(player.get("game_name") or ""),
            str(player.get("tag_line") or ""),
        )).casefold()
        if needle and needle not in searchable:
            continue
        rows.append({
            "key": key,
            "owner_puuid": owner_puuid,
            "puuid": puuid,
            "tag": tag,
            "player": {
                "game_name": str(player.get("game_name") or ""),
                "tag_line": str(player.get("tag_line") or ""),
                "profile_icon_id": player.get("profile_icon_id"),
            },
            "updated_at": float(record.get("_updated_at") or 0),
        })
    rows.sort(key=lambda row: (row["updated_at"], row["tag"]["label"], row["puuid"]), reverse=True)
    safe_page_size = max(1, min(page_size, 100))
    safe_page = max(1, page)
    start = (safe_page - 1) * safe_page_size
    return {
        "rows": rows[start:start + safe_page_size],
        "total": len(rows),
        "page": safe_page,
        "page_size": safe_page_size,
        "current_owner_puuid": current_owner,
    }


@router.post("/player-tags/import")
async def import_league_player_tags(body: PlayerTagsImportBody):
    current_owner = str(league_lab_service.current_summoner.get("puuid") or "")
    tags = _read_player_tags()
    imported = 0
    for row in body.rows:
        owner_puuid = row.owner_puuid or current_owner
        key = _player_tag_key(owner_puuid, row.puuid)
        tags[key] = {
            **row.model_dump(exclude={"puuid", "owner_puuid"}),
            "_owner_puuid": owner_puuid,
            "_updated_at": time.time(),
        }
        imported += 1
    if imported:
        _write_player_tags(tags)
    return {"imported": imported, "total": len(tags)}


@router.delete("/player-tags/{tag_key}")
async def delete_league_player_tag(tag_key: str):
    tags = _read_player_tags()
    if tag_key not in tags:
        raise HTTPException(status_code=404, detail="玩家标签不存在")
    tags.pop(tag_key, None)
    _write_player_tags(tags)
    return {"deleted": True, "key": tag_key}


@router.put("/player-tags/{tag_key}")
async def update_managed_league_player_tag(tag_key: str, body: PlayerTagBody):
    tags = _read_player_tags()
    existing = tags.get(tag_key)
    if not isinstance(existing, dict):
        raise HTTPException(status_code=404, detail="玩家标签不存在")
    owner_puuid, puuid = _split_player_tag_key(tag_key, existing)
    if body.label or body.note:
        tags[tag_key] = {
            **body.model_dump(),
            "_owner_puuid": owner_puuid,
            "_updated_at": time.time(),
        }
        result = _public_player_tag(tags[tag_key])
    else:
        tags.pop(tag_key, None)
        result = None
    _write_player_tags(tags)
    return {"key": tag_key, "owner_puuid": owner_puuid, "puuid": puuid, "tag": result}


@router.put("/players/{puuid}/tag")
async def save_league_player_tag(puuid: str, body: PlayerTagBody):
    tags = _read_player_tags()
    owner_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    key = _player_tag_key(owner_puuid, puuid)
    if body.label or body.note:
        tags[key] = {
            **body.model_dump(),
            "_owner_puuid": owner_puuid,
            "_updated_at": time.time(),
        }
    else:
        tags.pop(key, None)
        if not owner_puuid:
            tags.pop(puuid, None)
    _write_player_tags(tags)
    return {"puuid": puuid, "tag": _public_player_tag(tags.get(key)) if key in tags else None}


_ongoing_cache: dict = {"key": "", "expires_at": 0.0, "payload": None}
_ongoing_inflight: dict[str, asyncio.Task] = {}
_ongoing_deferred: dict[str, asyncio.Task] = {}
_ongoing_background_full: dict[str, asyncio.Task] = {}


def _consume_ongoing_task(task: asyncio.Task, label: str) -> None:
    """Consume detached task failures so asyncio never reports them unhandled."""
    try:
        task.result()
    except asyncio.CancelledError:
        return
    except Exception:
        logger.exception("%s failed", label)


def _ongoing_inflight_key(settings: LeagueLabSettings) -> str:
    """Coalesce overlapping refreshes without coupling callers to a task."""
    return json.dumps(
        [
            str(league_lab_service.phase or ""),
            settings.ongoing_match_history_load_count,
            settings.ongoing_query_concurrency,
            settings.ongoing_match_history_tag_preference,
            settings.ongoing_query_in_lobby_phase,
            settings.ongoing_premade_threshold,
            settings.ongoing_jungle_analysis_count,
            settings.ongoing_show_jungle_pathing,
            settings.ongoing_show_jungle_pathing_for_all_players,
            settings.ongoing_show_premade_tag,
        ],
        separators=(",", ":"),
    )


async def _league_ongoing_game_impl(*, snapshot: bool = False, force_refresh: bool = False):
    settings = league_lab_service.settings
    try:
        gameflow = await league_lab_service.request("GET", "/lol-gameflow/v1/session")
        # A snapshot is deliberately limited to gameflow/lobby/selection
        # reads.  It must never wait on the champion catalog or per-player
        # enrichment; those are part of the background full refresh.
        names = {} if snapshot else await _champion_names()
    except RuntimeError as exc:
        idle_phases = {"", "None", "Matchmaking", "PreEndOfGame", "WaitingForStats", "EndOfGame"}
        live_phase = str(league_lab_service.phase or "")
        if live_phase in idle_phases:
            return {
                "phase": live_phase or "None",
                "query_stage": "idle",
                "queue": {},
                "game_id": None,
                "players": [],
                "available": False,
                "show_match_history_item_border": settings.ongoing_show_match_history_item_border,
                "order_player_by": settings.ongoing_order_player_by,
            }
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    game_data = (gameflow or {}).get("gameData") or {}
    live_phase = str((gameflow or {}).get("phase") or league_lab_service.phase or "")
    lobby = None
    if live_phase == "Lobby" and settings.ongoing_query_in_lobby_phase:
        try:
            lobby = await league_lab_service.request("GET", "/lol-lobby/v2/lobby")
        except RuntimeError:
            lobby = None
    team_metadata = {}
    for team_id, key in ((100, "teamOne"), (200, "teamTwo")):
        for member in game_data.get(key) or []:
            if not isinstance(member, dict):
                continue
            puuid = str(member.get("puuid") or member.get("playerPuuid") or "")
            if puuid:
                team_metadata[puuid] = {**member, "team": member.get("team") or member.get("teamId") or team_id}
    if isinstance(lobby, dict):
        for member in lobby.get("members") or []:
            if not isinstance(member, dict):
                continue
            puuid = str(member.get("puuid") or "")
            if puuid:
                team_metadata[puuid] = {**member, "team": "LOBBY", "selectedPosition": member.get("selectedPosition") or ""}
    selections, seen = [], set()
    for selection in game_data.get("playerChampionSelections") or []:
        if not isinstance(selection, dict):
            continue
        puuid = str(selection.get("puuid") or selection.get("playerPuuid") or "")
        metadata = team_metadata.get(puuid) or {}
        merged = {**metadata, **selection}
        merged["team"] = selection.get("team") or selection.get("teamId") or metadata.get("team")
        merged["selectedPosition"] = selection.get("selectedPosition") or metadata.get("selectedPosition") or ""
        selections.append(merged)
        seen.add(puuid)
    selections.extend(row for puuid, row in team_metadata.items() if puuid not in seen)
    cache_key = json.dumps(
        [
            live_phase,
            "lobby" if isinstance(lobby, dict) else ("champ-select" if live_phase == "ChampSelect" else "in-game"),
            game_data.get("gameId") or (lobby or {}).get("partyId"),
            [
                settings.ongoing_match_history_load_count,
                settings.ongoing_query_concurrency,
                settings.ongoing_match_history_tag_preference,
                settings.ongoing_order_player_by,
                settings.ongoing_query_in_lobby_phase,
                settings.ongoing_premade_threshold,
                settings.ongoing_jungle_analysis_count,
                settings.ongoing_show_champion_usage,
                settings.ongoing_champion_usage_mode,
                settings.ongoing_show_match_history_item_border,
                settings.ongoing_show_jungle_pathing,
                settings.ongoing_show_jungle_pathing_for_all_players,
                settings.ongoing_show_premade_tag,
                settings.ongoing_show_local_tag,
                settings.ongoing_show_streak_tags,
                settings.ongoing_show_performance_tags,
                settings.ongoing_player_card_tag_settings,
            ],
            [
                [row.get("puuid") or row.get("playerPuuid"), row.get("championId"), row.get("team") or row.get("teamId"), row.get("selectedPosition")]
                for row in selections if isinstance(row, dict)
            ],
        ],
        ensure_ascii=False,
    )
    start_generation = int(_ongoing_cache.get("generation") or 0)
    if (
        not force_refresh
        and _ongoing_cache["key"] == cache_key
        and time.monotonic() < _ongoing_cache["expires_at"]
        and (snapshot or _ongoing_cache.get("kind") == "full")
    ):
        return _ongoing_cache["payload"]

    if snapshot:
        cached_players = {
            str(row.get("puuid") or ""): row
            for row in ((_ongoing_cache.get("payload") or {}).get("players") or [])
            if isinstance(row, dict) and row.get("puuid")
        } if _ongoing_cache.get("key") == cache_key else {}
        snapshot_players = []
        queue_data = game_data.get("queue") or ((lobby or {}).get("gameConfig") or {})
        usage_mode = settings.ongoing_champion_usage_mode if settings.ongoing_show_champion_usage else "none"
        for row in selections:
            if not isinstance(row, dict):
                continue
            puuid = str(row.get("puuid") or row.get("playerPuuid") or "")
            if not puuid:
                continue
            cached = cached_players.get(puuid) or {}
            player = dict(cached) if cached else {
                "puuid": puuid,
                "team": row.get("team") or row.get("teamId"),
                "champion_id": int(row.get("championId") or 0),
                "champion_name": "",
                "position": str(row.get("selectedPosition") or row.get("assignedPosition") or "").upper(),
                "summoner": {key: row[key] for key in ("gameName", "displayName", "tagLine", "profileIconId") if row.get(key) is not None},
                "ranked": {},
                "tag": {},
                "recent": {"matches": 0, "wins": 0, "average_kda": 0, "akari_score": 0, "details_analyzed": 0},
                "recent_matches": [],
                "rating_summary": _ongoing_rating_summary([]),
                "champion_usage": {"mode": usage_mode, "matches": 0, "wins": 0, "average_kda": 0, "mastery_level": 0, "mastery_points": 0},
                "jungle_analysis": None,
                "performance_tags": [],
                "data_availability": {"summoner": False, "ranked": False, "history": False, "mastery": usage_mode != "mastery", "unavailable": [], "history_source": None, "deferred": ["summoner", "ranked", "history"] + (["mastery"] if usage_mode == "mastery" else [])},
                "load_state": "loading",
            }
            # Refresh the live identity fields even when an older full card is
            # available; this prevents a stale phase/selection from leaking.
            player["team"] = row.get("team") or row.get("teamId") or player.get("team")
            player["champion_id"] = int(row.get("championId") or player.get("champion_id") or 0)
            player["position"] = str(row.get("selectedPosition") or row.get("assignedPosition") or player.get("position") or "").upper()
            snapshot_players.append(player)
        team_tags = _ongoing_win_rate_team_tags(snapshot_players)
        analysis = _ongoing_analysis_payload(snapshot_players, team_tags)
        result = {
            "phase": live_phase,
            "query_stage": "lobby" if isinstance(lobby, dict) else ("champ-select" if live_phase == "ChampSelect" else "in-game"),
            "queue": queue_data,
            "game_id": game_data.get("gameId") or (lobby or {}).get("partyId"),
            "players": snapshot_players,
            "teams": {team: list(info.get("players") or []) for team, info in analysis.get("teams", {}).items()},
            "analysis": analysis,
            "team_tags": team_tags,
            "available": bool(snapshot_players),
            "partial": True,
            "snapshot": True,
            "show_match_history_item_border": settings.ongoing_show_match_history_item_border,
            "order_player_by": settings.ongoing_order_player_by,
        }
        _ongoing_cache.update({"key": cache_key, "expires_at": time.monotonic() + 30.0, "payload": result, "kind": "snapshot", "generation": start_generation + 1})
        if cache_key not in _ongoing_background_full or _ongoing_background_full[cache_key].done():
            task = asyncio.create_task(_league_ongoing_game_impl(force_refresh=True))
            _ongoing_background_full[cache_key] = task
            def _clear_background(done_task):
                if _ongoing_background_full.get(cache_key) is done_task:
                    _ongoing_background_full.pop(cache_key, None)
                _consume_ongoing_task(done_task, "ongoing full enrichment")
            task.add_done_callback(_clear_background)
        return result
    query_semaphore = asyncio.Semaphore(settings.ongoing_query_concurrency)
    # Timeline calls are much heavier than identity/history reads.  Keep them
    # on a separate, bounded lane so an all-player scan cannot flood the LCU.
    timeline_semaphore = asyncio.Semaphore(max(1, min(settings.ongoing_query_concurrency, 4)))
    sgp_server_id = _sgp_server_id(league_lab_service.credentials)
    # Player tags are stored in one local JSON document. Snapshot it once for
    # the whole 10-player analysis instead of reopening and parsing the same
    # file independently in every enrichment coroutine.
    player_tags = _read_player_tags() if settings.ongoing_show_local_tag else {}

    async def enrich(row):
        if not isinstance(row, dict):
            return None, None
        puuid = str(row.get("puuid") or row.get("playerPuuid") or "")
        summoner, ranked, history, mastery = None, None, None, None
        history_source = "lcu"
        load_errors: dict[str, str] = {}
        if puuid:
            async def load_optional(name: str, path: str, *, params=None):
                try:
                    async with query_semaphore:
                        return await league_lab_service.request("GET", path, params=params)
                except RuntimeError as exc:
                    # Tencent and privacy-restricted accounts frequently expose
                    # only a subset of these optional endpoints.  Keep every
                    # successful field instead of letting one failure discard
                    # the entire player card.
                    load_errors[name] = str(exc)
                    return None

            summoner, ranked, history = await asyncio.gather(
                load_optional("summoner", f"/lol-summoner/v2/summoners/puuid/{puuid}"),
                load_optional("ranked", f"/lol-ranked/v1/ranked-stats/{puuid}"),
                load_optional(
                    "history",
                    f"/lol-match-history/v1/products/lol/{puuid}/matches",
                    params={"begIndex": 0, "endIndex": settings.ongoing_match_history_load_count - 1},
                ),
            )
            if not isinstance(history, dict):
                try:
                    async with query_semaphore:
                        history = await _sgp_match_history(
                            puuid,
                            0,
                            settings.ongoing_match_history_load_count,
                            sgp_server_id or None,
                        )
                    history_source = "sgp"
                    load_errors.pop("history", None)
                except RuntimeError as exc:
                    load_errors["history_sgp"] = str(exc)
            # Mastery and timeline data are deferred below.  They are useful
            # card enrichments, but must not delay the first ten-player roster
            # response when the LCU is under load.
        champion_id = int(row.get("championId") or 0)
        matches = _normalize_match_rows(history or {}, names, puuid)
        queue_data = game_data.get("queue") or ((lobby or {}).get("gameConfig") or {})
        queue_id = int(queue_data.get("id") or queue_data.get("queueId") or 0)
        if settings.ongoing_match_history_tag_preference == "current" and queue_id:
            matches = [match for match in matches if int(match.get("queue_id") or 0) == queue_id]
        detail_matches = matches[:settings.ongoing_game_details_load_count]
        usage_mode = settings.ongoing_champion_usage_mode if settings.ongoing_show_champion_usage else "none"
        selected_position = str(row.get("selectedPosition") or row.get("assignedPosition") or "").upper()
        jungle_requested = bool(
            settings.ongoing_show_jungle_pathing
            and (selected_position == "JUNGLE" or settings.ongoing_show_jungle_pathing_for_all_players)
            and isinstance(history, dict)
        )
        deferred_fields = (["mastery"] if usage_mode == "mastery" else []) + (["jungle"] if jungle_requested else [])
        champion_matches = [match for match in matches if match.get("champion_id") == champion_id] if usage_mode == "recent" else []
        champion_mastery = next(
            (item for item in mastery or [] if int(item.get("championId") or 0) == champion_id),
            {},
        ) if isinstance(mastery, list) and champion_id else {}
        jungle_analysis = None
        # Jungle timelines are scheduled after the response is cached.  The
        # returned player remains explicit about deferred data availability.
        recent_kda = round(sum(
            (float(match.get("kills") or 0) + float(match.get("assists") or 0))
            / max(1.0, float(match.get("deaths") or 0))
            for match in matches
        ) / max(1, len(matches)), 2)
        local_tag = _find_player_tag(player_tags, puuid) if settings.ongoing_show_local_tag else {}
        card_tags = _ongoing_performance_tags(
            detail_matches,
            show_streaks=settings.ongoing_show_streak_tags,
            show_performance=settings.ongoing_show_performance_tags,
            tag_settings=settings.ongoing_player_card_tag_settings,
        )
        own_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
        extra_tags = []
        tag_flags = settings.ongoing_player_card_tag_settings
        if puuid and puuid == own_puuid and tag_flags.get("self", True):
            extra_tags.append({"id": "self", "label": "自己", "tone": "info", "title": "当前登录的召唤师。"})
        if local_tag and tag_flags.get("tagged", True):
            extra_tags.append({"id": "tagged", "label": local_tag.get("label") or "已标记", "tone": "warning", "title": local_tag.get("note") or "这是保存在本机的玩家标签。"})
        if puuid != own_puuid and own_puuid and tag_flags.get("met", True) and any(
            any(str(participant.get("puuid") or "") == own_puuid for participant in (match.get("participants") or []))
            for match in detail_matches
        ):
            extra_tags.append({"id": "met", "label": "遇到过", "tone": "info", "title": "当前账号的近期对局样本中出现过这名玩家。"})
        if str((summoner or {}).get("privacy") or "").upper() == "PRIVATE" and tag_flags.get("privacy", True):
            extra_tags.append({"id": "privacy", "label": "战绩私密", "tone": "negative", "title": "客户端将该玩家的战绩隐私状态标记为 PRIVATE。"})
        data_availability = {
            "summoner": summoner is not None,
            "ranked": ranked is not None,
            "history": history is not None,
            "mastery": usage_mode != "mastery" or mastery is not None,
            "unavailable": sorted(load_errors),
            "history_source": history_source if isinstance(history, dict) else None,
        }
        if deferred_fields:
            data_availability["deferred"] = deferred_fields
        known_results = [
            match.get("win")
            for match in matches
            if isinstance(match.get("win"), bool)
        ]
        champion_known_results = [
            match.get("win")
            for match in champion_matches
            if isinstance(match.get("win"), bool)
        ]
        return ({
            "puuid": puuid,
            "team": row.get("team") or row.get("teamId"),
            "champion_id": champion_id,
            "champion_name": names.get(champion_id, str(champion_id)) if champion_id else "",
            "position": selected_position,
            "summoner": summoner or {},
            "ranked": ranked or {},
            "tag": local_tag,
            "recent": {
                "matches": len(matches),
                "known_results": len(known_results),
                "unknown_results": len(matches) - len(known_results),
                "wins": sum(1 for result in known_results if result),
                "average_kda": recent_kda,
                "akari_score": _ongoing_akari_score(detail_matches),
                "details_analyzed": len(detail_matches),
            },
            # Keep the normalized match rows alongside the aggregate metrics.
            # The ongoing player card can then render LeagueAkari-style recent
            # results without making a second request for every player.
            "recent_matches": detail_matches,
            "rating_summary": _ongoing_rating_summary(detail_matches),
            "champion_usage": {
                "mode": usage_mode,
                "matches": len(champion_matches),
                "known_results": len(champion_known_results),
                "unknown_results": len(champion_matches) - len(champion_known_results),
                "wins": sum(1 for result in champion_known_results if result),
                "average_kda": round(sum((match.get("kills", 0) + match.get("assists", 0)) / max(1, match.get("deaths", 0)) for match in champion_matches) / max(1, len(champion_matches)), 2),
                "mastery_level": champion_mastery.get("championLevel", 0),
                "mastery_points": champion_mastery.get("championPoints", 0),
            },
            "jungle_analysis": jungle_analysis,
            "performance_tags": extra_tags + card_tags,
            "data_availability": data_availability,
            "load_state": "partial" if deferred_fields else "ready",
            }, history or {})
    enriched = await asyncio.gather(*(enrich(row) for row in selections))
    players = [result[0] for result in enriched if result[0]]
    histories = {result[0]["puuid"]: result[1] for result in enriched if result[0] and result[0].get("puuid")}
    premade_groups = _infer_premade_groups(
        histories,
        set(histories),
        threshold=settings.ongoing_premade_threshold,
    ) if settings.ongoing_show_premade_tag else {}
    for player in players:
        player["premade_group"] = premade_groups.get(player.get("puuid"))
    position_order = {"TOP": 0, "JUNGLE": 1, "MIDDLE": 2, "MID": 2, "BOTTOM": 3, "UTILITY": 4}
    sorters = {
        "win-rate": lambda player: -float(player["recent"]["wins"]) / max(1, int(player["recent"].get("known_results") or 0)),
        "kda": lambda player: -float(player["recent"]["average_kda"]),
        "akari-score": lambda player: -float(player["recent"]["akari_score"]),
        "position": lambda player: position_order.get(str(player.get("position") or "").upper(), 99),
        "premade-team": lambda player: int(player.get("premade_group") or 999),
    }
    if settings.ongoing_order_player_by in sorters:
        players.sort(key=sorters[settings.ongoing_order_player_by])
    # Keep the raw player rows for compatibility, while also exposing the
    # upstream LeagueAkari-shaped aggregate contract for the rich ongoing
    # cards (team badges, KDA outliers and tag popovers).
    team_tags = _ongoing_win_rate_team_tags(players)
    analysis = _ongoing_analysis_payload(players, team_tags)
    queue = game_data.get("queue") or ((lobby or {}).get("gameConfig") or {})
    result = {
        "phase": live_phase,
        "query_stage": "lobby" if isinstance(lobby, dict) else ("champ-select" if live_phase == "ChampSelect" else "in-game"),
        "queue": queue,
        "game_id": game_data.get("gameId") or (lobby or {}).get("partyId"),
        "players": players,
        "teams": {
            team: list(info.get("players") or [])
            for team, info in analysis.get("teams", {}).items()
        },
        "analysis": analysis,
        "team_tags": team_tags,
        "available": bool(players),
        "partial": any(player.get("load_state") == "partial" for player in players),
        "show_match_history_item_border": settings.ongoing_show_match_history_item_border,
        "order_player_by": settings.ongoing_order_player_by,
    }
    if players and not isinstance(lobby, dict):
        _remember_recent_players(players, game_data.get("gameId"))
    has_partial_data = any(player.get("data_availability", {}).get("unavailable") for player in players)
    # A newer snapshot for another game/phase may have replaced the cache
    # while this full task was waiting on slow history.  Never publish that
    # stale result over the newer generation.
    if (
        _ongoing_cache.get("key") != cache_key
        and int(_ongoing_cache.get("generation") or 0) > start_generation
    ):
        return _ongoing_cache.get("payload")
    _ongoing_cache.update({
        "key": cache_key,
        # A partial payload is still a valid snapshot.  Keeping it briefly in
        # cache lets the deferred task fill it in instead of causing another
        # ten-player enrichment on every frontend poll.
        "expires_at": time.monotonic() + 30.0,
        "payload": result,
        "kind": "full",
    })

    async def deferred_enrichment():
        """Fill optional card fields after the fast roster is available."""
        try:
            for player in players:
                if _ongoing_cache.get("key") != cache_key:
                    return
                puuid = str(player.get("puuid") or "")
                if not puuid:
                    continue
                availability = player.setdefault("data_availability", {})
                deferred = set(availability.get("deferred") or [])
                if "mastery" in deferred:
                    try:
                        async with query_semaphore:
                            mastery = await league_lab_service.request(
                                "GET", f"/lol-champion-mastery/v1/{puuid}/champion-mastery"
                            )
                        champion_id = int(player.get("champion_id") or 0)
                        champion_mastery = next(
                            (item for item in mastery or [] if int(item.get("championId") or 0) == champion_id),
                            {},
                        ) if isinstance(mastery, list) and champion_id else {}
                        usage = player.setdefault("champion_usage", {})
                        usage["mastery_level"] = champion_mastery.get("championLevel", 0)
                        usage["mastery_points"] = champion_mastery.get("championPoints", 0)
                        availability["mastery"] = True
                    except RuntimeError as exc:
                        availability.setdefault("unavailable", []).append("mastery")
                        logger.debug("ongoing mastery deferred load failed for %s: %s", puuid, exc)
                    deferred.discard("mastery")
                if "jungle" in deferred:
                    history = histories.get(puuid) or {}
                    try:
                        jungle = await _load_jungle_analysis(
                            puuid,
                            history,
                            limit=settings.ongoing_jungle_analysis_count,
                            server_id=sgp_server_id or None,
                            semaphore=timeline_semaphore,
                        )
                        if isinstance(jungle, dict):
                            jungle["main_champions"] = _ongoing_jungle_main_champions(
                                player.get("recent_matches") or [], names
                            )
                            player["jungle_analysis"] = jungle
                        availability["jungle"] = True
                    except RuntimeError as exc:
                        availability.setdefault("unavailable", []).append("jungle")
                        logger.debug("ongoing jungle deferred load failed for %s: %s", puuid, exc)
                    deferred.discard("jungle")
                availability["deferred"] = sorted(deferred)
                player["load_state"] = "partial" if deferred or availability.get("unavailable") else "ready"
            result["team_tags"] = _ongoing_win_rate_team_tags(players)
            result["analysis"] = _ongoing_analysis_payload(players, result["team_tags"])
            result["partial"] = any(player.get("load_state") == "partial" for player in players)
            _ongoing_cache["payload"] = result
        finally:
            _ongoing_deferred.pop(cache_key, None)

    if any(player.get("data_availability", {}).get("deferred") for player in players):
        task = asyncio.create_task(deferred_enrichment())
        _ongoing_deferred[cache_key] = task
        task.add_done_callback(lambda done_task: _consume_ongoing_task(done_task, "ongoing deferred enrichment"))
    return result


@router.get("/ongoing-game")
async def league_ongoing_game(snapshot: bool = False):
    settings = league_lab_service.settings
    key = f"{_ongoing_inflight_key(settings)}:{'snapshot' if snapshot else 'full'}"
    existing = _ongoing_inflight.get(key)
    if existing and not existing.done():
        return await asyncio.shield(existing)
    task = asyncio.create_task(_league_ongoing_game_impl(snapshot=snapshot))
    _ongoing_inflight[key] = task
    try:
        return await asyncio.shield(task)
    finally:
        if _ongoing_inflight.get(key) is task:
            _ongoing_inflight.pop(key, None)


@router.get("/cooldown-timer/state")
async def league_cooldown_timer_state():
    settings = league_lab_service.settings
    if not settings.cooldown_timer_enabled:
        return {"enabled": False, "available": False, "players": [], "game_time": None}
    try:
        gameflow, spells, names = await asyncio.gather(
            league_lab_service.request("GET", "/lol-gameflow/v1/session"),
            league_lab_service.request("GET", "/lol-game-data/assets/v1/summoner-spells.json"),
            _champion_names(),
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    game_data = (gameflow or {}).get("gameData") or {}
    queue = game_data.get("queue") or {}
    mode = str(queue.get("gameMode") or "").upper()
    ability_haste = {
        "CLASSIC": 0,
        "PRACTICETOOL": 0,
        "ARAM": 70,
        "URF": 300,
        "ONEFORALL": 0,
        "NEXUSBLITZ": 0,
        "ULTBOOK": 0,
        "KIWI": 70,
    }.get(mode)
    if str((gameflow or {}).get("phase") or league_lab_service.phase) != "InProgress" or ability_haste is None:
        return {"enabled": True, "available": False, "players": [], "game_time": None, "game_mode": mode}
    own_puuid = str(league_lab_service.current_summoner.get("puuid") or "")
    team_one = [row for row in game_data.get("teamOne") or [] if isinstance(row, dict)]
    team_two = [row for row in game_data.get("teamTwo") or [] if isinstance(row, dict)]
    own_team = team_one if any(str(row.get("puuid") or "") == own_puuid for row in team_one) else team_two
    enemies = team_two if own_team is team_one else team_one
    selections = {
        str(row.get("puuid") or row.get("playerPuuid") or ""): row
        for row in game_data.get("playerChampionSelections") or []
        if isinstance(row, dict)
    }
    position_order = {"TOP": 0, "JUNGLE": 1, "MIDDLE": 2, "MID": 2, "BOTTOM": 3, "UTILITY": 4}
    players = []
    for index, member in enumerate(enemies):
        puuid = str(member.get("puuid") or member.get("playerPuuid") or "")
        selection = selections.get(puuid) or {}
        champion_id = int(selection.get("championId") or member.get("championId") or 0)
        players.append({
            "puuid": puuid,
            "champion_id": champion_id,
            "champion_name": names.get(champion_id, str(champion_id)),
            "position": str(member.get("selectedPosition") or selection.get("selectedPosition") or "").upper(),
            "spell1_id": int(selection.get("spell1Id") or member.get("spell1Id") or 0),
            "spell2_id": int(selection.get("spell2Id") or member.get("spell2Id") or 0),
            "source_index": index,
        })
    players.sort(key=lambda row: (position_order.get(row["position"], 99), row["source_index"]))
    spell_rows = spells if isinstance(spells, list) else list(spells.values()) if isinstance(spells, dict) else []
    spell_catalog = {
        int(row["id"]): {
            "id": int(row["id"]),
            "name": str(row.get("name") or row["id"]),
            "cooldown": float(row.get("cooldown") or 0),
        }
        for row in spell_rows
        if isinstance(row, dict) and row.get("id")
    }
    game_time = None
    try:
        async with httpx.AsyncClient(verify=False, timeout=1.5) as client:
            response = await client.get("https://127.0.0.1:2999/liveclientdata/gamestats")
        response.raise_for_status()
        payload = response.json()
        game_time = float(payload.get("gameTime")) if isinstance(payload, dict) and payload.get("gameTime") is not None else None
    except (httpx.HTTPError, TypeError, ValueError):
        pass
    return {
        "enabled": True,
        "available": bool(players),
        "game_mode": mode,
        "ability_haste": ability_haste,
        "timer_type": settings.cooldown_timer_type,
        "reverse_adjustment": settings.cooldown_timer_reverse_adjustment,
        "game_time": game_time,
        "players": players,
        "spells": spell_catalog,
    }


@router.post("/cooldown-timer/send")
async def league_cooldown_timer_send(body: InGameTextSend):
    if not league_lab_service.settings.cooldown_timer_enabled:
        raise HTTPException(status_code=409, detail="请先启用敌方召唤师技能计时器")
    try:
        pid = await asyncio.to_thread(_send_text_to_foreground_league_game, body.text)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"sent": True, "pid": pid}


@router.get("/champions")
async def league_champion_catalog():
    champions = await _champion_catalog()
    return {"champions": champions, "count": len(champions), "source": "lcu" if league_lab_service.credentials else "cache"}


@router.get("/assets/champions/{champion_id}.png")
async def league_champion_icon(champion_id: int):
    if champion_id <= 0:
        raise HTTPException(status_code=404, detail="英雄头像不存在")
    try:
        content, media_type = await league_lab_service.request_bytes(
            f"/lol-game-data/assets/v1/champion-icons/{champion_id}.png"
        )
    except RuntimeError:
        try:
            content, media_type = await _ddragon_champion_icon(champion_id)
        except RuntimeError as exc:
            raise HTTPException(status_code=404, detail=str(exc)) from exc
    return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})


@router.get("/assets/summoner-spells/{spell_id}.png")
async def league_summoner_spell_icon(spell_id: int):
    if spell_id <= 0:
        raise HTTPException(status_code=404, detail="召唤师技能图标不存在")
    # Current LCU builds no longer serve the numeric PNG path directly.  The
    # match payload still gives us the numeric spell id, while the catalog is
    # the source of truth for the corresponding client asset path (for
    # example Flash/Heal are resolved to their real iconPath).  Resolve this
    # before trying the legacy path so the browser does not receive a 400 and
    # render the empty Icon fallback.
    catalog_error = None
    try:
        catalog = await _league_loadout_catalog_payload()
        spell = next((row for row in catalog.get("spells") or [] if int(row.get("id") or 0) == spell_id), None)
        icon_path = str((spell or {}).get("icon_path") or "")
        if icon_path.startswith("/lol-game-data/assets/") and ".." not in icon_path:
            content, media_type = await league_lab_service.request_bytes(icon_path)
            return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})
    except (RuntimeError, HTTPException) as exc:
        catalog_error = exc

    # Keep compatibility with older clients/catalog caches whose spell rows do
    # not contain iconPath yet.  This branch is deliberately after the
    # catalog lookup because the numeric route returns 400 on current clients.
    try:
        content, media_type = await league_lab_service.request_bytes(
            f"/lol-game-data/assets/v1/summoner-spells/{spell_id}.png"
        )
    except RuntimeError as exc:
        detail = str(catalog_error or exc)
        raise HTTPException(status_code=404, detail=detail) from exc
    return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})


@router.get("/assets/items/{item_id}.png")
async def league_item_icon(item_id: int):
    if item_id <= 0:
        raise HTTPException(status_code=404, detail="装备图标不存在")
    try:
        content, media_type = await league_lab_service.request_bytes(f"/lol-game-data/assets/v1/items/{item_id}.png")
    except RuntimeError as exc:
        try:
            content, media_type = await _ddragon_item_icon(item_id)
        except RuntimeError:
            raise HTTPException(status_code=404, detail=str(exc)) from exc
    return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})


@router.get("/assets/profile-icons/{profile_icon_id}.jpg")
async def league_profile_icon(profile_icon_id: int):
    if profile_icon_id < 0:
        raise HTTPException(status_code=404, detail="召唤师头像不存在")
    last_error = None
    for suffix in ("jpg", "png"):
        try:
            content, media_type = await league_lab_service.request_bytes(
                f"/lol-game-data/assets/v1/profile-icons/{profile_icon_id}.{suffix}"
            )
            return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})
        except RuntimeError as exc:
            last_error = exc
    raise HTTPException(status_code=404, detail=str(last_error or "召唤师头像不存在"))


_loadout_catalog_cache: dict = {"expires_at": 0.0, "payload": None}


async def _league_loadout_catalog_payload() -> dict:
    cached = _loadout_catalog_cache.get("payload")
    if isinstance(cached, dict) and time.monotonic() < float(_loadout_catalog_cache.get("expires_at") or 0):
        return cached
    try:
        styles, spells, perk_rows = await asyncio.gather(
            league_lab_service.request("GET", "/lol-game-data/assets/v1/perkstyles.json"),
            league_lab_service.request("GET", "/lol-game-data/assets/v1/summoner-spells.json"),
            league_lab_service.request("GET", "/lol-game-data/assets/v1/perks.json"),
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    perk_catalog = {
        int(perk.get("id")): {
            "id": int(perk.get("id")),
            "name": str(perk.get("name") or perk.get("id")),
            "short_description": str(perk.get("shortDesc") or perk.get("shortDescription") or ""),
            "long_description": str(perk.get("longDesc") or perk.get("longDescription") or ""),
            "icon_path": str(perk.get("iconPath") or ""),
        }
        for perk in (perk_rows or [])
        if isinstance(perk, dict) and perk.get("id")
    }
    normalized_styles = []
    for style in styles if isinstance(styles, list) else []:
        if not isinstance(style, dict) or not style.get("id"):
            continue
        perks = []
        normalized_slots = []
        for slot in style.get("slots") or []:
            if not isinstance(slot, dict):
                continue
            slot_perks = []
            for perk in (slot.get("perks") or []):
                if not isinstance(perk, dict) or not perk.get("id"):
                    continue
                perk_id = int(perk.get("id"))
                normalized_perk = {
                    **perk_catalog.get(perk_id, {}),
                    "id": perk_id,
                    "name": str(perk_catalog.get(perk_id, {}).get("name") or perk.get("name") or perk_id),
                    "icon_path": str(perk_catalog.get(perk_id, {}).get("icon_path") or perk.get("iconPath") or ""),
                }
                perks.append(normalized_perk)
                slot_perks.append(normalized_perk)
            normalized_slots.append({
                "type": str(slot.get("type") or ""),
                "perks": slot_perks,
            })
        normalized_styles.append(
            {
                "id": int(style["id"]),
                "name": str(style.get("name") or style["id"]),
                "icon_path": style.get("iconPath") or "",
                "perks": perks,
                "allowed_sub_styles": [int(value) for value in (style.get("allowedSubStyles") or style.get("allowed_sub_styles") or []) if str(value).isdigit()],
                "slots": normalized_slots,
            }
        )
    spell_rows = spells if isinstance(spells, list) else list(spells.values()) if isinstance(spells, dict) else []
    normalized_spells = [
        {
            "id": int(spell.get("id")),
            "name": str(spell.get("name") or spell.get("id")),
            "description": str(spell.get("description") or ""),
            "icon_path": spell.get("iconPath") or "",
            "game_modes": [str(mode).upper() for mode in (spell.get("gameModes") or spell.get("game_modes") or [])],
        }
        for spell in spell_rows
        if isinstance(spell, dict) and spell.get("id")
    ]
    payload = {"styles": normalized_styles, "spells": normalized_spells, "perks": list(perk_catalog.values())}
    _loadout_catalog_cache.update({"expires_at": time.monotonic() + 300.0, "payload": payload})
    return payload


@router.get("/assets/perks/{perk_id}.png")
async def league_perk_icon(perk_id: int):
    if perk_id <= 0:
        raise HTTPException(status_code=404, detail="符文图标不存在")
    catalog = await _league_loadout_catalog_payload()
    perk = next((row for row in catalog.get("perks") or [] if int(row.get("id") or 0) == perk_id), None)
    icon_path = str((perk or {}).get("icon_path") or "")
    if not icon_path.startswith("/lol-game-data/assets/"):
        raise HTTPException(status_code=404, detail="符文图标不存在")
    try:
        content, media_type = await league_lab_service.request_bytes(icon_path)
    except RuntimeError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})


@router.get("/assets/client")
async def league_client_asset(path: str):
    """Proxy one read-only asset from the authenticated local League client."""
    icon_path = "/" + str(path or "").lstrip("/")
    if not icon_path.startswith("/lol-game-data/assets/") or ".." in icon_path:
        raise HTTPException(status_code=404, detail="客户端资源不存在")
    try:
        content, media_type = await league_lab_service.request_bytes(icon_path)
    except RuntimeError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})


@router.get("/assets/perkstyles/{style_id}.png")
async def league_perkstyle_icon(style_id: int):
    if style_id <= 0:
        raise HTTPException(status_code=404, detail="符文系图标不存在")
    catalog = await _league_loadout_catalog_payload()
    style = next((row for row in catalog.get("styles") or [] if int(row.get("id") or 0) == style_id), None)
    icon_path = str((style or {}).get("icon_path") or "")
    if not icon_path.startswith("/lol-game-data/assets/"):
        raise HTTPException(status_code=404, detail="符文系图标不存在")
    try:
        content, media_type = await league_lab_service.request_bytes(icon_path)
    except RuntimeError as exc:
        raise HTTPException(status_code=404, detail=str(exc)) from exc
    return Response(content=content, media_type=media_type, headers={"Cache-Control": "private, max-age=86400"})


@router.get("/loadout-catalog")
async def league_loadout_catalog():
    return await _league_loadout_catalog_payload()


@router.get("/toolkit/overview")
async def league_toolkit_overview():
    async def optional(path: str, *, params: dict | None = None):
        try:
            return await league_lab_service.request("GET", path, params=params)
        except RuntimeError:
            return None

    missions, mission_series, rewards, loot, friends, friend_groups, events, chat_me = await asyncio.gather(
        optional("/lol-missions/v1/missions"),
        optional("/lol-missions/v1/series"),
        optional("/lol-rewards/v1/grants"),
        optional("/lol-loot/v1/player-loot-map"),
        optional("/lol-chat/v1/friends"),
        optional("/lol-chat/v1/friend-groups"),
        optional("/lol-event-hub/v1/events"),
        optional("/lol-chat/v1/me"),
    )
    mission_rows = missions if isinstance(missions, list) else []
    reward_rows = rewards if isinstance(rewards, list) else []
    loot_rows = list(loot.values()) if isinstance(loot, dict) else loot if isinstance(loot, list) else []
    friend_rows = friends if isinstance(friends, list) else []
    event_rows = [row for row in events if isinstance(row, dict)] if isinstance(events, list) else []
    claimable_events = [
        row
        for row in event_rows
        if int(((row.get("eventInfo") or {}).get("unclaimedRewardCount") or 0)) > 0
    ]

    async def event_reward_options(event: dict) -> dict:
        event_id = str(event.get("eventId") or "")
        if not event_id:
            return {**event, "reward_options": []}
        safe_id = quote(event_id, safe="")
        regular, bonus = await asyncio.gather(
            optional(f"/lol-event-hub/v1/events/{safe_id}/reward-track/items"),
            optional(f"/lol-event-hub/v1/events/{safe_id}/reward-track/bonus-items"),
        )
        options = []
        for item in [*(regular if isinstance(regular, list) else []), *(bonus if isinstance(bonus, list) else [])]:
            for reward in item.get("rewardOptions") or []:
                if isinstance(reward, dict) and reward.get("state") == "Unselected":
                    options.append(reward)
        return {**event, "reward_options": options}

    claimable_events = list(await asyncio.gather(*(event_reward_options(row) for row in claimable_events)))
    claimable_missions = [row for row in mission_rows if row.get("status") == "SELECT_REWARDS"]
    claimable_rewards = [
        row for row in reward_rows if isinstance(row.get("info"), dict) and row["info"].get("status") == "PENDING_SELECTION"
    ]
    unclaimed_rewards = [row for row in reward_rows if not row.get("viewed") or not row.get("selected")]
    return {
        "missions": mission_rows,
        "claimable_missions": claimable_missions,
        "mission_series": mission_series if isinstance(mission_series, list) else [],
        "unclaimed_rewards": unclaimed_rewards,
        "claimable_rewards": claimable_rewards,
        "claimable_events": claimable_events,
        "loot": loot_rows,
        "friends": friend_rows,
        "friend_groups": friend_groups if isinstance(friend_groups, list) else [],
        "chat_presence": chat_me if isinstance(chat_me, dict) else None,
        "counts": {
            "missions": len(mission_rows),
            "unclaimed_rewards": len(unclaimed_rewards),
            "loot": len(loot_rows),
            "friends": len(friend_rows),
        },
        "read_only": not league_lab_service.settings.toolkit_account_actions_enabled,
        "account_actions_enabled": league_lab_service.settings.toolkit_account_actions_enabled,
    }


def _first_match_timestamp(payload: dict | None) -> int | str | None:
    games = (payload or {}).get("games") or []
    if isinstance(games, dict):
        games = games.get("games") or []
    if not isinstance(games, list) or not games:
        return None
    wrapper = games[0]
    game = wrapper.get("json") if isinstance(wrapper, dict) and isinstance(wrapper.get("json"), dict) else wrapper
    if not isinstance(game, dict):
        return None
    return game.get("gameCreation") or game.get("gameCreationDate") or game.get("gameStartTimestamp")


@router.get("/toolkit/friends/metadata")
async def league_friend_metadata():
    """Load LeagueAkari-compatible friend dates without delaying the toolkit overview."""

    async def optional(path: str, *, params: dict | None = None):
        try:
            return await league_lab_service.request("GET", path, params=params)
        except RuntimeError:
            return None

    friends, giftable = await asyncio.gather(
        optional("/lol-chat/v1/friends"),
        optional("/lol-store/v1/giftablefriends"),
    )
    friend_rows = [row for row in (friends or []) if isinstance(row, dict) and row.get("puuid")]
    giftable_rows = [row for row in (giftable or []) if isinstance(row, dict)]
    friend_since_by_summoner = {
        str(row.get("summonerId")): row.get("friendsSince")
        for row in giftable_rows
        if row.get("summonerId") is not None and row.get("friendsSince")
    }

    server_id = _sgp_server_id(league_lab_service.credentials)
    token = None
    if server_id in _SGP_MATCH_HISTORY_HOSTS:
        token_payload = await optional("/entitlements/v1/token")
        if isinstance(token_payload, dict):
            token = token_payload.get("accessToken")

    semaphore = asyncio.Semaphore(6)

    async def enrich(friend: dict) -> tuple[str, dict]:
        puuid = str(friend.get("puuid") or "")
        source = "lcu"
        last_game_at = None
        async with semaphore:
            if token:
                try:
                    history = await _sgp_match_history(
                        puuid,
                        0,
                        1,
                        server_id or None,
                        access_token=token,
                    )
                    last_game_at = _first_match_timestamp(history)
                    source = "sgp"
                except RuntimeError:
                    pass
            if not token or (source != "sgp" and last_game_at is None):
                history = await optional(
                    f"/lol-match-history/v1/products/lol/{quote(puuid, safe='')}/matches",
                    params={"begIndex": 0, "endIndex": 0},
                )
                last_game_at = _first_match_timestamp(history)
        return puuid, {
            "last_game_at": last_game_at,
            "friends_since": friend_since_by_summoner.get(str(friend.get("summonerId"))),
            "source": source,
        }

    metadata = dict(await asyncio.gather(*(enrich(friend) for friend in friend_rows)))
    return {"friends": metadata, "count": len(metadata), "source": "sgp" if token else "lcu"}


def _require_toolkit_account_actions() -> None:
    if not league_lab_service.settings.toolkit_account_actions_enabled:
        raise HTTPException(status_code=403, detail="账号写入工具已关闭，请先在工具箱中开启")


async def _require_live_phase(expected: str | set[str] | tuple[str, ...]) -> str:
    """Require a fresh LCU gameflow phase immediately before an account write.

    The cached ``LeagueLabService.phase`` is intentionally not used here: the
    manual action routes are allowed to write only when the current LCU state
    still permits the operation.  Tests can provide a fake ``request`` method
    without starting a real League client.
    """
    expected_phases = {expected} if isinstance(expected, str) else set(expected)
    try:
        phase = str(await league_lab_service.request("GET", "/lol-gameflow/v1/gameflow-phase") or "")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=f"无法确认当前游戏阶段：{exc}") from exc
    if phase not in expected_phases:
        readable = "、".join(sorted(expected_phases))
        raise HTTPException(status_code=409, detail=f"当前游戏阶段为 {phase or '未知'}，此操作仅允许在 {readable} 阶段执行")
    return phase


async def _require_reroll_evidence() -> dict:
    """Require the live LCU session to expose a usable champion reroll.

    ``benchEnabled`` identifies the bench surface, but newer ARAM-family
    queues (including KIWI) can expose a bench without the legacy reroll
    action.  Keep the write contract tied to the two explicit LCU fields used
    by LeagueAkari: ``allowRerolling`` and a positive ``rerollsRemaining``.
    """

    try:
        session = await league_lab_service.request("GET", "/lol-champ-select/v1/session")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=f"无法读取最新英雄选择会话：{exc}") from exc
    if not isinstance(session, dict):
        raise HTTPException(status_code=409, detail="无法确认客户端当前是否支持重随")
    try:
        rerolls_remaining = int(session.get("rerollsRemaining") or 0)
    except (TypeError, ValueError):
        rerolls_remaining = 0
    if session.get("allowRerolling") is not True or rerolls_remaining <= 0:
        raise HTTPException(status_code=409, detail="客户端当前未明确支持重随或没有剩余次数")
    return session


def _unique_nonempty(values: list[str], label: str) -> list[str]:
    normalized = [str(value).strip() for value in values]
    if any(not value for value in normalized):
        raise HTTPException(status_code=422, detail=f"{label}包含空 ID")
    if len(set(normalized)) != len(normalized):
        raise HTTPException(status_code=422, detail=f"{label}包含重复 ID")
    return normalized


@router.post("/toolkit/claims/mission")
async def league_claim_mission_reward(body: MissionRewardClaim):
    _require_toolkit_account_actions()
    reward_group_ids = _unique_nonempty(body.reward_group_ids, "任务奖励")
    try:
        missions = await league_lab_service.request("GET", "/lol-missions/v1/missions")
        mission = next(
            (row for row in (missions or []) if isinstance(row, dict) and str(row.get("id")) == body.mission_id),
            None,
        )
        if not mission or mission.get("status") != "SELECT_REWARDS":
            raise HTTPException(status_code=409, detail="任务当前不可领取或状态已经变化")
        available = {str(row.get("rewardGroup")) for row in (mission.get("rewards") or []) if row.get("rewardGroup")}
        if not set(reward_group_ids).issubset(available):
            raise HTTPException(status_code=422, detail="所选任务奖励不属于当前任务")
        strategy = mission.get("rewardStrategy") or {}
        minimum = max(1, int(strategy.get("selectMinGroupCount") or 1))
        maximum = max(minimum, int(strategy.get("selectMaxGroupCount") or 1))
        if not minimum <= len(reward_group_ids) <= maximum:
            raise HTTPException(status_code=422, detail=f"该任务必须选择 {minimum}–{maximum} 个奖励组")
        await league_lab_service.request(
            "PUT",
            f"/lol-missions/v1/player/{quote(body.mission_id, safe='')}",
            json_body={"rewardGroups": reward_group_ids},
        )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"claimed": True, "mission_id": body.mission_id, "reward_group_ids": reward_group_ids}


@router.post("/toolkit/claims/reward")
async def league_claim_reward_grant(body: RewardGrantClaim):
    _require_toolkit_account_actions()
    selection_ids = _unique_nonempty(body.selection_ids, "普通奖励")
    try:
        grants = await league_lab_service.request(
            "GET", "/lol-rewards/v1/grants", params={"status": "PENDING_SELECTION"}
        )
        grant = next(
            (
                row
                for row in (grants or [])
                if isinstance(row, dict) and str((row.get("info") or {}).get("id")) == body.grant_id
            ),
            None,
        )
        info = (grant or {}).get("info") or {}
        group = (grant or {}).get("rewardGroup") or {}
        if not grant or info.get("status") != "PENDING_SELECTION":
            raise HTTPException(status_code=409, detail="奖励当前不可领取或状态已经变化")
        if str(group.get("id")) != body.reward_group_id:
            raise HTTPException(status_code=422, detail="奖励组与当前待领取奖励不匹配")
        available = {str(row.get("id")) for row in (group.get("rewards") or []) if row.get("id")}
        if not set(selection_ids).issubset(available):
            raise HTTPException(status_code=422, detail="所选奖励不属于当前奖励组")
        strategy = group.get("selectionStrategyConfig") or {}
        minimum = max(1, int(strategy.get("minSelectionsAllowed") or 1))
        maximum = max(minimum, int(strategy.get("maxSelectionsAllowed") or 1))
        if not minimum <= len(selection_ids) <= maximum:
            raise HTTPException(status_code=422, detail=f"该奖励必须选择 {minimum}–{maximum} 项")
        payload = {
            "grantId": body.grant_id,
            "rewardGroupId": body.reward_group_id,
            "selections": selection_ids,
        }
        await league_lab_service.request(
            "POST",
            f"/lol-rewards/v1/grants/{quote(body.grant_id, safe='')}/select",
            json_body=payload,
        )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"claimed": True, **payload}


@router.post("/toolkit/claims/event")
async def league_claim_event_rewards(body: EventRewardClaim):
    _require_toolkit_account_actions()
    try:
        events = await league_lab_service.request("GET", "/lol-event-hub/v1/events")
        event = next(
            (row for row in (events or []) if isinstance(row, dict) and str(row.get("eventId")) == body.event_id),
            None,
        )
        if not event or int(((event.get("eventInfo") or {}).get("unclaimedRewardCount") or 0)) <= 0:
            raise HTTPException(status_code=409, detail="活动当前没有可领取奖励或状态已经变化")
        await league_lab_service.request(
            "POST",
            f"/lol-event-hub/v1/events/{quote(body.event_id, safe='')}/reward-track/claim-all",
        )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"claimed": True, "event_id": body.event_id}


@router.post("/toolkit/friends/delete")
async def league_delete_friends(body: FriendDeleteRequest):
    _require_toolkit_account_actions()
    friend_ids = _unique_nonempty(body.friend_ids, "好友列表")
    try:
        friends = await league_lab_service.request("GET", "/lol-chat/v1/friends")
        current = {str(row.get("id")) for row in (friends or []) if isinstance(row, dict) and row.get("id")}
        missing = [friend_id for friend_id in friend_ids if friend_id not in current]
        if missing:
            raise HTTPException(status_code=409, detail="部分好友已不存在或列表已经变化，请刷新后重试")
        deleted = []
        for friend_id in friend_ids:
            await league_lab_service.request("DELETE", f"/lol-chat/v1/friends/{quote(friend_id, safe='')}")
            deleted.append(friend_id)
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"deleted": deleted, "count": len(deleted)}


@router.get("/toolkit/lobby-options")
async def league_lobby_options():
    async def optional(method: str, path: str):
        try:
            return await league_lab_service.request(method, path)
        except RuntimeError:
            return None

    queues, party, self_eligibility, lobby, strawberry, loadouts = await asyncio.gather(
        optional("GET", "/lol-game-data/assets/v1/queues.json"),
        optional("POST", "/lol-lobby/v2/eligibility/party"),
        optional("POST", "/lol-lobby/v2/eligibility/self"),
        optional("GET", "/lol-lobby/v2/lobby"),
        optional("GET", "/lol-game-data/assets/v1/strawberry-hub.json"),
        optional("GET", "/lol-loadouts/v4/loadouts/scope/account"),
    )
    if not isinstance(queues, (list, dict)):
        raise HTTPException(status_code=409, detail="客户端没有返回可用队列目录")
    queue_rows = queues if isinstance(queues, list) else list(queues.values()) if isinstance(queues, dict) else []
    party_ids = {int(row.get("queueId")) for row in (party or []) if isinstance(row, dict) and row.get("queueId")}
    self_ids = {
        int(row.get("queueId")) for row in (self_eligibility or []) if isinstance(row, dict) and row.get("queueId")
    }
    normalized_queues = [
        {
            "id": int(row["id"]),
            "name": str(row.get("name") or row["id"]),
            "description": str(row.get("description") or ""),
            "eligible": int(row["id"]) in party_ids and int(row["id"]) in self_ids,
        }
        for row in queue_rows
        if isinstance(row, dict) and row.get("id")
    ]
    maps = []
    if isinstance(strawberry, list) and strawberry:
        for row in strawberry[0].get("MapDisplayInfoList") or []:
            value = row.get("value") or {}
            map_value = value.get("Map") or {}
            if map_value.get("ContentId") and map_value.get("ItemId"):
                maps.append(
                    {
                        "name": str(value.get("Name") or map_value["ItemId"]),
                        "content_id": str(map_value["ContentId"]),
                        "item_id": int(map_value["ItemId"]),
                    }
                )
    return {
        "queues": sorted(normalized_queues, key=lambda row: (not row["eligible"], row["name"])),
        "lobby": lobby if isinstance(lobby, dict) else None,
        "strawberry": {
            "active": str((((lobby or {}).get("gameConfig") or {}).get("gameMode") or "")).upper() == "STRAWBERRY",
            "maps": maps,
            "difficulties": [1, 2, 3],
            "loadout_available": bool(loadouts),
        },
    }


@router.post("/toolkit/lobby/create")
async def league_create_queue_lobby(body: QueueLobbyCreate):
    _require_toolkit_account_actions()
    try:
        party, self_eligibility = await asyncio.gather(
            league_lab_service.request("POST", "/lol-lobby/v2/eligibility/party"),
            league_lab_service.request("POST", "/lol-lobby/v2/eligibility/self"),
        )
        party_ids = {int(row.get("queueId")) for row in (party or []) if isinstance(row, dict) and row.get("queueId")}
        self_ids = {
            int(row.get("queueId"))
            for row in (self_eligibility or [])
            if isinstance(row, dict) and row.get("queueId")
        }
        if body.queue_id not in party_ids or body.queue_id not in self_ids:
            raise HTTPException(status_code=409, detail="当前账号或队伍不满足该队列的创建条件")
        await league_lab_service.request("POST", "/lol-lobby/v2/lobby", json_body={"queueId": body.queue_id})
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"created": True, "queue_id": body.queue_id}


@router.post("/toolkit/lobby/leave")
async def league_leave_lobby(body: LeaveLobbyRequest):
    _require_toolkit_account_actions()
    try:
        lobby = await league_lab_service.request("GET", "/lol-lobby/v2/lobby")
        if not isinstance(lobby, dict) or not lobby:
            raise HTTPException(status_code=409, detail="当前不在房间中")
        await league_lab_service.request("DELETE", "/lol-lobby/v2/lobby")
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"left": True}


async def _require_strawberry_lobby() -> dict:
    lobby = await league_lab_service.request("GET", "/lol-lobby/v2/lobby")
    mode = str((((lobby or {}).get("gameConfig") or {}).get("gameMode") or "")).upper()
    if mode != "STRAWBERRY":
        raise HTTPException(status_code=409, detail="当前房间不是无尽狂潮模式")
    return lobby


@router.put("/toolkit/strawberry/player")
async def league_update_strawberry_player(body: StrawberryPlayerUpdate):
    _require_toolkit_account_actions()
    try:
        await _require_strawberry_lobby()
        champions = await league_lab_service.request("GET", "/lol-game-data/assets/v1/champion-summary.json")
        available = {int(row.get("id")) for row in (champions or []) if isinstance(row, dict) and row.get("id")}
        if body.champion_id not in available:
            raise HTTPException(status_code=422, detail="英雄 ID 不在当前客户端目录中")
        payload = [{
            "championId": body.champion_id,
            "positionPreference": "UNSELECTED",
            "spell1": body.map_item_id,
            "spell2": body.difficulty,
        }]
        await league_lab_service.request(
            "PUT", "/lol-lobby/v1/lobby/members/localMember/player-slots", json_body=payload
        )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"applied": True, "champion_id": body.champion_id, "map_item_id": body.map_item_id, "difficulty": body.difficulty}


@router.put("/toolkit/strawberry/map")
async def league_update_strawberry_map(body: StrawberryMapUpdate):
    _require_toolkit_account_actions()
    try:
        await _require_strawberry_lobby()
        strawberry = await league_lab_service.request("GET", "/lol-game-data/assets/v1/strawberry-hub.json")
        available = set()
        if isinstance(strawberry, list) and strawberry:
            for row in strawberry[0].get("MapDisplayInfoList") or []:
                map_value = ((row.get("value") or {}).get("Map") or {})
                if map_value.get("ContentId") and map_value.get("ItemId"):
                    available.add((str(map_value["ContentId"]), int(map_value["ItemId"])))
        if (body.content_id, body.item_id) not in available:
            raise HTTPException(status_code=422, detail="所选地图不在当前客户端无尽狂潮目录中")
        await league_lab_service.request(
            "PUT", "/lol-lobby/v2/lobby/strawberryMapId",
            json_body={"contentId": body.content_id, "itemId": body.item_id},
        )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"applied": True, "content_id": body.content_id, "item_id": body.item_id}


@router.put("/toolkit/strawberry/difficulty")
async def league_update_strawberry_difficulty(body: StrawberryDifficultyUpdate):
    _require_toolkit_account_actions()
    try:
        await _require_strawberry_lobby()
        loadouts = await league_lab_service.request("GET", "/lol-loadouts/v4/loadouts/scope/account")
        content_id = str((loadouts or [{}])[0].get("id") or "")
        if not content_id:
            raise HTTPException(status_code=409, detail="客户端没有返回账号级配装")
        await league_lab_service.request(
            "PATCH",
            f"/lol-loadouts/v4/loadouts/{quote(content_id, safe='')}",
            json_body={"loadout": {"STRAWBERRY_DIFFICULTY": {
                "inventoryType": "STRAWBERRY_LOADOUT_ITEM", "itemId": body.difficulty
            }}},
        )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"applied": True, "difficulty": body.difficulty}


@router.get("/toolkit/profile/skins/{champion_id}")
async def league_profile_skin_catalog(champion_id: int):
    if champion_id <= 0:
        raise HTTPException(status_code=422, detail="英雄 ID 无效")
    try:
        details = await league_lab_service.request(
            "GET", f"/lol-game-data/assets/v1/champions/{champion_id}.json"
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    skins = []
    seen = set()
    for skin in (details or {}).get("skins") or []:
        candidates = [skin, *((skin.get("questSkinInfo") or {}).get("tiers") or [])]
        for candidate in candidates:
            skin_id = int(candidate.get("id") or 0)
            if not skin_id or skin_id in seen:
                continue
            seen.add(skin_id)
            augments = []
            for augment in ((candidate.get("skinAugments") or {}).get("augments") or []):
                if augment.get("contentId") is not None and augment.get("overlays"):
                    augments.append({"content_id": str(augment["contentId"]), "overlays": augment.get("overlays") or []})
            skins.append({
                "id": skin_id,
                "name": str(candidate.get("name") or skin_id),
                "splash_path": candidate.get("uncenteredSplashPath") or "",
                "augments": augments,
            })
    return {"champion_id": champion_id, "skins": skins}


@router.post("/toolkit/profile/background")
async def league_update_profile_background(body: ProfileBackgroundUpdate):
    _require_toolkit_account_actions()
    catalog = await league_profile_skin_catalog(body.champion_id)
    skin = next((row for row in catalog["skins"] if row["id"] == body.skin_id), None)
    if not skin:
        raise HTTPException(status_code=422, detail="所选皮肤不属于该英雄的当前客户端目录")
    available_augments = {row["content_id"] for row in skin["augments"]}
    if body.augment_id is not None and body.augment_id not in available_augments:
        raise HTTPException(status_code=422, detail="所选皮肤挂件不属于该皮肤")
    try:
        await league_lab_service.request(
            "POST", "/lol-summoner/v1/current-summoner/summoner-profile",
            json_body={"key": "backgroundSkinId", "value": body.skin_id},
        )
        if body.augment_id is not None:
            await league_lab_service.request(
                "POST", "/lol-summoner/v1/current-summoner/summoner-profile",
                json_body={"key": "backgroundSkinAugments", "value": body.augment_id},
            )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"applied": True, "skin_id": body.skin_id, "augment_id": body.augment_id}


@router.post("/toolkit/profile/action")
async def league_profile_utility_action(body: ProfileUtilityAction):
    _require_toolkit_account_actions()
    try:
        if body.action == "banner-accent":
            await league_lab_service.request(
                "POST", "/lol-challenges/v1/update-player-preferences/", json_body={"bannerAccent": "2"}
            )
        elif body.action == "remove-prestige-crest":
            current = await league_lab_service.request("GET", "/lol-regalia/v2/current-summoner/regalia")
            await league_lab_service.request(
                "PUT", "/lol-regalia/v2/current-summoner/regalia",
                json_body={
                    "preferredCrestType": "prestige",
                    "preferredBannerType": (current or {}).get("bannerType"),
                    "selectedPrestigeCrest": 22,
                },
            )
        elif body.action == "clear-challenge-tokens":
            chat_me = await league_lab_service.request("GET", "/lol-chat/v1/me")
            await league_lab_service.request(
                "POST", "/lol-challenges/v1/update-player-preferences/",
                json_body={"challengeIds": [], "bannerAccent": ((chat_me or {}).get("lol") or {}).get("bannerIdSelected")},
            )
        else:
            loadouts = await league_lab_service.request("GET", "/lol-loadouts/v4/loadouts/scope/account")
            content_id = str((loadouts or [{}])[0].get("id") or "")
            if not content_id:
                raise HTTPException(status_code=409, detail="客户端没有返回账号级配装")
            emote_keys = (
                "EMOTES_ACE", "EMOTES_FIRST_BLOOD", "EMOTES_VICTORY", "EMOTES_WHEEL_CENTER",
                "EMOTES_WHEEL_UPPER", "EMOTES_WHEEL_RIGHT", "EMOTES_WHEEL_UPPER_RIGHT",
                "EMOTES_WHEEL_UPPER_LEFT", "EMOTES_WHEEL_LOWER", "EMOTES_START", "EMOTES_WHEEL_LEFT",
                "EMOTES_WHEEL_LOWER_RIGHT", "EMOTES_WHEEL_LOWER_LEFT",
            )
            await league_lab_service.request(
                "PATCH", f"/lol-loadouts/v4/loadouts/{quote(content_id, safe='')}",
                json_body={"loadout": {key: {"inventoryType": "EMOTE", "itemId": -1} for key in emote_keys}},
            )
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"applied": True, "action": body.action}


async def _load_league_match_payload(
    game_id: int,
    source: Literal["auto", "lcu", "sgp"],
    include_timeline: bool,
) -> tuple[dict, dict | None, str, list[str]]:
    if game_id <= 0:
        raise HTTPException(status_code=422, detail="Game ID 必须是正整数")
    errors: list[str] = []
    game = None
    active_source = ""
    if source in {"auto", "lcu"}:
        try:
            candidate = await league_lab_service.request("GET", f"/lol-match-history/v1/games/{game_id}")
            candidate = candidate.get("json") if isinstance(candidate, dict) and isinstance(candidate.get("json"), dict) else candidate
            if not isinstance(candidate, dict) or not isinstance(candidate.get("participants"), list):
                raise RuntimeError("LCU 对局返回格式无效")
            game, active_source = candidate, "lcu"
        except RuntimeError as exc:
            errors.append(str(exc))
    if game is None and source in {"auto", "sgp"}:
        try:
            game, active_source = await _sgp_game_summary(game_id), "sgp"
        except RuntimeError as exc:
            errors.append(str(exc))
    if game is None:
        raise HTTPException(status_code=404, detail="；".join(errors) or "未找到该 Game ID")
    timeline = None
    if include_timeline:
        try:
            timeline = (
                await league_lab_service.request("GET", f"/lol-match-history/v1/game-timelines/{game_id}")
                if active_source == "lcu"
                else await _sgp_game_details(game_id)
            )
            if isinstance(timeline, dict) and isinstance(timeline.get("json"), dict):
                timeline = timeline["json"]
        except RuntimeError as exc:
            errors.append(str(exc))
    return game, timeline, active_source, errors


def _normalize_match_timeline(game: dict, timeline: dict | None, source: str) -> dict:
    identities: dict[int, dict] = {}
    for row in game.get("participantIdentities") or []:
        if not isinstance(row, dict):
            continue
        participant_id = int(row.get("participantId") or 0)
        if participant_id:
            identities[participant_id] = row.get("player") if isinstance(row.get("player"), dict) else row
    participants = []
    for index, row in enumerate(game.get("participants") or []):
        if not isinstance(row, dict):
            continue
        participant_id = int(row.get("participantId") or index + 1)
        identity = identities.get(participant_id) or {}
        participants.append({
            "participant_id": participant_id,
            "puuid": row.get("puuid") or identity.get("puuid") or identity.get("playerPuuid") or "",
            "game_name": row.get("riotIdGameName") or row.get("gameName") or identity.get("gameName") or identity.get("displayName") or identity.get("summonerName") or "",
            "tag_line": row.get("riotIdTagline") or row.get("tagLine") or identity.get("tagLine") or "",
            "champion_id": int(row.get("championId") or 0),
            "team_id": int(row.get("teamId") or 0),
        })
    frames = []
    event_keys = {
        "type", "timestamp", "participantId", "creatorId", "killerId", "victimId",
        "assistingParticipantIds", "monsterType", "monsterSubType", "buildingType",
        "towerType", "laneType", "itemId", "skillSlot", "levelUpType", "afterId",
        "beforeId", "killType", "multiKillLength", "position",
    }
    participant_frame_keys = {
        "totalGold", "currentGold", "level", "xp", "minionsKilled", "jungleMinionsKilled",
        "position", "damageStats",
    }
    champion_stat_keys = {
        "health", "healthMax", "healthRegen", "power", "powerMax", "powerRegen",
        "attackDamage", "attackSpeed", "abilityPower", "abilityHaste", "cooldownReduction",
        "armor", "magicResist", "armorPen", "armorPenPercent", "bonusArmorPenPercent",
        "magicPen", "magicPenPercent", "bonusMagicPenPercent", "movementSpeed", "lifesteal",
        "physicalVamp", "spellVamp", "omnivamp", "ccReduction",
    }
    for frame in (timeline or {}).get("frames") or []:
        if not isinstance(frame, dict):
            continue
        normalized_events = [
            {key: value for key, value in event.items() if key in event_keys}
            for event in (frame.get("events") or [])
            if isinstance(event, dict) and event.get("type")
        ]
        normalized_participants = {}
        for participant_id, participant_frame in (frame.get("participantFrames") or {}).items():
            if not isinstance(participant_frame, dict):
                continue
            normalized_frame = {
                key: value for key, value in participant_frame.items() if key in participant_frame_keys
            }
            champion_stats = participant_frame.get("championStats")
            if isinstance(champion_stats, dict):
                normalized_frame["championStats"] = {
                    key: value for key, value in champion_stats.items()
                    if key in champion_stat_keys and isinstance(value, (int, float))
                }
            normalized_participants[str(participant_id)] = normalized_frame
        frames.append({
            "timestamp": int(frame.get("timestamp") or 0),
            "participant_frames": normalized_participants,
            "events": normalized_events,
        })
    return {
        "source": source,
        "game_id": game.get("gameId"),
        "map_id": game.get("mapId"),
        "participants": participants,
        "frames": frames,
        "events": [event for frame in frames for event in frame["events"]],
        "frame_count": len(frames),
        "event_count": sum(len(frame["events"]) for frame in frames),
    }


@router.get("/matches/{game_id}/details")
async def league_match_details(
    game_id: int,
    source: Literal["auto", "lcu", "sgp"] = "auto",
):
    game, timeline, active_source, errors = await _load_league_match_payload(game_id, source, True)
    result = _normalize_match_timeline(game, timeline, active_source)
    result["warnings"] = errors
    return result


@router.get("/toolkit/game-preview/{game_id}")
async def league_game_preview(
    game_id: int,
    source: Literal["auto", "lcu", "sgp"] = "auto",
    include_timeline: bool = True,
):
    game, timeline, active_source, errors = await _load_league_match_payload(game_id, source, include_timeline)
    names = await _champion_names()
    result = _normalize_game_preview(game, names, active_source, timeline)
    result["warnings"] = errors
    return result


async def _league_game_settings_path() -> Path:
    install_root = await league_lab_service.request("GET", "/data-store/v1/install-dir")
    if not isinstance(install_root, str) or not install_root.strip():
        raise RuntimeError("LCU 未返回游戏安装目录")
    root = Path(install_root).expanduser().resolve()
    region = (league_lab_service.credentials.region if league_lab_service.credentials else "").upper()
    config_dir = (root.parent / "Game" / "Config") if region == "TENCENT" else (root / "Config")
    settings_path = (config_dir / "PersistedSettings.json").resolve()
    if not settings_path.is_file():
        raise RuntimeError("未找到 PersistedSettings.json")
    return settings_path


async def _league_game_settings_file_mode() -> str:
    settings_path = await _league_game_settings_path()
    return "writable" if settings_path.stat().st_mode & stat.S_IWRITE else "readonly"


def _league_client_window_handles():
    if os.name != "nt":
        raise RuntimeError("League 客户端窗口调整仅支持 Windows")
    user32 = ctypes.windll.user32
    user32.FindWindowW.restype = wintypes.HWND
    user32.FindWindowExW.restype = wintypes.HWND
    parent = user32.FindWindowW("RCLIENT", "League of Legends")
    child = user32.FindWindowExW(parent, None, None, "CefBrowserWindow") if parent else None
    if not parent or not child:
        raise RuntimeError("未找到 LeagueClientUx 主窗口，请先显示客户端")
    return user32, parent, child


def _league_client_window_info() -> dict:
    user32, parent, _ = _league_client_window_handles()
    rect = wintypes.RECT()
    if not user32.GetWindowRect(parent, ctypes.byref(rect)):
        raise RuntimeError("读取 LeagueClientUx 窗口尺寸失败")
    dpi = int(user32.GetDpiForWindow(parent)) if hasattr(user32, "GetDpiForWindow") else 96
    return {
        "width": int(rect.right - rect.left),
        "height": int(rect.bottom - rect.top),
        "left": int(rect.left),
        "top": int(rect.top),
        "dpi": dpi or 96,
        "scale_factor": round((dpi or 96) / 96, 3),
        "supported": True,
    }


def _resize_league_client_window(base_width: int, base_height: int, zoom: float) -> dict:
    if zoom <= 0:
        raise RuntimeError("LeagueClientUx 返回了无效缩放比例")
    user32, parent, child = _league_client_window_handles()
    width = max(1, round(base_width * zoom))
    height = max(1, round(base_height * zoom))
    screen_width = int(user32.GetSystemMetrics(0))
    screen_height = int(user32.GetSystemMetrics(1))
    x, y = (screen_width - width) // 2, (screen_height - height) // 2
    swp_no_zorder = 0x0004
    if not user32.SetWindowPos(parent, None, x, y, width, height, swp_no_zorder):
        raise RuntimeError("调整 LeagueClientUx 主窗口失败，可能需要管理员权限")
    if not user32.SetWindowPos(child, None, 0, 0, width, height, swp_no_zorder):
        raise RuntimeError("调整 LeagueClientUx 内容窗口失败，可能需要管理员权限")
    return {**_league_client_window_info(), "base_width": base_width, "base_height": base_height, "zoom": zoom}


@router.get("/toolkit/game-settings-file")
async def league_game_settings_file_status():
    try:
        path = await _league_game_settings_path()
        mode = "writable" if path.stat().st_mode & stat.S_IWRITE else "readonly"
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"mode": mode, "file_name": path.name}


@router.put("/toolkit/game-settings-file")
async def league_game_settings_file_update(body: GameSettingsFileModeUpdate):
    _require_toolkit_account_actions()
    try:
        path = await _league_game_settings_path()
        os.chmod(path, stat.S_IREAD if body.mode == "readonly" else stat.S_IREAD | stat.S_IWRITE)
        mode = await _league_game_settings_file_mode()
    except (OSError, RuntimeError) as exc:
        raise HTTPException(status_code=409, detail=f"修改游戏设置文件属性失败: {exc}") from exc
    if mode != body.mode:
        raise HTTPException(status_code=409, detail="游戏设置文件属性未按预期生效")
    return {"mode": mode, "file_name": path.name, "applied": True}


@router.get("/toolkit/client-window")
async def league_client_window_status():
    try:
        info = await asyncio.to_thread(_league_client_window_info)
        zoom = await league_lab_service.request("GET", "/riotclient/zoom-scale")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {**info, "zoom": float(zoom) if isinstance(zoom, (int, float)) else None}


@router.put("/toolkit/client-window")
async def league_client_window_resize(body: LeagueClientWindowResize):
    try:
        zoom = await league_lab_service.request("GET", "/riotclient/zoom-scale")
        if not isinstance(zoom, (int, float)):
            raise RuntimeError("LCU 未返回客户端缩放比例")
        info = await asyncio.to_thread(_resize_league_client_window, body.base_width, body.base_height, float(zoom))
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {**info, "applied": True}


@router.put("/toolkit/chat-presence")
async def league_update_chat_presence(body: ChatPresenceUpdate):
    _require_toolkit_account_actions()
    patch: dict[str, str] = {}
    if body.availability is not None:
        patch["availability"] = body.availability
    if body.status_message is not None:
        patch["statusMessage"] = body.status_message
    if not patch:
        raise HTTPException(status_code=422, detail="没有需要应用的聊天状态")
    try:
        if body.status_message is not None:
            league_lab_service._interrupt_chat_ready_automation()
        await league_lab_service.request("PUT", "/lol-chat/v1/me", json_body=patch)
        current = await league_lab_service.request("GET", "/lol-chat/v1/me")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"chat_presence": current if isinstance(current, dict) else patch, "applied": True}


@router.put("/toolkit/ranked-status")
async def league_update_ranked_status(body: RankedStatusUpdate):
    _require_toolkit_account_actions()
    try:
        await league_lab_service.apply_ranked_status(body)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"ranked_status": body.model_dump(), "applied": True}


@router.post("/toolkit/terminate-game-client")
async def league_terminate_game_client(body: TerminateGameClientRequest):
    _require_toolkit_account_actions()
    try:
        pid = await asyncio.to_thread(_terminate_foreground_league_game_client)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"terminated": True, "pid": pid}


_in_game_send_lock = asyncio.Lock()
_in_game_send_cancel = threading.Event()


async def _send_league_preset_lines(lines: list[str]) -> dict:
    normalized = [line.strip() for line in lines if line.strip()]
    if not normalized:
        raise HTTPException(status_code=422, detail="预设内容不能为空")
    if len(normalized) > 10 or any(len(line) > 300 for line in normalized):
        raise HTTPException(status_code=422, detail="每次最多发送 10 行，单行不能超过 300 字")
    try:
        live_phase = await league_lab_service.request("GET", "/lol-gameflow/v1/gameflow-phase")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    live_phase = str(live_phase or league_lab_service.phase)
    if live_phase in {"Lobby", "ChampSelect"}:
        try:
            conversations = await league_lab_service.request("GET", "/lol-chat/v1/conversations")
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        wanted = {"ChampSelect": {"championselect", "champion-select"}, "Lobby": {"customgame", "custom-game"}}[live_phase]
        conversation = next(
            (
                row for row in conversations
                if isinstance(row, dict) and str(row.get("type") or "").lower() in wanted and row.get("id")
            ),
            None,
        ) if isinstance(conversations, list) else None
        if not conversation:
            raise HTTPException(status_code=409, detail="当前阶段没有可用的 LCU 对话")
        try:
            await league_lab_service.request(
                "POST", f"/lol-chat/v1/conversations/{conversation['id']}/messages",
                json_body={"body": "\n".join(normalized), "type": "chat"},
            )
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        return {"sent": True, "phase": live_phase, "line_count": len(normalized), "transport": "lcu"}
    if live_phase != "InProgress":
        raise HTTPException(status_code=409, detail="当前不在房间、英雄选择或游戏进行阶段")
    async with _in_game_send_lock:
        _in_game_send_cancel.clear()
        pid = None
        for index, line in enumerate(normalized):
            if _in_game_send_cancel.is_set():
                return {"sent": False, "cancelled": True, "phase": live_phase, "line_count": index, "transport": "native", "pid": pid}
            try:
                pid = await asyncio.to_thread(_send_text_to_foreground_league_game, line)
            except RuntimeError as exc:
                raise HTTPException(status_code=409, detail=str(exc)) from exc
            if index < len(normalized) - 1:
                await asyncio.sleep(league_lab_service.settings.in_game_send_interval_ms / 1000)
        _in_game_send_cancel.clear()
    return {"sent": True, "phase": live_phase, "line_count": len(normalized), "transport": "native", "pid": pid}


@router.post("/toolkit/in-game-presets/send")
async def league_send_in_game_preset(body: InGamePresetSend):
    _require_toolkit_account_actions()
    settings = league_lab_service.settings
    if not settings.in_game_send_enabled:
        raise HTTPException(status_code=403, detail="请先启用游戏内预设发送")
    preset = next((row for row in settings.in_game_fixed_presets if row.id == body.preset_id), None)
    if not preset:
        raise HTTPException(status_code=404, detail="未找到该固定文字预设")
    if body.trigger == "manual":
        if body.confirmation != "我确认发送":
            raise HTTPException(status_code=422, detail="确认短语不正确")
    elif not preset.shortcut:
        raise HTTPException(status_code=409, detail="该预设没有启用快捷键")
    result = await _send_league_preset_lines(preset.content.splitlines())
    return {**result, "preset_id": preset.id, "trigger": body.trigger}


@router.post("/toolkit/in-game-presets/send-lines")
async def league_send_in_game_lines(body: InGameAdHocSend):
    _require_toolkit_account_actions()
    if not league_lab_service.settings.in_game_send_enabled:
        raise HTTPException(status_code=403, detail="请先启用游戏内预设发送")
    if body.trigger == "manual":
        if body.confirmation != "我确认发送":
            raise HTTPException(status_code=422, detail="确认短语不正确")
    else:
        if not body.kind or not body.target:
            raise HTTPException(status_code=422, detail="快捷键发送缺少预设类型或目标")
        shortcuts = getattr(league_lab_service.settings, f"in_game_{body.kind}_shortcuts")
        if not getattr(shortcuts, body.target):
            raise HTTPException(status_code=409, detail="该分析预设目标没有启用快捷键")
    return await _send_league_preset_lines(body.lines)


@router.post("/toolkit/in-game-presets/cancel")
async def league_cancel_in_game_send():
    _in_game_send_cancel.set()
    return {"cancel_requested": True}


@router.post("/toolkit/chat-message")
async def league_send_chat_message(body: ChatMessageSend):
    _require_toolkit_account_actions()
    lines = [line.strip() for line in body.lines if line.strip()]
    if not lines:
        raise HTTPException(status_code=422, detail="消息内容不能为空")
    if any(len(line) > 300 for line in lines):
        raise HTTPException(status_code=422, detail="单行消息不能超过 300 字")
    try:
        conversations = await league_lab_service.request("GET", "/lol-chat/v1/conversations")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    wanted = {"ChampSelect": {"championselect", "champion-select"}, "Lobby": {"customgame", "custom-game"}}.get(
        league_lab_service.phase, set()
    )
    conversation = next(
        (row for row in conversations if isinstance(row, dict) and str(row.get("type") or "").lower() in wanted),
        None,
    ) if isinstance(conversations, list) else None
    if not conversation or not conversation.get("id"):
        raise HTTPException(status_code=409, detail="当前不在可发送消息的房间或英雄选择阶段")
    try:
        await league_lab_service.request(
            "POST",
            f"/lol-chat/v1/conversations/{conversation['id']}/messages",
            json_body={"body": "\n".join(lines), "type": "chat"},
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return {"sent": True, "phase": league_lab_service.phase, "line_count": len(lines)}


@router.post("/actions/cancel-auto-accept")
async def cancel_auto_accept():
    """Cancel only the current local auto-accept deadline.

    This route deliberately does not use the account-write gate and never
    calls the LCU.  It is a local automation control, not a ReadyCheck
    response; the user can still explicitly choose accept/decline afterwards.
    """
    if league_lab_service.phase == "ReadyCheck":
        # ``inf`` is the scheduler's terminal sentinel for this phase.  A
        # phase transition resets it, while the current loop will not create a
        # new deadline after the user cancels it.
        league_lab_service._accept_due_at = float("inf")
    else:
        league_lab_service._accept_due_at = None
    league_lab_service.last_action = "已取消本次自动接受"
    league_lab_service.last_action_at = time.time()
    return league_lab_service.status()


@router.post("/actions/cancel-auto-matchmaking")
async def cancel_auto_matchmaking():
    """Cancel the local auto-search plan without touching the LCU queue."""
    await league_lab_service.cancel_auto_matchmaking("normal")
    league_lab_service.settings.auto_matchmaking_enabled = False
    league_lab_service._write_settings(league_lab_service.settings)
    league_lab_service.last_action = "已取消自动匹配"
    league_lab_service.last_action_at = time.time()
    return league_lab_service.status()


@router.post("/actions/{action}")
async def run_league_lab_action(action: Literal["accept", "decline-ready-check", "play-again", "reconnect", "start-matchmaking", "stop-matchmaking"]):
    endpoints = {
        "accept": ("接受对局", "/lol-matchmaking/v1/ready-check/accept"),
        "decline-ready-check": ("拒绝对局", "/lol-matchmaking/v1/ready-check/decline"),
        "play-again": ("返回房间", "/lol-lobby/v2/play-again"),
        "reconnect": ("重新连接", "/lol-gameflow/v1/reconnect"),
        "start-matchmaking": ("开始匹配", "/lol-lobby/v2/lobby/matchmaking/search"),
        "stop-matchmaking": ("停止匹配", "/lol-lobby/v2/lobby/matchmaking/search"),
    }
    label, path = endpoints[action]
    _require_toolkit_account_actions()
    allowed_phases = {
        "accept": {"ReadyCheck"},
        "decline-ready-check": {"ReadyCheck"},
        "play-again": {"PreEndOfGame", "EndOfGame", "WaitingForStats"},
        "reconnect": {"Reconnect"},
        "start-matchmaking": {"Lobby"},
        "stop-matchmaking": {"Matchmaking"},
    }
    try:
        await _require_live_phase(allowed_phases[action])
        method = "DELETE" if action == "stop-matchmaking" else "POST"
        await league_lab_service._record_action(label, method, path)
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return league_lab_service.status()


async def _league_champ_select_trade_action(trade_id: str, operation: Literal["accept", "decline"]):
    """Perform one explicit champion-swap action after a fresh session check."""
    _require_toolkit_account_actions()
    await _require_live_phase("ChampSelect")
    try:
        session = await league_lab_service.request("GET", "/lol-champ-select/v1/session")
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=f"无法读取最新英雄选择会话：{exc}") from exc
    normalized = LeagueLabService._normalize_champ_select(session)
    trade = next(
        (item for item in normalized.get("trades") or [] if str(item.get("id")) == str(trade_id)),
        None,
    )
    if trade is None:
        raise HTTPException(status_code=409, detail="换英雄请求不存在或已过期")
    if not trade.get("actionable"):
        raise HTTPException(status_code=409, detail=f"换英雄请求当前不可操作（状态：{trade.get('state') or '未知'}）")
    path = f"/lol-champ-select/v1/session/champion-swaps/{quote(str(trade_id), safe='')}/{operation}"
    try:
        await league_lab_service.request("POST", path)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    league_lab_service.last_action = f"已{('接受' if operation == 'accept' else '拒绝')}换英雄请求"
    league_lab_service.last_action_at = time.time()
    return league_lab_service.status()


@router.post("/champ-select/trades/{trade_id}/accept")
async def league_champ_select_trade_accept(trade_id: str):
    return await _league_champ_select_trade_action(trade_id, "accept")


@router.post("/champ-select/trades/{trade_id}/decline")
async def league_champ_select_trade_decline(trade_id: str):
    return await _league_champ_select_trade_action(trade_id, "decline")


@router.post("/champ-select/bench/swap/{champion_id}")
async def league_bench_swap(champion_id: int):
    if champion_id <= 0:
        raise HTTPException(status_code=422, detail="无效英雄 ID")
    _require_toolkit_account_actions()
    try:
        await _require_live_phase("ChampSelect")
        await league_lab_service._record_action(
            f"已从备战席换取英雄 {champion_id}",
            "POST",
            f"/lol-champ-select/v1/session/bench/swap/{champion_id}",
        )
        await league_lab_service._refresh_state()
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return league_lab_service.status()


@router.post("/champ-select/select/{champion_id}")
async def league_champ_select_select_or_bench(champion_id: int):
    """Select a Mini champion using the same two paths as LeagueAkari.

    In the ARAM-style BAN_PICK subset phase, the first click is a normal
    ``session/actions/:id`` pick/lock request when the local player has not
    selected a champion yet.  Once a champion is held (or during finalization),
    the same surface is a bench swap.  Keep this decision on a fresh session
    snapshot so the Mini never guesses from its last poll.  The account-write
    gate is intentionally checked before any LCU request.
    """

    if champion_id <= 0:
        raise HTTPException(status_code=422, detail="无效英雄 ID")
    _require_toolkit_account_actions()
    try:
        await _require_live_phase("ChampSelect")
        session = await league_lab_service.request("GET", "/lol-champ-select/v1/session")
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=f"无法读取最新英雄选择会话：{exc}") from exc

    if not isinstance(session, dict):
        raise HTTPException(status_code=409, detail="无法读取当前英雄选择会话")

    timer = session.get("timer") if isinstance(session.get("timer"), dict) else {}
    timer_phase = str(timer.get("phase") or "")
    local_cell = session.get("localPlayerCellId")
    local_member = next(
        (
            row for row in (session.get("myTeam") or [])
            if isinstance(row, dict) and str(row.get("cellId")) == str(local_cell)
        ),
        {},
    )
    current_champion = int(local_member.get("championId") or 0)

    # LeagueAkari scopes the direct pick to the first unfinished local pick
    # action and only performs it from BAN_PICK.  In subset modes, the
    # candidate must also be in the server-provided subset pool.
    is_direct_pick = timer_phase == "BAN_PICK" and current_champion <= 0
    if is_direct_pick:
        try:
            pickable = await league_lab_service.request("GET", "/lol-champ-select/v1/pickable-champion-ids")
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=f"无法确认英雄当前是否可选：{exc}") from exc
        try:
            pickable_ids = {int(value) for value in (pickable or [])}
        except (TypeError, ValueError):
            pickable_ids = set()
        if champion_id not in pickable_ids:
            raise HTTPException(status_code=409, detail="该英雄当前不可选或已被占用")

    if is_direct_pick and bool(session.get("allowSubsetChampionPicks")):
        try:
            subset = await league_lab_service.request(
                "GET", "/lol-lobby-team-builder/champ-select/v1/subset-champion-list"
            )
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=f"无法确认当前子集英雄池：{exc}") from exc
        try:
            subset_ids = {int(value) for value in (subset or [])}
        except (TypeError, ValueError):
            subset_ids = set()
        if champion_id not in subset_ids:
            raise HTTPException(status_code=409, detail="该英雄不在当前可选英雄子集内")

    if is_direct_pick:
        actions = [group for group in (session.get("actions") or []) if isinstance(group, list)]
        current_group = next(
            (
                group
                for group in actions
                if group and not all(
                    bool(action.get("completed"))
                    for action in group
                    if isinstance(action, dict)
                )
            ),
            [],
        )
        first_pick = next(
            (
                action
                for action in current_group
                if isinstance(action, dict)
                and str(action.get("actorCellId")) == str(local_cell)
                and str(action.get("type") or "") == "pick"
                and not bool(action.get("completed"))
            ),
            None,
        )
        if first_pick is None or first_pick.get("id") is None:
            raise HTTPException(status_code=409, detail="当前没有可直接锁定的英雄选择动作")
        action_id = quote(str(first_pick.get("id")), safe="")
        try:
            await league_lab_service.request(
                "PATCH",
                f"/lol-champ-select/v1/session/actions/{action_id}",
                json_body={"championId": champion_id, "completed": True, "type": "pick"},
            )
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=f"选择英雄失败：{exc}") from exc
        league_lab_service.last_action = f"已从 Mini 选择并锁定英雄 {champion_id}"
    else:
        bench_ids = {
            int(item.get("championId"))
            for item in (session.get("benchChampions") or [])
            if isinstance(item, dict) and item.get("championId") is not None
        }
        if champion_id not in bench_ids:
            raise HTTPException(status_code=409, detail="该英雄不在当前备战席中")
        try:
            await league_lab_service.request(
                "POST", f"/lol-champ-select/v1/session/bench/swap/{champion_id}"
            )
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=f"换取英雄失败：{exc}") from exc
        league_lab_service.last_action = f"已从 Mini 换取英雄 {champion_id}"

    league_lab_service.last_action_at = time.time()
    try:
        await league_lab_service._refresh_state()
    except RuntimeError:
        # The write already reached the LCU; a transient refresh failure must
        # not turn a successful action into an unsafe retry prompt.
        pass
    return league_lab_service.status()


@router.put("/champ-select/auto-select-temporarily-disabled")
async def league_set_auto_select_temporarily_disabled(body: AutoSelectTemporaryDisableBody):
    if league_lab_service.phase != "ChampSelect":
        league_lab_service.auto_select_temporarily_disabled = False
        raise HTTPException(status_code=409, detail="当前不在英雄选择阶段")
    league_lab_service.auto_select_temporarily_disabled = body.disabled
    if body.disabled:
        league_lab_service._champion_action_due_at.clear()
    return league_lab_service.status()


@router.post("/champ-select/dodge")
async def league_champ_select_dodge(body: ChampSelectDodgeRequest):
    _require_toolkit_account_actions()
    try:
        phase = str(await league_lab_service.request("GET", "/lol-gameflow/v1/gameflow-phase") or "")
        if phase != "ChampSelect":
            raise HTTPException(status_code=409, detail="当前不在英雄选择阶段")
        await league_lab_service.request(
            "POST",
            "/lol-login/v1/session/invoke",
            params={
                "destination": "lcdsServiceProxy",
                "method": "call",
                "args": '["", "teambuilder-draft", "quitV2", ""]',
            },
            json_body={"data": ["", "teambuilder-draft", "quitV2", ""]},
        )
        league_lab_service.last_action = "已执行一次英雄选择秒退"
        league_lab_service.last_action_at = time.time()
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return league_lab_service.status()


@router.post("/champ-select/dodge-loop/start")
async def league_champ_select_dodge_loop_start(body: ChampSelectDodgeLoopRequest):
    """Start the explicitly confirmed, cancellable five-worker dodge loop."""

    _require_toolkit_account_actions()
    try:
        await _require_live_phase("ChampSelect")
    except HTTPException:
        raise
    try:
        league_lab_service.start_dodge_loop()
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return league_lab_service.status()


@router.post("/champ-select/dodge-loop/cancel")
async def league_champ_select_dodge_loop_cancel():
    """Cancel only the local dodge-loop task; it never writes to the LCU."""

    league_lab_service._terminate_dodge_loop("user-cancelled")
    return league_lab_service.status()


@router.post("/champ-select/reroll")
async def league_champ_select_reroll():
    _require_toolkit_account_actions()
    try:
        await _require_live_phase("ChampSelect")
        await _require_reroll_evidence()
        await league_lab_service._record_action(
            "已使用一次重随",
            "POST",
            "/lol-champ-select/v1/session/my-selection/reroll",
        )
        await league_lab_service._refresh_state()
    except HTTPException:
        raise
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return league_lab_service.status()


@router.post("/champ-select/reroll-charity")
async def league_champ_select_charity_reroll(body: ChampSelectCharityRerollRequest):
    """Reroll and, only with fresh evidence, take the original champion back.

    The second write is deliberately conservative: the original champion must
    be present in the fresh bench, be currently pickable, and be usable in the
    current bench phase.  Missing/ambiguous optional LCU data means the
    reroll has happened but no follow-up swap is attempted.
    """

    _require_toolkit_account_actions()
    await _require_live_phase("ChampSelect")
    before = await _require_reroll_evidence()

    local_cell = before.get("localPlayerCellId")
    local_member = next(
        (
            item for item in (before.get("myTeam") or [])
            if isinstance(item, dict) and str(item.get("cellId")) == str(local_cell)
        ),
        None,
    )
    try:
        original_champion_id = int((local_member or {}).get("championId") or 0)
        rerolls_remaining = int(before.get("rerollsRemaining") or 0)
    except (TypeError, ValueError):
        original_champion_id = 0
        rerolls_remaining = 0
    if original_champion_id <= 0:
        raise HTTPException(status_code=409, detail="当前没有可记录的已选择英雄，已阻止慈善重随")
    try:
        await league_lab_service.request(
            "POST",
            "/lol-champ-select/v1/session/my-selection/reroll",
        )
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=f"重随失败：{exc}") from exc

    report = {
        "original_champion_id": original_champion_id,
        "rerolled": True,
        "grabbed_back": False,
        "swap_reason": None,
    }

    try:
        after = await league_lab_service.request("GET", "/lol-champ-select/v1/session")
    except RuntimeError as exc:
        after = None
        report["swap_reason"] = f"无法读取重随后的会话：{exc}"

    if isinstance(after, dict):
        bench_rows = after.get("benchChampions")
        timer = after.get("timer")
        phase = str((timer or {}).get("phase") or "") if isinstance(timer, dict) else ""
        bench_ids = []
        if isinstance(bench_rows, list):
            for item in bench_rows:
                if isinstance(item, dict):
                    try:
                        champion_id = int(item.get("championId") or 0)
                    except (TypeError, ValueError):
                        champion_id = 0
                    if champion_id > 0:
                        bench_ids.append(champion_id)
        if not bool(after.get("benchEnabled")):
            report["swap_reason"] = "重随后的会话没有启用备战席"
        elif original_champion_id not in bench_ids:
            report["swap_reason"] = "原英雄没有进入重随后的备战席"
        elif phase not in {"FINALIZATION", "BAN_PICK"}:
            report["swap_reason"] = "当前阶段不允许从备战席换取英雄"
        else:
            try:
                pickable = await league_lab_service.request(
                    "GET", "/lol-champ-select/v1/pickable-champion-ids"
                )
            except RuntimeError as exc:
                pickable = None
                report["swap_reason"] = f"无法确认原英雄当前是否可选：{exc}"
            pickable_ids = set()
            if isinstance(pickable, list):
                for value in pickable:
                    try:
                        pickable_ids.add(int(value))
                    except (TypeError, ValueError):
                        continue
            if report["swap_reason"] is None and original_champion_id not in pickable_ids:
                report["swap_reason"] = "原英雄当前不可选，未执行换回"
            if report["swap_reason"] is None and phase == "BAN_PICK":
                subset = after.get("allowSubsetChampionPicks")
                try:
                    subset_ids = await league_lab_service.request(
                        "GET", "/lol-champ-select/v1/subset-champion-ids"
                    )
                except RuntimeError as exc:
                    subset_ids = None
                    report["swap_reason"] = f"无法确认子集英雄池：{exc}"
                if report["swap_reason"] is None and not bool(subset):
                    report["swap_reason"] = "当前 BAN_PICK 阶段没有子集选人证据"
                elif report["swap_reason"] is None and (
                    not isinstance(subset_ids, list) or original_champion_id not in {
                        int(value) for value in subset_ids
                        if isinstance(value, (int, float, str)) and str(value).isdigit()
                    }
                ):
                    report["swap_reason"] = "原英雄不在当前子集英雄池，未执行换回"

        if report["swap_reason"] is None:
            # Re-check both controls immediately before the optional second
            # account write.  The reroll may have changed the phase or the
            # user may have disabled account actions while it was settling.
            _require_toolkit_account_actions()
            try:
                await _require_live_phase("ChampSelect")
                await league_lab_service.request(
                    "POST",
                    f"/lol-champ-select/v1/session/bench/swap/{original_champion_id}",
                )
                report["grabbed_back"] = True
            except HTTPException as exc:
                report["swap_reason"] = exc.detail
            except RuntimeError as exc:
                report["swap_reason"] = f"换回原英雄失败：{exc}"

    if report["grabbed_back"]:
        league_lab_service.last_action = f"已慈善重随并取回原英雄 {original_champion_id}"
    else:
        league_lab_service.last_action = "已慈善重随，原英雄未满足安全换回条件"
    league_lab_service.last_action_at = time.time()
    try:
        await league_lab_service._refresh_state()
    except RuntimeError:
        # The reroll itself already completed; preserve its result if the
        # optional post-write refresh races a client phase transition.
        pass
    result = league_lab_service.status()
    result["charity_reroll"] = report
    return result


@router.post("/champ-select/skin/{skin_id}")
async def league_champ_select_skin(skin_id: int):
    if skin_id <= 0:
        raise HTTPException(status_code=422, detail="无效的皮肤 ID")
    _require_toolkit_account_actions()
    await _require_live_phase("ChampSelect")
    selector = league_lab_service.champ_select.get("skin_selector") or {}
    allowed = {int(row.get("id") or 0) for row in selector.get("skins") or []}
    if skin_id not in allowed:
        raise HTTPException(status_code=409, detail="该皮肤当前不可用或不属于本账号")
    if selector.get("disabled"):
        raise HTTPException(status_code=409, detail="当前阶段不可切换皮肤")
    try:
        await league_lab_service.request(
            "PATCH",
            "/lol-champ-select/v1/session/my-selection",
            json_body={"selectedSkinId": skin_id},
        )
        await league_lab_service._refresh_state()
        return league_lab_service.status()
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail=str(exc)) from exc
    return league_lab_service.status()
