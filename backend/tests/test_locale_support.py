from app.env_utils import resolve_effective_locale, resolve_system_locale


def test_effective_locale_accepts_regional_locales():
    for locale in ("zh", "zh-HK", "zh-TW", "en", "ms-MY", "ru-RU"):
        assert resolve_effective_locale(locale) == locale


def test_effective_locale_invalid_value_falls_back_to_chinese():
    assert resolve_effective_locale("fr-FR") == "zh"


def test_system_locale_maps_supported_language_tags(monkeypatch):
    import locale as locale_module

    for language, expected in (
        ("zh_HK", "zh-HK"),
        ("zh_TW", "zh-TW"),
        ("zh_CN", "zh"),
        ("ms_MY", "ms-MY"),
        ("ru_RU", "ru-RU"),
        ("fr_FR", "en"),
    ):
        monkeypatch.setattr(locale_module, "getdefaultlocale", lambda: (language, "UTF-8"))
        assert resolve_system_locale() == expected
