const PROFILE_KEYS = ["P", "A", "S"];

export const VALORANT_RESOLUTION_PRESETS = [
  { id: "1568x1080", width: 1568, height: 1080, label: "1568×1080", hintKey: "valorant.stretch.popular" },
  { id: "1440x1080", width: 1440, height: 1080, label: "1440×1080" },
  { id: "1280x960", width: 1280, height: 960, label: "1280×960" },
  { id: "1024x768", width: 1024, height: 768, label: "1024×768" },
  { id: "1680x1050", width: 1680, height: 1050, label: "1680×1050" },
];

export const DEFAULT_DISPLAY_STATUS = {
  overall: "unknown",
  gpu: { status: "unknown", name: "" },
  monitor: { status: "unknown", name: "" },
  refreshRate: { status: "unknown", value: null },
  checkedAt: null,
  source: "unknown",
  rawMonitorStatus: null,
  safeToSkipDisable: false,
};

export const CROSSHAIR_COLORS = {
  white: "#f6f7fb",
  green: "#61d67a",
  yellowgreen: "#b8db50",
  yellow: "#f4d35e",
  cyan: "#55d6e8",
  pink: "#f48fb1",
  red: "#ef6262",
};

export const DEFAULT_CROSSHAIR_PROFILE = {
  color: "green",
  outlines: true,
  outlineOpacity: 0.8,
  outlineThickness: 1,
  centerDot: false,
  centerDotOpacity: 1,
  centerDotThickness: 2,
  innerLines: true,
  innerLinesOpacity: 1,
  innerLinesLength: 4,
  innerLinesThickness: 2,
  innerLinesOffset: 2,
  outerLines: false,
  outerLinesOpacity: 0.5,
  outerLinesLength: 2,
  outerLinesThickness: 2,
  outerLinesOffset: 3,
  firingError: false,
  movementError: false,
};

export const DEFAULT_CROSSHAIR_PROFILES = PROFILE_KEYS.reduce((profiles, key) => {
  profiles[key] = { ...DEFAULT_CROSSHAIR_PROFILE };
  return profiles;
}, {});

const VALID_STATUS = new Set(["ready", "warning", "unknown", "error"]);

function normalizeStatus(value) {
  if (typeof value === "string") return VALID_STATUS.has(value) ? value : "unknown";
  if (value && typeof value === "object") {
    const status = typeof value.status === "string" && VALID_STATUS.has(value.status) ? value.status : "unknown";
    return { ...value, status };
  }
  return "unknown";
}

function normalizeDevice(value, fallback = {}) {
  const normalized = normalizeStatus(value);
  return typeof normalized === "string" ? { ...fallback, status: normalized } : { ...fallback, ...normalized };
}

export function normalizeDisplayStatus(value) {
  const raw = value && typeof value === "object" ? value : {};
  const gpu = normalizeDevice(raw.gpu ?? raw.gpu_status, { name: raw.gpu_name || "" });
  const monitor = normalizeDevice(raw.monitor ?? raw.monitor_status, { name: raw.monitor_name || "" });
  const refreshRate = normalizeDevice(raw.refreshRate ?? raw.refresh_rate ?? raw.refresh_status, {
    value: raw.refresh_rate_hz ?? raw.refreshRateHz ?? null,
  });
  const overall = normalizeStatus(raw.overall ?? raw.status);
  return {
    overall: typeof overall === "string" ? overall : overall.status,
    gpu,
    monitor,
    refreshRate,
    checkedAt: raw.checkedAt ?? raw.checked_at ?? null,
    source: raw.source || "backend",
    rawMonitorStatus: raw.rawMonitorStatus ?? raw.raw_monitor_status ?? raw.monitor_disable_status ?? raw.monitor?.monitor_disable_status ?? null,
    safeToSkipDisable: raw.safeToSkipDisable === true || raw.safe_to_skip_disable === true,
  };
}

export function isDisplayStatusReady(status) {
  const value = status || DEFAULT_DISPLAY_STATUS;
  return value.overall === "ready" && [value.gpu, value.monitor, value.refreshRate].every((item) => item?.status === "ready");
}

export function resolutionFromSelection(selection, custom = {}) {
  if (selection === "custom") {
    return {
      width: Math.max(320, Math.round(Number(custom.width) || 0)),
      height: Math.max(240, Math.round(Number(custom.height) || 0)),
      preset: "custom",
    };
  }
  const preset = VALORANT_RESOLUTION_PRESETS.find((item) => item.id === selection) || VALORANT_RESOLUTION_PRESETS[0];
  return { width: preset.width, height: preset.height, preset: preset.id };
}

function clampNumber(value, min, max, fallback) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(max, Math.max(min, parsed));
}

export function normalizeCrosshairProfile(value) {
  const raw = value && typeof value === "object" ? value : {};
  return {
    ...DEFAULT_CROSSHAIR_PROFILE,
    ...raw,
    color: Object.prototype.hasOwnProperty.call(CROSSHAIR_COLORS, raw.color) ? raw.color : DEFAULT_CROSSHAIR_PROFILE.color,
    outlines: raw.outlines !== false,
    outlineOpacity: clampNumber(raw.outlineOpacity, 0, 1, DEFAULT_CROSSHAIR_PROFILE.outlineOpacity),
    outlineThickness: clampNumber(raw.outlineThickness, 1, 4, DEFAULT_CROSSHAIR_PROFILE.outlineThickness),
    centerDot: raw.centerDot === true,
    centerDotOpacity: clampNumber(raw.centerDotOpacity, 0, 1, DEFAULT_CROSSHAIR_PROFILE.centerDotOpacity),
    centerDotThickness: clampNumber(raw.centerDotThickness, 1, 4, DEFAULT_CROSSHAIR_PROFILE.centerDotThickness),
    innerLines: raw.innerLines !== false,
    innerLinesOpacity: clampNumber(raw.innerLinesOpacity, 0, 1, DEFAULT_CROSSHAIR_PROFILE.innerLinesOpacity),
    innerLinesLength: clampNumber(raw.innerLinesLength, 1, 12, DEFAULT_CROSSHAIR_PROFILE.innerLinesLength),
    innerLinesThickness: clampNumber(raw.innerLinesThickness, 1, 6, DEFAULT_CROSSHAIR_PROFILE.innerLinesThickness),
    innerLinesOffset: clampNumber(raw.innerLinesOffset, 0, 12, DEFAULT_CROSSHAIR_PROFILE.innerLinesOffset),
    outerLines: raw.outerLines === true,
    outerLinesOpacity: clampNumber(raw.outerLinesOpacity, 0, 1, DEFAULT_CROSSHAIR_PROFILE.outerLinesOpacity),
    outerLinesLength: clampNumber(raw.outerLinesLength, 1, 12, DEFAULT_CROSSHAIR_PROFILE.outerLinesLength),
    outerLinesThickness: clampNumber(raw.outerLinesThickness, 1, 6, DEFAULT_CROSSHAIR_PROFILE.outerLinesThickness),
    outerLinesOffset: clampNumber(raw.outerLinesOffset, 0, 16, DEFAULT_CROSSHAIR_PROFILE.outerLinesOffset),
    firingError: raw.firingError === true,
    movementError: raw.movementError === true,
  };
}

export function normalizeCrosshairProfiles(value) {
  const raw = value && typeof value === "object" ? value : {};
  const source = raw.profiles && typeof raw.profiles === "object" ? raw.profiles : raw;
  return PROFILE_KEYS.reduce((profiles, key) => {
    profiles[key] = normalizeCrosshairProfile(source[key] || source[key.toLowerCase()] || (key === "P" ? source.profile : null));
    return profiles;
  }, {});
}

const COLOR_IDS = Object.freeze({ white: 0, green: 1, yellowgreen: 2, greenyellow: 3, yellow: 4, cyan: 5, pink: 6, red: 7 });
const COLOR_NAMES = Object.freeze(Object.fromEntries(Object.entries(COLOR_IDS).map(([name, id]) => [String(id), name])));

function nativeNumber(value, fallback) {
  const number = Number(value);
  const safe = Number.isFinite(number) ? number : fallback;
  return String(Math.round(safe * 1000) / 1000);
}

function standardSection(name, rawProfile) {
  const profile = normalizeCrosshairProfile(rawProfile);
  return [
    name,
    "c", String(COLOR_IDS[profile.color] ?? 1),
    "h", profile.outlines ? "1" : "0",
    "o", nativeNumber(profile.outlineOpacity, 0.8),
    "t", nativeNumber(profile.outlineThickness, 1),
    "d", profile.centerDot ? "1" : "0",
    "a", nativeNumber(profile.centerDotOpacity, 1),
    "z", nativeNumber(profile.centerDotThickness, 2),
    "f", "0",
    "0b", profile.innerLines ? "1" : "0",
    "0a", nativeNumber(profile.innerLinesOpacity, 1),
    "0l", nativeNumber(profile.innerLinesLength, 4),
    "0t", nativeNumber(profile.innerLinesThickness, 2),
    "0o", nativeNumber(profile.innerLinesOffset, 2),
    "0m", profile.movementError ? "1" : "0",
    "0f", profile.firingError ? "1" : "0",
    "1b", profile.outerLines ? "1" : "0",
    "1a", nativeNumber(profile.outerLinesOpacity, 0.5),
    "1l", nativeNumber(profile.outerLinesLength, 2),
    "1t", nativeNumber(profile.outerLinesThickness, 2),
    "1o", nativeNumber(profile.outerLinesOffset, 3),
    "1m", profile.movementError ? "1" : "0",
    "1f", profile.firingError ? "1" : "0",
  ];
}

function sniperSection(rawProfile) {
  const profile = normalizeCrosshairProfile(rawProfile);
  return [
    "S",
    "c", String(COLOR_IDS[profile.color] ?? 1),
    "d", profile.centerDot ? "1" : "0",
    "o", nativeNumber(profile.centerDotOpacity, 1),
    "s", nativeNumber(profile.centerDotThickness, 2),
  ];
}

export function serializeCrosshairCode(profiles) {
  const normalized = normalizeCrosshairProfiles(profiles);
  return ["0", "s", "1", ...standardSection("P", normalized.P), ...standardSection("A", normalized.A), ...sniperSection(normalized.S)].join(";");
}

export function parseCrosshairCode(value) {
  const text = String(value || "").trim();
  if (!text) throw new Error("empty-crosshair");
  const tokens = text.split(";").map((token) => token.trim());
  if (tokens[0] !== "0") throw new Error("invalid-crosshair-version");
  const sections = { start: {}, P: {}, A: {}, S: {} };
  let section = "start";
  for (let index = 1; index < tokens.length;) {
    const token = tokens[index];
    if (["P", "A", "S"].includes(token)) {
      section = token;
      index += 1;
      continue;
    }
    if (!token || index + 1 >= tokens.length || !tokens[index + 1]) throw new Error("invalid-crosshair-token");
    sections[section][token] = tokens[index + 1];
    index += 2;
  }
  if (!Object.keys(sections.P).length) throw new Error("missing-primary-crosshair");

  const number = (raw, fallback) => {
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : fallback;
  };
  const standard = (fields) => normalizeCrosshairProfile({
    color: COLOR_NAMES[fields.c] || "white",
    outlines: fields.h == null ? true : fields.h === "1",
    outlineOpacity: number(fields.o, 0.5),
    outlineThickness: number(fields.t, 1),
    centerDot: fields.d === "1",
    centerDotOpacity: number(fields.a, 1),
    centerDotThickness: number(fields.z, 2),
    innerLines: fields["0b"] == null ? true : fields["0b"] === "1",
    innerLinesOpacity: number(fields["0a"], 0.8),
    innerLinesLength: number(fields["0l"], 6),
    innerLinesThickness: number(fields["0t"], 2),
    innerLinesOffset: number(fields["0o"], 3),
    outerLines: fields["1b"] == null ? true : fields["1b"] === "1",
    outerLinesOpacity: number(fields["1a"], 0.35),
    outerLinesLength: number(fields["1l"], 2),
    outerLinesThickness: number(fields["1t"], 2),
    outerLinesOffset: number(fields["1o"], 10),
    movementError: fields["0m"] === "1" || fields["1m"] === "1",
    firingError: fields["0f"] === "1" || fields["1f"] === "1",
  });
  const sniper = normalizeCrosshairProfile({
    color: COLOR_NAMES[sections.S.c] || "white",
    centerDot: sections.S.d == null ? true : sections.S.d === "1",
    centerDotOpacity: number(sections.S.o, 1),
    centerDotThickness: number(sections.S.s, 2),
    innerLines: false,
    outerLines: false,
  });
  return { P: standard(sections.P), A: standard(sections.A), S: sniper };
}

export function getResolutionLabel(selection, custom) {
  const resolution = resolutionFromSelection(selection, custom);
  return resolution.width + "×" + resolution.height;
}
