import json
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[2]
HOOK_PATH = REPO_ROOT / "frontend" / "src-tauri" / "windows" / "upgrade-hooks.nsh"
TAURI_CONFIG_PATH = REPO_ROOT / "frontend" / "src-tauri" / "tauri.conf.json"

LEGACY_PRODUCT = "CS2 Ultimate Insight Studio"
LEGACY_UNINSTALL_KEY = "CS2 Ultimate Insight Studio"
LEGACY_INSTALL_LOCATION = r"D:\CS2-Ultimate-Insight-Studio"
LEGACY_EXE = "cs2-insight-agent-desktop.exe"
CURRENT_PRODUCT = "MaxGameStudio"


@pytest.fixture(scope="module")
def hook() -> str:
    return HOOK_PATH.read_text(encoding="utf-8")


def _function_body(hook: str, name: str) -> str:
    start = hook.index(f"Function {name}")
    end = hook.index("FunctionEnd", start)
    return hook[start:end]


def _macro_body(hook: str, name: str) -> str:
    start = hook.index(f"!macro {name}")
    end = hook.index("!macroend", start)
    return hook[start:end]


def _matches_exact_legacy_entry(key: str, display_name: str) -> bool:
    return (
        key == LEGACY_UNINSTALL_KEY
        and display_name == LEGACY_PRODUCT
        and display_name != CURRENT_PRODUCT
    )


def test_registry_fixture_matches_only_the_known_legacy_tauri_entry(hook: str):
    assert LEGACY_INSTALL_LOCATION == r"D:\CS2-Ultimate-Insight-Studio"
    assert LEGACY_EXE == "cs2-insight-agent-desktop.exe"
    assert '!define CS2_LEGACY_TAURI_PRODUCT "CS2 Ultimate Insight Studio"' in hook
    assert '!define CS2_LEGACY_TAURI_UNINSTALL_KEY "CS2 Ultimate Insight Studio"' in hook
    assert '!define CS2_CURRENT_TAURI_PRODUCT "MaxGameStudio"' in hook

    assert _matches_exact_legacy_entry(LEGACY_UNINSTALL_KEY, LEGACY_PRODUCT)
    assert not _matches_exact_legacy_entry("MaxGameStudio", CURRENT_PRODUCT)
    assert not _matches_exact_legacy_entry(LEGACY_UNINSTALL_KEY, "CS2 Ultimate Insight Studio Preview")
    assert not _matches_exact_legacy_entry("CS2 Ultimate Insight Studio Preview", LEGACY_PRODUCT)

    for hive, function_name in (
        ("HKCU", "CS2_RemoveLegacyTauriHKCU"),
        ("HKLM", "CS2_RemoveLegacyTauriHKLM"),
    ):
        body = _function_body(hook, function_name)
        assert (
            f'ReadRegStr $R0 {hive} '
            '"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\$R5" '
            '"DisplayName"'
        ) in body
        assert '${If} $R0 == "${CS2_CURRENT_TAURI_PRODUCT}"' .replace("§", "$") in body
        assert '${If} $R0 != "${CS2_LEGACY_TAURI_PRODUCT}"' .replace("§", "$") in body
        assert '${If} $R5 != "${CS2_LEGACY_TAURI_UNINSTALL_KEY}"' .replace("§", "$") in body


def test_same_directory_is_preinstall_only_and_other_directories_are_postinstall(hook: str):
    preinstall = _macro_body(hook, "NSIS_HOOK_PREINSTALL")
    postinstall = _macro_body(hook, "NSIS_HOOK_POSTINSTALL")

    same_scope = 'StrCpy $CS2LegacyTauriScope "samedir"'
    different_scope = 'StrCpy $CS2LegacyTauriScope "differentdir"'
    assert same_scope in preinstall
    assert preinstall.index(same_scope) < preinstall.index("Call CS2_RemoveLegacyTauri")
    assert preinstall.index("Call CS2_RemoveLegacyTauri") < preinstall.index("SetOutPath $INSTDIR")

    runtime_check = postinstall.index("Call CS2_ValidateBundledRuntime")
    data_migration = postinstall.index("desktop_data_migration.py")
    post_cleanup = postinstall.index(different_scope)
    assert runtime_check < data_migration < post_cleanup
    assert postinstall.index("Call CS2_RemoveLegacyTauri", post_cleanup) > post_cleanup

    # If a same-directory legacy registry entry survives PREINSTALL, the
    # postinstall path must fail closed rather than uninstalling $INSTDIR.
    for function_name in ("CS2_RemoveLegacyTauriHKCU", "CS2_RemoveLegacyTauriHKLM"):
        body = _function_body(hook, function_name)
        different_branch = body.index('${ElseIf} $CS2LegacyTauriScope == "differentdir"'.replace("§", "$"))
        assert "仍指向当前安装目录" in body[different_branch:]
        assert "Call CS2_AbortMigrationInstall" in body[different_branch:]


def test_runtime_and_data_failure_abort_before_legacy_tauri_retirement(hook: str):
    postinstall = _macro_body(hook, "NSIS_HOOK_POSTINSTALL")
    validator = _function_body(hook, "CS2_ValidateBundledRuntime")
    assert "demoparser_runtime.py" in validator
    assert "ExecWait" in validator
    assert "Call CS2_AbortMigrationInstall" in validator
    assert postinstall.index("Call CS2_ValidateBundledRuntime") < postinstall.index(
        "Call CS2_RemoveLegacyTauri"
    )
    assert postinstall.index("desktop_data_migration.py") < postinstall.index(
        "Call CS2_RemoveLegacyTauri"
    )
    assert "Call CS2_AbortMigrationInstall" in postinstall


def test_legacy_shortcut_cleanup_is_exact_and_non_recursive(hook: str):
    body = _function_body(hook, "CS2_RemoveLegacyBrandShortcuts")
    expected_paths = (
        '$DESKTOP\\CS2 Ultimate Insight Studio.lnk',
        '$SMPROGRAMS\\CS2 Ultimate Insight Studio.lnk',
        '$SMPROGRAMS\\CS2 Ultimate Insight Studio\\CS2 Ultimate Insight Studio.lnk',
    )
    for path in expected_paths:
        assert f'Delete "{path}"' in body

    assert 'RMDir "$SMPROGRAMS\\CS2 Ultimate Insight Studio"' in body
    assert "RMDir /r" not in body
    assert "*" not in body
    assert '$DESKTOP\\MaxGameStudio.lnk' not in body
    assert '$SMPROGRAMS\\MaxGameStudio.lnk' not in body
    postinstall = _macro_body(hook, "NSIS_HOOK_POSTINSTALL")
    assert postinstall.index("Call CS2_RemoveLegacyBrandShortcuts") > postinstall.index(
        "Call CS2_RemoveLegacyTauri"
    )


def test_legacy_uninstaller_is_silent_and_never_receives_data_delete_switch(hook: str):
    body = _function_body(hook, "CS2_RunLegacyTauriUninstaller")
    assert "ExecWait '\"$CS2LegacyTauriUninsExe\" /S'" in body
    assert "DELETEAPPDATA" not in body.upper()
    assert "RMDir /r" not in body
    assert "APPDATA" in body


def test_missing_legacy_tauri_uninstaller_retires_only_the_exact_stale_registry_entry(hook: str):
    for hive, function_name, marker in (
        ("HKCU", "CS2_RemoveLegacyTauriHKCU", "cs2_legacy_tauri_hkcu_missing_uninstaller"),
        ("HKLM", "CS2_RemoveLegacyTauriHKLM", "cs2_legacy_tauri_hklm_missing_uninstaller"),
    ):
        body = _function_body(hook, function_name)
        branch = body.index(marker)
        tail = body[branch:]
        scope_check = body.index('${If} $CS2LegacyTauriScope == "samedir"')
        assert 'StrCpy $R9 "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\$R5"' in body
        assert f"DeleteRegKey {hive} \"$R9\"" in tail
        assert f'ReadRegStr $R3 {hive} "$R9" "DisplayName"' in tail
        assert scope_check < branch
        clear_errors = tail.index("ClearErrors")
        delete_key = tail.index(f'DeleteRegKey {hive} "$R9"')
        error_guard = tail.index("${If} ${Errors}")
        assert clear_errors < delete_key < error_guard
        # The missing-file path must not invoke the absent old uninstaller.
        assert "Call CS2_RunLegacyTauriUninstaller" not in tail.split("Goto cs2_legacy_tauri_hk", 1)[0]
        assert "Goto cs2_legacy_tauri_hk" in tail


def test_tauri_identity_and_update_channel_remain_unchanged():
    config = json.loads(TAURI_CONFIG_PATH.read_text(encoding="utf-8"))
    updater = config["plugins"]["updater"]

    assert config["identifier"] == "com.cs2insightagent.app"
    assert updater["pubkey"] == (
        "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDNBMjcwNkFBNUIwNEJGNDgKUldSSXZ3UmJxZ1luT25TSzY2Y2M3WGhDWlpVQWVScWdKbWhqOFlGL0p5QktEVEFScERlRVRmYWIK"
    )
    assert updater["endpoints"] == [
        "https://raw.githubusercontent.com/INEEDBUG/MaxGameStudio/updater/latest.json"
    ]
