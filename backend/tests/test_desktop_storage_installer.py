from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
HOOK_PATH = REPO_ROOT / "frontend" / "src-tauri" / "windows" / "upgrade-hooks.nsh"


def _macro_body(hook: str, name: str) -> str:
    start = hook.index(f"!macro {name}")
    end = hook.index("!macroend", start)
    return hook[start:end]


def test_installer_defers_storage_migration_to_first_launch_bootstrap():
    hook = HOOK_PATH.read_text(encoding="utf-8")
    postinstall = _macro_body(hook, "NSIS_HOOK_POSTINSTALL")

    assert "desktop_data_migration.py" not in postinstall
    assert "--appdata" not in postinstall
    assert "first-launch native storage bootstrap" in postinstall
    assert "canonical non-system storage root" in postinstall


def test_installer_keeps_different_directory_electron_recovery_copy():
    hook = HOOK_PATH.read_text(encoding="utf-8")
    postinstall = _macro_body(hook, "NSIS_HOOK_POSTINSTALL")

    assert 'StrCpy $CS2ElectronScope "all"' not in postinstall
    assert "Call CS2_RemoveLegacyElectron" not in postinstall
    assert "recoverable fallback" in postinstall
    assert 'StrCpy $CS2LegacyTauriScope "differentdir"' in postinstall
    assert "Call CS2_RemoveLegacyTauri" in postinstall


def test_fresh_install_default_preserves_explicit_and_registered_paths():
    hook = HOOK_PATH.read_text(encoding="utf-8")

    body = hook.split("Function MGS_SelectFreshInstallDrive", 1)[1].split("FunctionEnd", 1)[0]
    assert "MUI_CUSTOMFUNCTION_GUIINIT" in hook
    assert body.index("ReadRegStr") < body.index("GetDriveTypeW")
    assert body.index('${GetOptions} $CMDLINE "/D="') < body.index("GetDriveTypeW")
    assert 'StrCpy $3 $WINDIR 2' in body
    assert 'IntCmp $0 3' in body
    assert 'Fresh silent installations require an explicit /D=' in hook
