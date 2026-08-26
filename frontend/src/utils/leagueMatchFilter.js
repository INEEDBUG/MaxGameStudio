const NUMERIC_FIELDS = new Set([
  "kills", "deaths", "assists", "kda", "damage", "gold", "cs", "queue_id", "champion_id",
  "duration_seconds", "spell1_id", "spell2_id", "kill_participation", "vision_score", "damage_taken",
  "gold_spent", "tower_damage", "healing", "time_ccing", "solo_kills", "double_kills", "triple_kills",
  "quadra_kills", "penta_kills", "level", "played_at",
]);

const CHALLENGE_FIELDS = {
  kill_participation: "killParticipation",
  vision_score: "visionScore",
  damage_taken: "damageTakenOnTeamPercentage",
  gold_spent: "goldPerMinute",
  tower_damage: "damageToTurrets",
  healing: "effectiveHealAndShielding",
  time_ccing: "enemyChampionImmobilizations",
  solo_kills: "soloKills",
};

export function leagueMatchRuleValue(match, field) {
  if (field === "kda") return (Number(match.kills || 0) + Number(match.assists || 0)) / Math.max(1, Number(match.deaths || 0));
  if (field === "has_item") return match?.items || [];
  if (field === "has_spell") return [match?.spell1_id, match?.spell2_id].filter(Boolean);
  if (field === "has_perk") return match?.perks || [];
  if (field === "has_augment") return match?.augments || [];
  if (field === "is_remake") return Number(match?.duration_seconds || 0) > 0 && Number(match.duration_seconds) <= 300;
  if (field === "is_matched_game") return String(match?.game_type || "").toUpperCase() === "MATCHED_GAME";
  if (field === "is_pve_game") return /PVE|BOT/.test(`${match?.game_type || ""} ${match?.game_mode || ""}`.toUpperCase());
  if (field in CHALLENGE_FIELDS) return match?.[field] ?? match?.challenges?.[CHALLENGE_FIELDS[field]];
  return match?.[field];
}

export function matchesLeagueRule(match, rule) {
  if (!rule?.field || rule.value === "") return true;
  const scope = String(rule.scope || "self");
  if (scope !== "self") {
    const [quantifier, group] = scope.split("-");
    const selfPuuid = String(match?.participant_puuid || "");
    const selfTeam = String(match?.team_id ?? "");
    const participants = (match?.participants || []).filter((participant) => {
      if (group === "allies") return String(participant?.team_id ?? "") === selfTeam && (!selfPuuid || String(participant?.puuid || "") !== selfPuuid);
      if (group === "enemies") return String(participant?.team_id ?? "") !== selfTeam;
      return !selfPuuid || String(participant?.puuid || "") !== selfPuuid;
    });
    if (!participants.length) return false;
    const predicate = (participant) => matchesLeagueRule(participant, { ...rule, scope: "self" });
    return quantifier === "every" ? participants.every(predicate) : participants.some(predicate);
  }
  const actual = leagueMatchRuleValue(match, rule.field);
  if (Array.isArray(actual)) {
    const expected = String(rule.value);
    const contains = actual.some((item) => String(item) === expected);
    return rule.operator === "neq" ? !contains : contains;
  }
  if (typeof actual === "boolean") {
    const expected = ["true", "1", "yes", "是", "胜利"].includes(String(rule.value).toLowerCase());
    return rule.operator === "neq" ? actual !== expected : actual === expected;
  }
  if (NUMERIC_FIELDS.has(rule.field)) {
    const left = Number(actual || 0), right = Number(rule.value);
    if (!Number.isFinite(right)) return true;
    if (rule.operator === "gte") return left >= right;
    if (rule.operator === "lte") return left <= right;
    if (rule.operator === "neq") return left !== right;
    return left === right;
  }
  const left = String(actual || "").toLowerCase(), right = String(rule.value).toLowerCase();
  if (rule.operator === "neq") return left !== right;
  if (rule.operator === "contains") return left.includes(right);
  return left === right;
}

export function matchesLeagueRules(match, rules = [], logic = "and") {
  if (!rules.length) return true;
  return logic === "or" ? rules.some((rule) => matchesLeagueRule(match, rule)) : rules.every((rule) => matchesLeagueRule(match, rule));
}

export function matchesLeagueRuleTree(match, node) {
  if (!node) return true;
  if (node.type === "rule") return matchesLeagueRule(match, node);
  const children = Array.isArray(node.children) ? node.children : [];
  if (!children.length) return true;
  const values = children.map((child) => matchesLeagueRuleTree(match, child));
  const combined = node.logic === "or" ? values.some(Boolean) : values.every(Boolean);
  return node.negate ? !combined : combined;
}
