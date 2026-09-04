import json
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
TAURI_ROOT = REPO_ROOT / "frontend" / "src-tauri"
TAURI_BUILD_SCRIPT = REPO_ROOT / "frontend" / "scripts" / "tauri-build-version.mjs"
TAURI_RUNTIME = TAURI_ROOT / "src" / "lib.rs"
LEAGUE_ADMIN_LAUNCHER = TAURI_ROOT / "src" / "league_admin_launcher.ps1"


def test_tauri_identifier_and_installer_hook_are_stable():
    config = json.loads((TAURI_ROOT / "tauri.conf.json").read_text(encoding="utf-8"))

    assert config["productName"] == "MaxGameStudio"
    assert config["identifier"] == "com.cs2insightagent.app"
    assert config["build"]["beforeBuildCommand"] == "node node_modules/vite/bin/vite.js build"
    hook = config["bundle"]["windows"]["nsis"]["installerHooks"]
    assert hook == "windows/upgrade-hooks.nsh"
    assert (TAURI_ROOT / hook).is_file()


def test_installer_hook_covers_electron_upgrade_surfaces():
    hook = (TAURI_ROOT / "windows" / "upgrade-hooks.nsh").read_text(encoding="utf-8")

    assert 'tasklist.exe" /FI "IMAGENAME eq $R9"' in hook
    assert 'StrCpy $R9 "CS2 Insight Agent.exe"' in hook
    assert 'StrCpy $R9 "cs2-insight-agent-desktop.exe"' in hook
    assert 'StrCpy $R9 "MaxGameStudioLeague.exe"' in hook
    league_guard = hook.index('StrCpy $R9 "MaxGameStudioLeague.exe"')
    shell_guard = hook.index('StrCpy $R9 "cs2-insight-agent-desktop.exe"')
    assert league_guard < shell_guard
    assert "请先正常关闭英雄联盟工作台" in hook[league_guard:shell_guard]
    assert "Call CS2_AbortMigrationInstall" in hook[league_guard:shell_guard]
    # A running Tauri shell is waited for and force-killed with its backend
    # child tree instead of aborting the install.
    assert 'taskkill.exe" /IM "cs2-insight-agent-desktop.exe" /F /T' in hook
    # An orphaned backend must not keep port 19871 busy after an upgrade.
    assert "LocalPort 19871" in hook
    # Same-directory Electron installs are retired before file copy; a
    # different-directory copy remains as recovery until first-launch storage
    # migration verifies the new root.
    assert 'StrCpy $CS2ElectronScope "samedir"' in hook
    postinstall = hook[hook.index("!macro NSIS_HOOK_POSTINSTALL"):]
    assert 'StrCpy $CS2ElectronScope "all"' not in postinstall
    assert "Call CS2_RemoveLegacyElectron" not in postinstall
    assert "EnumRegKey $R5 HKCU" in hook
    assert "EnumRegKey $R5 HKLM" in hook
    assert "SetRegView 64" in hook
    assert "SetRegView 32" in hook
    assert "uninstall cs2 insight agent.exe" in hook.lower()
    assert "ExecWait '$R8 /S'" in hook
    assert "desktop_data_migration.py" not in hook
    assert "--require-desktop-stopped" not in hook
    assert "--require-electron-ui-export" not in hook
    # In-place upgrades must remove every historical patched-parser metadata
    # generation before copying the new runtime, not a hard-coded version list.
    assert 'FindFirst $0 $1 "$INSTDIR\\python\\Lib\\site-packages\\demoparser2-*.dist-info"' in hook
    assert 'RMDir /r "$INSTDIR\\python\\Lib\\site-packages\\demoparser2"' in hook
    assert "Call CS2_RemoveBundledDemoparser" in hook
    assert 'Delete "$INSTDIR\\league-runtime\\.maxgamestudio-acl-hardened"' in hook
    assert "cs2_stale_league_acl_marker" in hook
    assert 'Delete "$DESKTOP\\CS2 Ultimate Insight Studio.lnk"' in hook
    assert 'Delete "$SMPROGRAMS\\CS2 Ultimate Insight Studio.lnk"' in hook
    assert "Call CS2_RemoveLegacyBrandShortcuts" in hook
    assert "demoparser2-0.41.4+cs2insight1.dist-info" not in hook
    # The installed parser contract is checked before the app can be launched.
    assert 'backend\\app\\demoparser_runtime.py' in hook
    assert hook.count("Call CS2_ValidateBundledRuntime") == 2
    final_validation = hook.rindex("Call CS2_ValidateBundledRuntime")
    assert final_validation > hook.index("Call CS2_RemoveLegacyTauri", hook.index("!macro NSIS_HOOK_POSTINSTALL"))
    assert final_validation > hook.index("Call CS2_RemoveLegacyBrandShortcuts", hook.index("!macro NSIS_HOOK_POSTINSTALL"))
    assert "pyarrow-25.0.0.dist-info" in hook
    assert '!define CS2_TAURI_RELEASE_DIR "${__FILEDIR__}\\..\\target\\release"' in hook
    assert 'File /a "/oname=WebView2Loader.dll"' in hook
    assert 'Delete "$INSTDIR\\WebView2Loader.dll"' in hook
    assert "NSIS_HOOK_PREINSTALL" in hook
    assert "NSIS_HOOK_POSTINSTALL" in hook
    assert "NSIS_HOOK_POSTUNINSTALL" in hook


def test_versioned_build_rejects_missing_webview2_loader_bundle():
    script = TAURI_BUILD_SCRIPT.read_text(encoding="utf-8")

    assert "dirname(process.execPath)" in script
    assert "WebView2Loader.dll" in script
    assert "GNU Tauri build is missing required runtime loader" in script
    assert "NSIS hook does not install WebView2Loader.dll" in script
    assert "validated Windows runtime bundle" in script
    assert "createUpdaterArtifacts: false" in script
    assert "updater private key not found" in script
    assert "rmSync(updaterSignature)" in script
    assert '!define PRODUCTNAME "MaxGameStudio"' in script
    assert "does not create the branded desktop shortcut" in script


def test_league_administrator_launcher_uses_a_protected_same_drive_copy():
    script = LEAGUE_ADMIN_LAUNCHER.read_text(encoding="utf-8")
    hook = (TAURI_ROOT / "windows" / "upgrade-hooks.nsh").read_text(encoding="utf-8")

    assert "MaxGameStudioAdminRuntime" in script
    assert "New-ProtectedDirectory $sessionRoot" in script
    assert "The runtime manifest does not match the signed host" in script
    assert "[IO.FileShare]::Read" in script
    assert "NODE|ELECTRON" in script
    assert "Invoke-Expression" not in script
    assert "HardenLeagueRuntimeAcl" not in hook


def test_close_destroys_webview_before_waiting_for_backend():
    source = TAURI_RUNTIME.read_text(encoding="utf-8")

    destroy = source.index("window.destroy()")
    stop = source.index("stop_backend(&handle);", destroy)
    assert destroy < stop
    # The graceful backend stop must run off the event loop thread so the
    # window disappears immediately instead of freezing on screen.
    spawn = source.index("thread::spawn", destroy)
    assert spawn < stop
