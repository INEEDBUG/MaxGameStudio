import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, writeFileSync, mkdirSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';

// Native NSIS behavior, not a JavaScript reimplementation. Production registry
// literals are remapped to an isolated disposable key before compilation.
const compiler = process.env.MGS_TEST_MAKENSIS;
const hook = readFileSync(new URL('../../../frontend/src-tauri/windows/upgrade-hooks.nsh', import.meta.url), 'utf8');

test('scope selection resolves a matching machine update and rejects conflicting scope', { skip: !compiler || process.platform !== 'win32' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'mgs-installer-scope-'));
  const script = join(root, 'scope.nsi');
  const probe = join(root, 'scope.exe');
  const output = join(root, 'scope.txt');
  const settings = readFileSync(new URL('../../../frontend/src-tauri/windows/install-scope.nsh', import.meta.url), 'utf8');
  const resolver = settings.slice(settings.indexOf('Function MGS_ResolveCommandScope'), settings.indexOf('Function MGS_ApplyScope'));
  const user = join(root, 'User Program');
  const machine = join(root, 'Machine Program');
  mkdirSync(user);
  mkdirSync(machine);
  writeFileSync(script, `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${probe}"
!include LogicLib.nsh
!include FileFunc.nsh
Var MGSMachineProgramDir
Var MGSUserProgramDir
Var MGSMachineTarget
Var MGSMachineScope
Var MGSExplicitProgramDir
Var MGSScopeRequested
Var UpdateMode
${resolver}
Section
  StrCpy $MGSMachineTarget "${machine}"
  StrCpy $MGSMachineProgramDir "${machine}"
  StrCpy $MGSUserProgramDir "${user}"
  StrCpy $UpdateMode 1
  Call MGS_ResolveCommandScope
  FileOpen $9 "${output}" w
  FileWrite $9 "$MGSMachineScope"
  FileClose $9
  SetErrorLevel $R0
SectionEnd
`);
  const build = spawnSync(resolve(compiler), ['/V2', script], { encoding: 'utf8', windowsHide: true });
  assert.equal(build.status, 0, build.stdout + build.stderr);
  for (const [flags, target, exit, scope] of [
    [[], machine, 0, '1'],
    [[], user, 0, '0'],
    [['/ALLUSERS'], '', 0, '1'],
    [['/CURRENTUSER'], machine, 2, '0'],
    [['/ALLUSERS'], user, 2, '0'],
    [['/ALLUSERS', '/CURRENTUSER'], '', 2, '0'],
  ]) {
    const args = ['/S', ...flags];
    if (target) args.push(`/D=${target}`);
    const run = spawnSync(probe, args, { windowsHide: true, windowsVerbatimArguments: true, timeout: 15000 });
    assert.equal(run.status, exit, String(run.error || args.join(' ')));
    assert.equal(readFileSync(output, 'utf8'), scope, args.join(' '));
  }
});

test('canceling or failing elevation stops the machine install before payload writes', { skip: !compiler || process.platform !== 'win32' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'mgs-installer-consent-'));
  const scenario = readFileSync(new URL('../../../frontend/src-tauri/windows/install-scope.nsh', import.meta.url), 'utf8');
  const requestAccess = scenario.slice(scenario.indexOf('Function MGS_RequestMachineInstallAccess'), scenario.indexOf('Function MGS_FinalizeOriginalUser'));
  const script = join(root, 'consent.nsi');
  const probe = join(root, 'consent.exe');
  const message = join(root, 'message.txt');
  const marker = join(root, 'copied.txt');
  const source = `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${probe}"
!include LogicLib.nsh
!include WinMessages.nsh
!define UAC_IsAdmin "0 == 1"
!macro UAC_RunElevated
  ReadEnvStr $0 "MGS_UAC_RESULT"
!macroend
Var MGSMachineScope
Var MGSInnerInstaller
${requestAccess}
Function MGS_ReserveInstallationDirectories
  StrCpy $R0 0
FunctionEnd
Function MGS_CheckProgramAndDataDirectories
  StrCpy $R0 0
FunctionEnd
Function CS2_AbortMigrationInstall
  FileOpen $9 "${message}" w
  FileWriteUTF16LE $9 "$R7"
  FileClose $9
  SetErrorLevel 2
  Abort
FunctionEnd
Section
  StrCpy $MGSMachineScope 1
  Call MGS_RequestMachineInstallAccess
  FileOpen $9 "${marker}" w
  FileClose $9
SectionEnd
`;
  writeFileSync(script, '\ufeff' + source);
  const build = spawnSync(resolve(compiler), ['/V2', script], { encoding: 'utf8', windowsHide: true });
  assert.equal(build.status, 0, build.stdout + build.stderr);
  for (const [status, expected] of [['1223', '已取消管理员授权'], ['5', '错误 5']]) {
    const run = spawnSync(probe, ['/S'], {
      env: { ...process.env, MGS_UAC_RESULT: status },
      windowsHide: true,
      timeout: 15000,
    });
    assert.equal(run.status, 2, String(run.error || ''));
    assert.ok(readFileSync(message, 'utf16le').includes(expected));
    assert.equal(existsSync(marker), false);
  }
});

test('directly elevated machine setup preserves the personal data request', { skip: !compiler || process.platform !== 'win32' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'mgs-installer-admin-data-'));
  const key = `Software\\MaxGameStudioInstallerTests\\${randomUUID()}`;
  const scope = readFileSync(new URL('../../../frontend/src-tauri/windows/install-scope.nsh', import.meta.url), 'utf8');
  const finalizer = scope.slice(scope.indexOf('Function MGS_FinalizeOriginalUser'), scope.indexOf('; Tasklist'));
  const script = join(root, 'finalize.nsi');
  const probe = join(root, 'finalize.exe');
  const output = join(root, 'result.txt');
  const source = `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${probe}"
!include LogicLib.nsh
!define UAC_IsAdmin "1 == 1"
!define UNINSTKEY "${key}"
!define MANUPRODUCTKEY "${key}"
!define MAINBINARYNAME "no-installed-product"
Var UpdateMode
Var MGSUserProgramDir
Var MGSMachineTarget
${finalizer}
Function MGS_RecordDataDirectorySelection
  WriteRegStr HKCU "${key}" "InstallerDataRootV1" "would overwrite the approved request"
FunctionEnd
Section
  WriteRegStr HKCU "${key}" "InstallerDataRootV1" "prior request"
  Call MGS_FinalizeOriginalUser
  ReadRegStr $0 HKCU "${key}" "InstallerDataRootV1"
  DeleteRegKey HKCU "${key}"
  FileOpen $9 "${output}" w
  FileWrite $9 "$0"
  FileClose $9
SectionEnd
`;
  writeFileSync(script, '\ufeff' + source);
  const build = spawnSync(resolve(compiler), ['/V2', script], { encoding: 'utf8', windowsHide: true });
  assert.equal(build.status, 0, build.stdout + build.stderr);
  const run = spawnSync(probe, ['/S'], { windowsHide: true, timeout: 15000 });
  assert.equal(run.status, 0, String(run.error || ''));
  assert.equal(readFileSync(output, 'utf8'), 'prior request');
});

test('machine WebView2 ignores a user-only runtime and rejects an unsigned embedded bootstrapper', { skip: !compiler || process.platform !== 'win32' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'mgs-installer-webview-'));
  const key = `Software\\MaxGameStudioInstallerTests\\${randomUUID()}`;
  const dir = new URL('../../../frontend/src-tauri/windows/', import.meta.url);
  const template = readFileSync(new URL('installer.nsi', dir), 'utf8');
  const block = template.slice(template.indexOf('Function MGS_ReadWebView2Version'), template.indexOf('Section Install'))
    .replace(/HKLM "SOFTWARE\\(?:WOW6432Node\\)?Microsoft\\EdgeUpdate\\Clients\\\$\{WEBVIEW2APPGUID\}"/g, `HKCU "${key}\\Machine"`)
    .replaceAll('HKCU "SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2APPGUID}"', `HKCU "${key}\\User"`);
  const guard = readFileSync(new URL('directory-guard.nsh', dir), 'utf8');
  const findPowerShell = guard.slice(guard.indexOf('Function MGS_FindWindowsPowerShell'));
  const payload = join(root, 'not-microsoft.exe');
  const script = join(root, 'webview.nsi');
  const probe = join(root, 'webview.exe');
  const marker = join(root, 'copied.txt');
  const message = join(root, 'message.txt');
  writeFileSync(payload, 'unsigned inert fixture');
  const source = `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${probe}"
!include LogicLib.nsh
!include x64.nsh
!define MGS_WINDOWS_DIR "${fileURLToPath(dir)}"
!define MINIMUMWEBVIEW2VERSION ""
!define INSTALLWEBVIEW2MODE "embedBootstrapper"
!define WEBVIEW2BOOTSTRAPPERPATH "${payload}"
!define WEBVIEW2INSTALLERARGS "/silent"
Var MGSMachineScope
Var UpdateMode
LangString installingWebview2 1033 "fixture"
LangString webview2AbortError 1033 "fixture"
LangString webview2InstallSuccess 1033 "fixture"
${findPowerShell}
Function CS2_AbortMigrationInstall
  DeleteRegKey HKCU "${key}"
  FileOpen $9 "${message}" w
  FileWriteUTF16LE $9 "$R7"
  FileClose $9
  SetErrorLevel 2
  Abort
FunctionEnd
Section Setup
  ReadEnvStr $MGSMachineScope "MGS_WEBVIEW_MACHINE_SCOPE"
  WriteRegStr HKCU "${key}\\User" "pv" "100.0.0.0"
SectionEnd
${block}
Section Payload
  DeleteRegKey HKCU "${key}"
  FileOpen $9 "${marker}" w
  FileClose $9
SectionEnd
`;
  assert.ok(!block.includes('Microsoft\\EdgeUpdate\\Clients'));
  writeFileSync(script, '\ufeff' + source);
  const build = spawnSync(resolve(compiler), ['/V2', script], { encoding: 'utf8', windowsHide: true });
  assert.equal(build.status, 0, build.stdout + build.stderr);
  const runMachine = spawnSync(probe, ['/S'], {
    env: { ...process.env, MGS_WEBVIEW_MACHINE_SCOPE: '1' }, windowsHide: true, timeout: 25000,
  });
  assert.equal(runMachine.status, 2, String(runMachine.error || ''));
  assert.ok(readFileSync(message, 'utf16le').includes('无法验证内置的 Microsoft WebView2'));
  assert.equal(existsSync(marker), false);
  const runUser = spawnSync(probe, ['/S'], {
    env: { ...process.env, MGS_WEBVIEW_MACHINE_SCOPE: '0' }, windowsHide: true, timeout: 15000,
  });
  assert.equal(runUser.status, 0, String(runUser.error || ''));
  assert.equal(existsSync(marker), true);
});
for (const scenario of ['registered', 'uninstall-fallback', 'missing-registration', 'stale-registration', 'explicit', 'fresh-explicit', 'manual-overlay', 'manual-uninstall-fallback']) {
  test(`updater install path: ${scenario}`, { skip: !compiler || process.platform !== 'win32' }, () => {
    const root = mkdtempSync(join(tmpdir(), 'mgs-installer-path-'));
    const existing = join(root, 'Existing App');
    const wrong = join(root, 'Wrong Default');
    const explicit = join(root, 'Explicit App');
    mkdirSync(existing);
    writeFileSync(join(existing, 'cs2-insight-agent-desktop.exe'), 'fixture');
    const key = `Software\\MaxGameStudioInstallerTests\\${randomUUID()}`;
    const functions = hook.slice(hook.indexOf('Function MGS_'), hook.indexOf('Function CS2_AbortMigrationInstall'));
    const preinstall = hook.split('!macro NSIS_HOOK_PREINSTALL')[1].split('  Call CS2_PrepareRunningApps')[0];
    const registry = ['registered', 'explicit', 'manual-overlay'].includes(scenario)
      ? `WriteRegStr HKCU "${key}\\Product" "" "${existing}"`
      : scenario === 'stale-registration'
        ? `WriteRegStr HKCU "${key}\\Product" "" "${wrong}"`
        : scenario.endsWith('uninstall-fallback')
        ? `WriteRegStr HKCU "${key}\\Uninstall" "InstallLocation" '$\\"${existing}$\\"'`
        : '';
    const remap = (text) => text
      .replaceAll('Software\\cs2insightagent\\MaxGameStudio', `${key}\\Product`)
      .replaceAll('Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\MaxGameStudio', `${key}\\Uninstall`);
    const source = `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${join(root, 'probe.exe')}"
!include LogicLib.nsh
!include FileFunc.nsh
Var MGSExistingProgramDir
Var MGSVerifiedProgramDir
${remap(functions)}
Function CS2_AbortMigrationInstall
  DeleteRegKey HKCU "${key}"
  SetErrorLevel 2
  Quit
FunctionEnd
Section
  ${registry}
  StrCpy $INSTDIR "${scenario.includes('explicit') ? explicit : wrong}"
  ${scenario.startsWith('manual-') ? 'Call MGS_ApplyOverlayDirectory' : ''}
  ${remap(preinstall)}
  FileOpen $9 "${join(root, 'result.txt')}" w
  FileWrite $9 "$INSTDIR"
  FileClose $9
  DeleteRegKey HKCU "${key}"
SectionEnd
`;
    const file = join(root, 'probe.nsi');
    writeFileSync(file, '\ufeff' + source);
    const build = spawnSync(resolve(compiler), ['/V2', file], { encoding: 'utf8', windowsHide: true });
    assert.equal(build.status, 0, build.stdout + build.stderr);
    const args = scenario === 'fresh-explicit' || scenario.startsWith('manual-') ? ['/S'] : ['/S', '/UPDATE'];
    if (scenario.includes('explicit')) args.push(`/D=${explicit}`);
    const run = spawnSync(join(root, 'probe.exe'), args, { windowsHide: true, windowsVerbatimArguments: true, timeout: 15000 });
    assert.equal(run.status, ['missing-registration', 'stale-registration'].includes(scenario) ? 2 : 0, String(run.error || ''));
    if (run.status === 0) assert.equal(readFileSync(join(root, 'result.txt'), 'utf8'), scenario.includes('explicit') ? explicit : existing);
  });
}

test('installer stores only the explicitly chosen Unicode location', { skip: !compiler || process.platform !== 'win32' }, () => {
    const root = mkdtempSync(join(tmpdir(), 'mgs-installer-storage-'));
    const data = join(root, '应用数据 位置');
    const key = `Software\\MaxGameStudioInstallerTests\\${randomUUID()}`;
    const output = join(root, 'result.txt');
    const probe = join(root, 'probe.exe');
    const script = join(root, 'probe.nsi');
    const page = readFileSync(new URL('../../../frontend/src-tauri/windows/storage-page.nsh', import.meta.url), 'utf8')
      .replaceAll('Software\\MaxGameStudio\\Storage', key);
    const source = `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${probe}"
!include MUI2.nsh
!include FileFunc.nsh
!include nsDialogs.nsh
${page}
Function SkipIfPassive
FunctionEnd
Function CS2_AbortMigrationInstall
  DeleteRegKey HKCU "${key}"
  SetErrorLevel 2
  Quit
FunctionEnd
Section
  WriteRegStr HKCU "${key}" "LocationV1" "existing locator"
  WriteRegStr HKCU "${key}" "InstallerDataRootV1" "old selection"
  Call MGS_RecordDataDirectorySelection
  ReadRegStr $6 HKCU "${key}" "InstallerDataRootV1"
  StrCpy $MGSDataDirectoryConfirmed "1"
  Call MGS_RecordDataDirectorySelection
  ReadRegStr $7 HKCU "${key}" "InstallerDataRootV1"
  StrCpy $MGSSelectedDataDir "${data}"
  Call MGS_RecordDataDirectorySelection
  ReadRegStr $8 HKCU "${key}" "InstallerDataRootV1"
  ReadRegStr $5 HKCU "${key}" "LocationV1"
  FileOpen $9 "${output}" w
  FileWriteUTF16LE $9 "$6;$7;$8;$5"
  FileClose $9
  DeleteRegKey HKCU "${key}"
SectionEnd
`;
    writeFileSync(script, '\ufeff' + source);
    const build = spawnSync(resolve(compiler), ['/V2', script], { encoding: 'utf8', windowsHide: true });
    assert.equal(build.status, 0, build.stdout + build.stderr);
    const run = spawnSync(probe, ['/S'], { windowsHide: true, timeout: 15000 });
    assert.equal(run.status, 0, String(run.error || ''));
    const [untouched, cleared, selected, locator] = readFileSync(output, 'utf16le').split(';');
    assert.equal(untouched, 'old selection');
    assert.equal(cleared, '');
    const [request, path] = selected.split('|');
    assert.match(request, /^\d+-\d+$/);
    assert.equal(path, data);
    assert.equal(locator, 'existing locator');
});

test('native preflight rejects /D overlapping either product\u2019s files', { skip: !compiler || process.platform !== 'win32' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'mgs-directory-conflict-'));
  const league = join(root, 'League', 'Toolbox');
  const leagueData = join(root, 'League Data');
  const mainData = join(root, 'Main Data');
  const main = join(root, 'Main Program');
  const allowed = join(root, 'Fresh Program');
  for (const path of [league, leagueData, mainData, main, allowed]) mkdirSync(path, { recursive: true });
  const sentinel = join(leagueData, 'profile.sqlite');
  writeFileSync(sentinel, 'profile sentinel');
  writeFileSync(join(league, 'MaxGameStudioLeagueToolbox.exe'), 'other program');
  writeFileSync(join(main, 'cs2-insight-agent-desktop.exe'), 'own program');
  const alias = join(root, 'Toolbox Alias');
  symlinkSync(league, alias, 'junction');
  const roaming = join(root, 'Roaming');
  const control = join(roaming, 'MaxGameStudioLeague');
  mkdirSync(control, { recursive: true });
  writeFileSync(join(control, 'data-location.txt'), `\ufeff${leagueData}`, 'utf16le');
  const registry = `Software\\MaxGameStudioInstallerTests\\${randomUUID()}`;
  const disposableGuard = join(root, 'validate-install-directories.ps1');
  const guard = readFileSync(new URL('../../../frontend/src-tauri/windows/validate-install-directories.ps1', import.meta.url), 'utf8');
  writeFileSync(disposableGuard, guard
    .replaceAll('Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall', `${registry}\\Uninstall`)
    .replaceAll('Software\\b1d9ab62-895c-574e-a779-36d3f2ee3611', `${registry}\\League`)
    .replaceAll('Software\\cs2insightagent\\MaxGameStudio', `${registry}\\Main`)
    .replaceAll('Software\\MaxGameStudio\\Storage', `${registry}\\Storage`));
  const wrapper = readFileSync(new URL('../../../frontend/src-tauri/windows/directory-guard.nsh', import.meta.url), 'utf8')
    .replace('${__FILEDIR__}\\validate-install-directories.ps1', disposableGuard)
    .replace('Global\\MaxGameStudio.InstallDirectoryOwnership', 'Global\\MGSInstallerTest.' + randomUUID());
  const profileJson = JSON.stringify({ root: mainData }).replaceAll('"', '$\\"');
  const script = join(root, 'probe.nsi');
  const probe = join(root, 'probe.exe');
  const trace = join(root, 'directory-validation.txt');
  const source = `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${probe}"
!include LogicLib.nsh
Var MGSSelectedDataDir
${wrapper}
Section
  WriteRegStr HKCU "${registry}\\League" "InstallLocation" "${league}"
  WriteRegStr HKCU "${registry}\\Uninstall\\MaxGameStudio" "InstallLocation" '$\\"${main}$\\"'
  WriteRegStr HKCU "${registry}\\Storage" "LocationV1" "${profileJson}"
  ReadEnvStr $MGSSelectedDataDir "MGS_TEST_CHOSEN_DATA"
  Call MGS_ReserveInstallationDirectories
  StrCmp $R0 "0" +4
    DeleteRegKey HKCU "${registry}"
    SetErrorLevel 3
    Quit
  Call MGS_CheckProgramAndDataDirectories
  FileOpen $9 "${trace}" w
  FileWriteUTF16LE /BOM $9 "$R0 $R1"
  FileClose $9
  DeleteRegKey HKCU "${registry}"
  StrCmp $R0 "0" +3
    SetErrorLevel 2
    Quit
  FileOpen $0 "$INSTDIR\\fixture-payload.txt" w
  FileWrite $0 "permitted"
  FileClose $0
SectionEnd
`;
  writeFileSync(script, '\ufeff' + source);
  const build = spawnSync(resolve(compiler), ['/V2', script], { encoding: 'utf8', windowsHide: true });
  assert.equal(build.status, 0, build.stdout + build.stderr);
  const env = { ...process.env, APPDATA: roaming, LOCALAPPDATA: join(root, 'Local') };
  for (const [path, data] of [
    [league], [join(root, 'League')], [join(alias, 'resources')], [leagueData], [mainData], [control], [allowed, leagueData],
  ]) {
    const run = spawnSync(probe, [`/D=${path}`], {
      env: { ...env, MGS_TEST_CHOSEN_DATA: data ?? '' },
      windowsHide: true,
      windowsVerbatimArguments: true,
      timeout: 25000,
    });
    assert.equal(run.status, 2, `accepted conflicting /D=${path}`);
    assert.equal(existsSync(join(path, 'fixture-payload.txt')), false);
  }
  assert.equal(readFileSync(sentinel, 'utf8'), 'profile sentinel');
  for (const path of [main, allowed]) {
    const run = spawnSync(probe, [`/D=${path}`], { env, windowsHide: true, windowsVerbatimArguments: true, timeout: 15000 });
    const detail = existsSync(trace) ? readFileSync(trace, 'utf16le').trim() : String(run.error || '');
    assert.equal(run.status, 0, `rejected separate or registered /D=${path}: ${detail}`);
    assert.equal(readFileSync(join(path, 'fixture-payload.txt'), 'utf8'), 'permitted');
  }
});
