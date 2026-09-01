import { describe, test, expect } from "vitest";
import zh from "../dict/zh.js";
import en from "../dict/en.js";
import { DICTS, translate } from "../useT.js";
import { msMY, ruRU, zhHK, zhTW } from "../dict/regional.js";

const overlays = { "zh-HK": zhHK, "zh-TW": zhTW, "ms-MY": msMY, "ru-RU": ruRU };
const coreKeys = Object.keys(en).filter((key) =>
  /^(common|nav|update|league)\./.test(key) ||
  /^(settings\.(pageTitle|pageSubtitle|btnUpdate|updateChecking|saveBtn|saveAllBtn|saveFooterDesc|sectionLanguage|locale))/.test(key),
);

describe("dict consistency", () => {
  test("en and zh have the same complete base key set", () => {
    expect(Object.keys(zh).filter((key) => !key.startsWith("__test_only_") && !(key in en))).toEqual([]);
    expect(Object.keys(en).filter((key) => !(key in zh))).toEqual([]);
  });

  test("regional dictionaries are sparse overlays of their base dictionaries", () => {
    for (const [locale, overlay] of Object.entries(overlays)) {
      const base = locale.startsWith("zh") ? zh : en;
      expect(Object.keys(overlay).length, `${locale} overlay grew beyond the approved core`).toBeLessThanOrEqual(100);
      const invalid = Object.keys(overlay).filter((key) => !(key in base));
      expect(invalid, `${locale} contains keys absent from its base`).toEqual([]);
    }
  });

  test("core user-visible keys are covered", () => {
    for (const [locale, overlay] of Object.entries(overlays)) {
      const missing = coreKeys.filter((key) => !(key in overlay));
      expect(missing, `${locale} missing required core keys`).toEqual([]);
    }
  });

  test("core copy has no known mechanical concatenations", () => {
    const forbidden = /Automatikmation|Выкл\.icial|Игрок Center|繁體Китайский|Light from|active$/;
    for (const [locale, overlay] of Object.entries(overlays)) {
      const polluted = coreKeys.filter((key) => forbidden.test(String(overlay[key] || "")));
      expect(polluted, `${locale} contains mechanical copy`).toEqual([]);
    }
  });

  test("deep namespaces fall back to the locale base", () => {
    expect(translate("zh-HK", "library.colScore")).toBe(zh["library.colScore"]);
    expect(translate("zh-TW", "library.colScore")).toBe(zh["library.colScore"]);
    expect(translate("ms-MY", "library.colScore")).toBe(en["library.colScore"]);
    expect(translate("ru-RU", "library.colScore")).toBe(en["library.colScore"]);
  });
});
