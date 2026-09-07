import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';

// Native NSIS behavior, not a JavaScript reimplementation. Production registry
// literals are remapped to an isolated disposable key before compilation.
const compiler = process.env.MGS_TEST_MAKENSIS;
const hook = readFileSync(new URL('../src-tauri/windows/upgrade-hooks.nsh', import.meta.url), 'utf8');
for (const scenario of ['registered', 'uninstall-fallback', 'missing-registration', 'stale-registration', 'explicit', 'fresh-explicit']) {
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
    const registry = scenario === 'registered' || scenario === 'explicit'
      ? `WriteRegStr HKCU "${key}\\Product" "" "${existing}"`
      : scenario === 'stale-registration'
        ? `WriteRegStr HKCU "${key}\\Product" "" "${wrong}"`
      : scenario === 'uninstall-fallback'
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
${remap(functions)}
Function CS2_AbortMigrationInstall
  DeleteRegKey HKCU "${key}"
  SetErrorLevel 2
  Quit
FunctionEnd
Section
  ${registry}
  StrCpy $INSTDIR "${scenario.includes('explicit') ? explicit : wrong}"
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
    const args = scenario === 'fresh-explicit' ? ['/S'] : ['/S', '/UPDATE'];
    if (scenario.includes('explicit')) args.push(`/D=${explicit}`);
    const run = spawnSync(join(root, 'probe.exe'), args, { windowsHide: true, windowsVerbatimArguments: true, timeout: 15000 });
    assert.equal(run.status, ['missing-registration', 'stale-registration'].includes(scenario) ? 2 : 0, String(run.error || ''));
    if (run.status === 0) assert.equal(readFileSync(join(root, 'result.txt'), 'utf8'), scenario.includes('explicit') ? explicit : existing);
  });
}
