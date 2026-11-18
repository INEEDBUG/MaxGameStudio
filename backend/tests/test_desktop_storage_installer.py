import json
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
HOOK_PATH = REPO_ROOT / "frontend" / "src-tauri" / "windows" / "upgrade-hooks.nsh"


def _macro_body(hook: str, name: str) -> str:
    start = hook.index(f"!macro {name}")
    end = hook.index("!macroend", start)
    return hook[start:end]


def test_installer_leaves_user_data_to_the_native_storage_bootstrap():
    hook = HOOK_PATH.read_text(encoding="utf-8")
    postinstall = _macro_body(hook, "NSIS_HOOK_POSTINSTALL")

    assert "desktop_data_migration.py" not in postinstall
    assert "--appdata" not in postinstall
    assert "MGS_RecordDataDirectorySelection" in postinstall
    assert "--destination" not in postinstall
    native = (REPO_ROOT / "frontend/src-tauri/src/desktop_storage.rs").read_text(encoding="utf-8")
    assert 'registry_string("InstallerDataRootV1")' in native
    assert '"switch-only-v2"' in native


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
    assert body.index('GetCommandLineW') < body.index('${GetOptions} $1 "/D="') < body.index("GetDriveTypeW")
    assert 'StrCpy $3 $WINDIR 2' in body
    assert 'IntCmp $0 3' in body
    assert '首次静默安装需要通过 /D= 指定程序目录' in hook


def test_chinese_data_page_is_after_program_directory_and_before_file_copy():
    tauri = REPO_ROOT / "frontend/src-tauri"
    nsis = json.loads((tauri / "tauri.conf.json").read_text(encoding="utf-8"))["bundle"]["windows"]["nsis"]
    template = (tauri / nsis["template"]).read_text(encoding="utf-8")
    assert nsis["languages"] == ["SimpChinese"]
    assert nsis["displayLanguageSelector"] is False
    assert template.index("!insertmacro MUI_PAGE_DIRECTORY") < template.index("Page custom MGS_ShowDataDirectory")
    assert template.index("Page custom MGS_ShowDataDirectory") < template.index("!insertmacro MUI_PAGE_INSTFILES")
