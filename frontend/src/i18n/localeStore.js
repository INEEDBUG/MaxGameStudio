import { create } from "zustand";
import API from "../api/api";

export const SUPPORTED_LOCALES = ["auto", "zh", "zh-HK", "zh-TW", "en", "ms-MY", "ru-RU"];
export const LOCALE_LABEL_KEYS = {
  auto: "settings.localeAuto",
  zh: "settings.localeZh",
  "zh-HK": "settings.localeZhHk",
  "zh-TW": "settings.localeZhTw",
  en: "settings.localeEn",
  "ms-MY": "settings.localeMsMy",
  "ru-RU": "settings.localeRuRu",
};
const DEFAULT_LOCALE = "auto";

// 解析 "auto" 为实际区域语言代码
export function resolveEffectiveLocale(locale, language = null) {
  if (locale === "auto") {
    const browserLang = String(language ?? navigator.language ?? navigator.userLanguage ?? "").toLowerCase();
    if (browserLang.startsWith("zh-hk") || browserLang.startsWith("zh-mo")) return "zh-HK";
    if (browserLang.startsWith("zh-tw")) return "zh-TW";
    if (browserLang.startsWith("zh")) return "zh";
    if (browserLang.startsWith("ms")) return "ms-MY";
    if (browserLang.startsWith("ru")) return "ru-RU";
    return "en";
  }
  return locale;
}

// 验证配置值是否合法
function normalizeConfig(next) {
  return SUPPORTED_LOCALES.includes(next) ? next : DEFAULT_LOCALE;
}

// 验证实际语言代码是否合法
function normalizeEffective(next) {
  const resolved = resolveEffectiveLocale(next);
  return SUPPORTED_LOCALES.includes(resolved) && resolved !== "auto" ? resolved : "zh";
}

export const useLocaleStore = create((set, get) => ({
  locale: DEFAULT_LOCALE, // 配置值（可能是 "auto"）
  effectiveLocale: normalizeEffective(DEFAULT_LOCALE), // 实际使用的语言（zh/en）

  // 从后端配置注入（GET /api/config 拉取后调用）：只更新内存，不回写后端
  hydrate: (next) => {
    const locale = normalizeConfig(next);
    const effectiveLocale = normalizeEffective(locale);
    set({ locale, effectiveLocale });
  },

  // 用户主动切换：立即更新 UI，并持久化到 cs2-insight.config.json（PUT /api/config）
  setLocale: (next) => {
    const locale = normalizeConfig(next);
    const effectiveLocale = normalizeEffective(locale);
    set({ locale, effectiveLocale });
    API.put("config", { locale }).catch((e) => {
      if (import.meta.env?.DEV) {
        console.warn("[i18n] persist locale failed:", e);
      }
    });
  },
}));
