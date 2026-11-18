import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, writeFileSync, mkdirSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';

// Native NSIS behavior, not a JavaScript reimplementation. Production registry
// literals are remapped to an isolated disposable key before compilation.
const compiler = process.env.MGS_TEST_MAKENSIS;
const hook = readFileSync(new URL('../../../frontend/src-tauri/windows/upgrade-hooks.nsh', import.meta.url), 'utf8');
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
    .replace('${__FILEDIR__}\\validate-install-directories.ps1', disposableGuard);
  const profileJson = JSON.stringify({ root: mainData }).replaceAll('"', '$\\"');
  const script = join(root, 'probe.nsi');
  const probe = join(root, 'probe.exe');
  const source = `Unicode true
RequestExecutionLevel user
SilentInstall silent
OutFile "${probe}"
!include LogicLib.nsh
Var MGSSelectedDataDir
${wrapper}
Section
  WriteRegStr HKCU "${registry}\\League" "InstallLocation" "${league}"
  WriteRegStr HKCU "${registry}\\Main" "" "${main}"
  WriteRegStr HKCU "${registry}\\Storage" "LocationV1" "${profileJson}"
  ReadEnvStr $MGSSelectedDataDir "MGS_TEST_CHOSEN_DATA"
  Call MGS_CheckProgramAndDataDirectories
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
    assert.equal(run.status, 0, `rejected separate or registered /D=${path}`);
    assert.equal(readFileSync(join(path, 'fixture-payload.txt'), 'utf8'), 'permitted');
  }
});
