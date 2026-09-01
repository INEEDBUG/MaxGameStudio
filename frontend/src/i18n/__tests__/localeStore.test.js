import { describe, test, expect, beforeEach, vi } from "vitest";

// 持久化改为后端 config（PUT /api/config），不再用 localStorage。Mock API 避免真实网络。
const putMock = vi.fn(() => Promise.resolve({ data: {} }));
vi.mock("../../api/api", () => ({ default: { put: (...a) => putMock(...a) } }));

import { resolveEffectiveLocale, useLocaleStore } from "../localeStore.js";

describe("localeStore", () => {
  beforeEach(() => {
    putMock.mockClear();
    useLocaleStore.getState().hydrate("zh");
  });

  test("默认 locale 为 zh", () => {
    expect(useLocaleStore.getState().locale).toBe("zh");
  });

  test("hydrate 从配置注入但不回写后端", () => {
    useLocaleStore.getState().hydrate("en");
    expect(useLocaleStore.getState().locale).toBe("en");
    expect(putMock).not.toHaveBeenCalled();
  });

  test("hydrate 非法值回退到 auto 并解析系统语言", () => {
    useLocaleStore.getState().hydrate("fr");
    expect(useLocaleStore.getState().locale).toBe("auto");
    expect(["zh", "en"]).toContain(useLocaleStore.getState().effectiveLocale);
  });

  test("setLocale 更新 state 并持久化到 config", () => {
    useLocaleStore.getState().setLocale("en");
    expect(useLocaleStore.getState().locale).toBe("en");
    expect(putMock).toHaveBeenCalledWith("config", { locale: "en" });
  });

  test("setLocale 非法值回退到 auto", () => {
    useLocaleStore.getState().setLocale("fr");
    expect(useLocaleStore.getState().locale).toBe("auto");
    expect(putMock).toHaveBeenCalledWith("config", { locale: "auto" });
  });

  test.each(["zh-HK", "zh-TW", "ms-MY", "ru-RU"])("支持区域 locale %s", (locale) => {
    useLocaleStore.getState().setLocale(locale);
    expect(useLocaleStore.getState().locale).toBe(locale);
    expect(useLocaleStore.getState().effectiveLocale).toBe(locale);
    expect(putMock).toHaveBeenCalledWith("config", { locale });
  });

  test.each([
    ["zh-HK", "zh-HK"], ["zh-TW", "zh-TW"], ["ms-MY", "ms-MY"], ["ru-RU", "ru-RU"], ["fr-FR", "en"],
  ])("auto 根据系统语言映射 %s", (language, expected) => {
    expect(resolveEffectiveLocale("auto", language)).toBe(expected);
  });
});
