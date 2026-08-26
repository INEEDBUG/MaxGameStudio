import API from "./api";

export async function fetchLeagueLabStatus() {
  const { data } = await API.get("/league-lab/status");
  return data;
}

export async function fetchLeagueClients() {
  const { data } = await API.get("/league-lab/clients");
  return data;
}

export async function selectLeagueClient(pid) {
  const { data } = await API.post("/league-lab/clients/select", { pid });
  return data;
}

export async function fetchLeagueClientInstallations() {
  const { data } = await API.get("/league-lab/installations");
  return data;
}

export async function launchLeagueClient(kind) {
  const { data } = await API.post("/league-lab/installations/launch", { kind });
  return data;
}

export async function saveLeagueLabSettings(settings) {
  const { data } = await API.put("/league-lab/settings", settings);
  return data;
}

export async function runLeagueLabAction(action) {
  const { data } = await API.post(`/league-lab/actions/${action}`);
  return data;
}

export async function declineLeagueReadyCheck() {
  return runLeagueLabAction("decline-ready-check");
}

export async function cancelLeagueAutoAccept() {
  const { data } = await API.post("/league-lab/actions/cancel-auto-accept");
  return data;
}

export async function stopLeagueMatchmaking() {
  return runLeagueLabAction("stop-matchmaking");
}

export async function fetchLeagueMatches(limit = 20, begIndex = 0) {
  const params = { limit };
  if (Number(begIndex) > 0) params.beg_index = Number(begIndex);
  const { data } = await API.get("/league-lab/matches", { params });
  return data;
}

export async function collectLeagueMatches(options = {}) {
  const { data } = await API.post("/league-lab/matches/collect", options);
  return data;
}

export async function fetchLeagueReplay(gameId) {
  const { data } = await API.get(`/league-lab/replays/${encodeURIComponent(gameId)}`);
  return data;
}

export async function downloadLeagueReplay(gameId, match) {
  const playedAtValue = match?.played_at;
  const numericPlayedAt = Number(playedAtValue || 0);
  const playedAt = Number.isFinite(numericPlayedAt) && numericPlayedAt > 0
    ? numericPlayedAt
    : (Date.parse(String(playedAtValue || "")) || 0);
  const durationMs = Number(match?.duration_seconds || 0) * 1000;
  const { data } = await API.post(`/league-lab/replays/${encodeURIComponent(gameId)}/download`, {
    game_version: match?.game_version || "",
    game_type: match?.game_type || "",
    queue_id: Number(match?.queue_id || 0),
    game_end: playedAt > 0 ? playedAt + durationMs : 0,
  });
  return data;
}

export async function watchLeagueReplay(gameId) {
  const { data } = await API.post(`/league-lab/replays/${encodeURIComponent(gameId)}/watch`);
  return data;
}

export async function fetchLeagueChampions() {
  const { data } = await API.get("/league-lab/champions");
  return data;
}

export async function fetchLeagueLoadoutCatalog() {
  const { data } = await API.get("/league-lab/loadout-catalog");
  return data;
}

export async function fetchCurrentLeaguePlayer() {
  const { data } = await API.get("/league-lab/players/current");
  return data;
}

export async function fetchLeaguePlayer(puuid, matchLimit = 20, begIndex = 0, serverId = "") {
  const { data } = await API.get(`/league-lab/players/${encodeURIComponent(puuid)}`, { params: { match_limit: matchLimit, beg_index: begIndex, server_id: serverId || undefined } });
  return data;
}

export async function fetchLeaguePlayerCollection(puuid, limit = 100) {
  const { data } = await API.get(`/league-lab/players/${encodeURIComponent(puuid)}/collection`, { params: { limit } });
  return data;
}

export async function fetchLeaguePlayerMastery(puuid) {
  const { data } = await API.get(`/league-lab/players/${encodeURIComponent(puuid)}/mastery`);
  return data;
}

export async function fetchLeaguePlayerJungleAnalysis(puuid, limit = 6, serverId = "") {
  const { data } = await API.get(`/league-lab/players/${encodeURIComponent(puuid)}/jungle-analysis`, { params: { limit, server_id: serverId || undefined } });
  return data;
}

export async function searchLeaguePlayer(gameName, tagLine, serverId = "") {
  const { data } = await API.get("/league-lab/players/search", { params: { game_name: gameName, tag_line: tagLine, server_id: serverId || undefined } });
  return data;
}

export async function fetchLeaguePlayerSearchServers() {
  const { data } = await API.get("/league-lab/players/search-servers");
  return data;
}

export async function fetchRecentLeaguePlayers(limit = 40) {
  const { data } = await API.get("/league-lab/players/recent", { params: { limit } });
  return data;
}

export async function fetchLeaguePlayerSearchHistory(limit = 40) {
  const { data } = await API.get("/league-lab/players/search-history", { params: { limit } });
  return data;
}

export async function pinLeaguePlayerSearchHistory(puuid, pinned, serverId = "") {
  const { data } = await API.put(`/league-lab/players/search-history/${encodeURIComponent(puuid)}/pin`, { pinned }, { params: { server_id: serverId || undefined } });
  return data;
}

export async function deleteLeaguePlayerSearchHistory(puuid, serverId = "") {
  const { data } = await API.delete(`/league-lab/players/search-history/${encodeURIComponent(puuid)}`, { params: { server_id: serverId || undefined } });
  return data;
}

export async function fetchLeaguePlayerFriends() {
  const { data } = await API.get("/league-lab/players/friends");
  return data;
}

export async function spectateLeagueFriend(puuid) {
  const { data } = await API.post(`/league-lab/players/friends/${encodeURIComponent(puuid)}/spectate`);
  return data;
}

export async function fetchLeaguePlayerEncounters(puuid, page = 1, pageSize = 10) {
  const { data } = await API.get(`/league-lab/players/${encodeURIComponent(puuid)}/encounters`, { params: { page, page_size: pageSize } });
  return data;
}

export async function deleteLeaguePlayerEncounter(puuid, gameId) {
  const { data } = await API.delete(`/league-lab/players/${encodeURIComponent(puuid)}/encounters/${encodeURIComponent(gameId)}`);
  return data;
}

export async function saveLeaguePlayerTag(puuid, tag) {
  const { data } = await API.put(`/league-lab/players/${encodeURIComponent(puuid)}/tag`, tag);
  return data;
}

export async function fetchLeaguePlayerTags({ page = 1, pageSize = 20, query = "", currentAccountOnly = true } = {}) {
  const { data } = await API.get("/league-lab/player-tags", { params: {
    page,
    page_size: pageSize,
    query,
    current_account_only: currentAccountOnly,
  } });
  return data;
}

export async function updateLeaguePlayerTag(tagKey, tag) {
  const { data } = await API.put(`/league-lab/player-tags/${encodeURIComponent(tagKey)}`, tag);
  return data;
}

export async function deleteLeaguePlayerTag(tagKey) {
  const { data } = await API.delete(`/league-lab/player-tags/${encodeURIComponent(tagKey)}`);
  return data;
}

export async function importLeaguePlayerTags(rows) {
  const { data } = await API.post("/league-lab/player-tags/import", { rows });
  return data;
}

export async function fetchLeagueOngoingGame() {
  const { data } = await API.get("/league-lab/ongoing-game");
  return data;
}

export async function fetchLeagueCooldownTimerState() {
  const { data } = await API.get("/league-lab/cooldown-timer/state");
  return data;
}

export async function sendLeagueCooldownTimerText(text) {
  const { data } = await API.post("/league-lab/cooldown-timer/send", { text });
  return data;
}

export async function fetchLeagueToolkitOverview() {
  const { data } = await API.get("/league-lab/toolkit/overview");
  return data;
}

export async function fetchLeagueFriendMetadata() {
  const { data } = await API.get("/league-lab/toolkit/friends/metadata");
  return data;
}

export async function claimLeagueMissionReward(missionId, rewardGroupIds, confirmation) {
  const { data } = await API.post("/league-lab/toolkit/claims/mission", {
    mission_id: missionId,
    reward_group_ids: rewardGroupIds,
    confirmation,
  });
  return data;
}

export async function claimLeagueRewardGrant(grantId, rewardGroupId, selectionIds, confirmation) {
  const { data } = await API.post("/league-lab/toolkit/claims/reward", {
    grant_id: grantId,
    reward_group_id: rewardGroupId,
    selection_ids: selectionIds,
    confirmation,
  });
  return data;
}

export async function claimLeagueEventRewards(eventId, confirmation) {
  const { data } = await API.post("/league-lab/toolkit/claims/event", {
    event_id: eventId,
    confirmation,
  });
  return data;
}

export async function deleteLeagueFriends(friendIds, confirmation) {
  const { data } = await API.post("/league-lab/toolkit/friends/delete", {
    friend_ids: friendIds,
    confirmation,
  });
  return data;
}

export async function fetchLeagueLobbyOptions() {
  const { data } = await API.get("/league-lab/toolkit/lobby-options");
  return data;
}

export async function createLeagueQueueLobby(queueId, confirmation) {
  const { data } = await API.post("/league-lab/toolkit/lobby/create", { queue_id: queueId, confirmation });
  return data;
}

export async function leaveLeagueLobby(confirmation) {
  const { data } = await API.post("/league-lab/toolkit/lobby/leave", { confirmation });
  return data;
}

export async function updateLeagueStrawberryPlayer(championId, mapItemId, difficulty, confirmation) {
  const { data } = await API.put("/league-lab/toolkit/strawberry/player", {
    champion_id: championId, map_item_id: mapItemId, difficulty, confirmation,
  });
  return data;
}

export async function updateLeagueStrawberryMap(contentId, itemId, confirmation) {
  const { data } = await API.put("/league-lab/toolkit/strawberry/map", {
    content_id: contentId, item_id: itemId, confirmation,
  });
  return data;
}

export async function updateLeagueStrawberryDifficulty(difficulty, confirmation) {
  const { data } = await API.put("/league-lab/toolkit/strawberry/difficulty", { difficulty, confirmation });
  return data;
}

export async function fetchLeagueProfileSkins(championId) {
  const { data } = await API.get(`/league-lab/toolkit/profile/skins/${championId}`);
  return data;
}

export async function updateLeagueProfileBackground(championId, skinId, augmentId, confirmation) {
  const { data } = await API.post("/league-lab/toolkit/profile/background", {
    champion_id: championId, skin_id: skinId, augment_id: augmentId || null, confirmation,
  });
  return data;
}

export async function runLeagueProfileUtilityAction(action, confirmation) {
  const { data } = await API.post("/league-lab/toolkit/profile/action", { action, confirmation });
  return data;
}

export async function fetchLeagueGamePreview(gameId, source = "auto", includeTimeline = true) {
  const { data } = await API.get(`/league-lab/toolkit/game-preview/${encodeURIComponent(gameId)}`, {
    params: { source, include_timeline: includeTimeline },
  });
  return data;
}

export async function fetchLeagueMatchDetails(gameId, source = "auto") {
  const { data } = await API.get(`/league-lab/matches/${encodeURIComponent(gameId)}/details`, {
    params: { source },
  });
  return data;
}

export async function updateLeagueChatPresence(payload) {
  const { data } = await API.put("/league-lab/toolkit/chat-presence", payload);
  return data;
}

export async function updateLeagueRankedStatus(payload) {
  const { data } = await API.put("/league-lab/toolkit/ranked-status", payload);
  return data;
}

export async function sendLeagueChatMessage(lines) {
  const { data } = await API.post("/league-lab/toolkit/chat-message", { lines });
  return data;
}

export async function sendLeagueInGamePreset(presetId, trigger = "manual", confirmation = "") {
  const { data } = await API.post("/league-lab/toolkit/in-game-presets/send", {
    preset_id: presetId,
    trigger,
    confirmation,
  });
  return data;
}

export async function sendLeagueInGameLines(lines, confirmation = "", trigger = "manual", kind = null, target = null) {
  const { data } = await API.post("/league-lab/toolkit/in-game-presets/send-lines", { lines, confirmation, trigger, kind, target });
  return data;
}

export async function cancelLeagueInGameSend() {
  const { data } = await API.post("/league-lab/toolkit/in-game-presets/cancel");
  return data;
}

export async function terminateLeagueGameClient(confirmation = "我确认结束游戏") {
  const { data } = await API.post("/league-lab/toolkit/terminate-game-client", { confirmation });
  return data;
}

export async function fetchLeagueGameSettingsFile() {
  const { data } = await API.get("/league-lab/toolkit/game-settings-file");
  return data;
}

export async function updateLeagueGameSettingsFile(mode) {
  const { data } = await API.put("/league-lab/toolkit/game-settings-file", { mode });
  return data;
}

export async function fetchLeagueClientWindow() {
  const { data } = await API.get("/league-lab/toolkit/client-window");
  return data;
}

export async function resizeLeagueClientWindow(baseWidth, baseHeight) {
  const { data } = await API.put("/league-lab/toolkit/client-window", { base_width: baseWidth, base_height: baseHeight });
  return data;
}

export async function swapLeagueBenchChampion(championId) {
  const { data } = await API.post(`/league-lab/champ-select/bench/swap/${championId}`);
  return data;
}

// Mirrors LeagueAkari's Mini click behavior: in BAN_PICK this can lock the
// first local pick; otherwise the same candidate is exchanged from the bench.
export async function selectLeagueChampionFromMini(championId) {
  const { data } = await API.post(`/league-lab/champ-select/select/${championId}`);
  return data;
}

export async function rerollLeagueChampion() {
  const { data } = await API.post("/league-lab/champ-select/reroll");
  return data;
}

export async function charityRerollLeagueChampion(confirmation) {
  const { data } = await API.post("/league-lab/champ-select/reroll-charity", { confirmation });
  return data;
}

export async function startLeagueDodgeLoop(confirmation) {
  const { data } = await API.post("/league-lab/champ-select/dodge-loop/start", { confirmation });
  return data;
}

export async function cancelLeagueDodgeLoop() {
  const { data } = await API.post("/league-lab/champ-select/dodge-loop/cancel");
  return data;
}

export async function selectLeagueChampionSkin(skinId) {
  const { data } = await API.post(`/league-lab/champ-select/skin/${skinId}`);
  return data;
}

export async function setLeagueAutoSelectTemporarilyDisabled(disabled) {
  const { data } = await API.put("/league-lab/champ-select/auto-select-temporarily-disabled", { disabled });
  return data;
}

export async function dodgeLeagueChampSelect(confirmation) {
  const { data } = await API.post("/league-lab/champ-select/dodge", { confirmation });
  return data;
}

export async function acceptLeagueChampSelectTrade(tradeId) {
  const { data } = await API.post(`/league-lab/champ-select/trades/${encodeURIComponent(tradeId)}/accept`);
  return data;
}

export async function declineLeagueChampSelectTrade(tradeId) {
  const { data } = await API.post(`/league-lab/champ-select/trades/${encodeURIComponent(tradeId)}/decline`);
  return data;
}
