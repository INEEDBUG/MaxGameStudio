export function baseLocale(locale = "zh") {
  return String(locale).toLowerCase().startsWith("zh") ? "zh" : "en";
}

export function intlLocale(locale = "zh") {
  const value = String(locale || "zh");
  if (value === "zh") return "zh-CN";
  return value;
}
