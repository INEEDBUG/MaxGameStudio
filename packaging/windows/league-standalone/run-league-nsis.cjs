const { cpSync, readFileSync, writeFileSync } = require('node:fs')
const { createRequire } = require('node:module')
const { join, resolve } = require('node:path')

function patchInstallerTemplate(template) {
  const startup = /^Function \.onInit\r?\n  Call setInstallSectionSpaceRequired\r?\n\r?\n  SetOutPath \$INSTDIR(?=\r?$)/gm
  if ([...template.matchAll(startup)].length !== 1) {
    throw new Error('Expected one pinned electron-builder .onInit output path.')
  }
  // SetOutPath creates the directory. Redirect startup before customInit validates the selected path.
  return template.replace(startup, (match) => match.replace('$INSTDIR', () => '$TEMP'))
}

function patchScopeTemplates(templates, nativeShortcuts = false) {
  const section = join(templates, 'installSection.nsh')
  const original = readFileSync(section, 'utf8')
  const crossScopeUninstall = /^\$\{if\} \$installMode == "all"\r?\n  !insertmacro uninstallOldVersion HKEY_CURRENT_USER\r?\n  !insertmacro handleUninstallResult HKEY_CURRENT_USER\r?\n\$\{endIf\}\r?\n/gm
  const linkSetup = /^!insertmacro setLinkVars(?=\r?$)/gm
  const keepShortcuts = /^\$\{if\} \$isTryToKeepShortcuts == "true"(?=\r?$)/gm
  const sameScopeUninstall = /^!insertmacro uninstallOldVersion SHELL_CONTEXT\r?\n!insertmacro handleUninstallResult SHELL_CONTEXT(?=\r?$)/gm
  const launchLink = /^\$\{if\} \$\{FileExists\} "\$newStartMenuLink"(?=\r?$)/gm
  const commitInstall = /^SetOutPath \$INSTDIR\r?\n\r?\n!ifdef UNINSTALLER_ICON\r?\n  File \/oname=uninstallerIcon\.ico "\$\{UNINSTALLER_ICON\}"\r?\n!endif\r?\n\r?\n!insertmacro installApplicationFiles\r?\n!insertmacro registryAddInstallInfo(?=\r?$)/gm
  for (const anchor of [crossScopeUninstall, linkSetup, keepShortcuts, sameScopeUninstall, launchLink, commitInstall]) {
    if ([...original.matchAll(anchor)].length !== 1) throw new Error('Expected the pinned League install section.')
  }
  const patchedSection = original
    // The elevated HKCU can belong to a different credential administrator.
    .replace(crossScopeUninstall, '')
    .replace(linkSetup, (match) => [
      '!insertmacro leagueBeforeInstall',
      match,
      '${if} $LeagueRepair == "1"',
      '  StrCpy $oldMenuDirectory ""',
      '${endif}',
    ].join('\n'))
    // Never run a command or reuse shortcut information from an incomplete registration.
    .replace(keepShortcuts, (match) => match + '\n${andIf} $LeagueRepair != "1"')
    .replace(sameScopeUninstall, (match) => [
      '!ifndef MGS_LEAGUE_NATIVE_PACKAGE',
      '${if} $LeagueRepair != "1"',
      ...match.split(/\r?\n/).map((line) => '  ' + line),
      '${endif}',
      '!endif',
    ].join('\n'))
    .replace(launchLink, (match) => match + '\n${andIf} $LeagueRepair != "1"')
    .replace(commitInstall, (match) => [
      '!ifdef MGS_LEAGUE_NATIVE_PACKAGE',
      '!insertmacro leagueInstallNativeFiles',
      '!else',
      match.replace(/\r?\n!insertmacro registryAddInstallInfo$/, ''),
      '!endif',
      '!insertmacro leagueBeforeRegistration',
      '!insertmacro registryAddInstallInfo',
    ].join('\n'))
  writeFileSync(section, patchedSection, 'utf8')
  patchInstallUtilities(join(templates, 'include', 'installUtil.nsh'))
  patchNativeUninstaller(join(templates, 'uninstaller.nsh'))
  patchExtractionRetries(join(templates, 'include', 'extractAppPackage.nsh'))
  if (nativeShortcuts) patchNativeShellLinks(templates)
  for (const filename of ['installer.nsi', 'multiUserUi.nsh']) {
    const path = join(templates, filename)
    const text = readFileSync(path, 'utf8')
    const failure = /MessageBox mb_IconStop\|mb_TopMost\|mb_SetForeground "Unable to elevate, error \$0"/g
    if ([...text.matchAll(failure)].length !== 1) throw new Error(`Missing pinned elevation prompt in ${filename}.`)
    let localized = text.replace(failure, () =>
      'MessageBox mb_IconStop|mb_TopMost|mb_SetForeground "无法获取管理员权限（错误 $0）。" /SD IDOK')
    if (filename === 'multiUserUi.nsh') {
      const secondaryLogon = '"Logon service not running, aborting!"'
      if (text.split(secondaryLogon).length !== 2) throw new Error('Missing pinned Secondary Logon prompt.')
      localized = localized.replace(secondaryLogon, '"Windows 登录服务未运行，无法请求管理员权限。"')
    } else {
      const cancelled = /(\$\{Case\} 1223[^\r\n]*\r?\n)( +)(\$\{Break\})/
      const failed = /(\$\{Default\}\r?\n)( +)(MessageBox mb_IconStop\|mb_TopMost\|mb_SetForeground)/
      if (!cancelled.test(localized) || !failed.test(localized)) throw new Error('Missing pinned silent UAC branches.')
      localized = localized
        .replace(cancelled, '$1$2SetErrorLevel 1223\n$2$3')
        .replace(failed, (_, before, indent, message) => `${before}${indent}SetErrorLevel $0\n${indent}${message}`)
    }
    writeFileSync(path, localized, 'utf8')
  }
}

function patchNativeShellLinks(templates) {
  const header = 'native-shell-links.nsh'
  const files = [
    ['include/installer.nsh', { SetLnkAUMI: 5, UninstShortcut: 2 }],
    ['uninstaller.nsh', { UninstAppUserModelId: 1, UninstShortcut: 2 }],
  ]
  for (const [filename, operations] of files) {
    const path = join(templates, filename)
    let content = readFileSync(path, 'utf8')
    for (const [operation, expected] of Object.entries(operations)) {
      const call = 'WinShell::' + operation + ' '
      if (content.split(call).length - 1 !== expected) {
        throw new Error('Unexpected number of ' + call.trim() + ' calls in ' + filename)
      }
      content = content.replaceAll(call, '!insertmacro MgsLeague' + operation + ' ')
    }
    if (/WinShell::/i.test(content)) throw new Error('Unhandled WinShell call in ' + filename)
    writeFileSync(path, '!include "' + header + '"\n' + content, 'utf8')
  }
  cpSync(join(__dirname, header), join(templates, 'include', header))
}

function patchInstallUtilities(path) {
  const original = readFileSync(path, 'utf8')
  const oldPathHelpers = /^Function GetInQuotes\r?\n[\s\S]*?^Function GetFileParent\r?\n[\s\S]*?^FunctionEnd(?=\r?$)/gm
  const oldUninstaller = /^!macro readReg VAR ROOT_KEY SUB_KEY NAME\r?\n[\s\S]*?^!macro uninstallOldVersion ROOT_KEY\r?\n[\s\S]*?^!macroend(?=\r?$)/gm
  for (const anchor of [oldPathHelpers, oldUninstaller]) {
    if ([...original.matchAll(anchor)].length !== 1) throw new Error('Expected the pinned League old-uninstaller utilities.')
  }
  // The copy and shortcut macros are still used by the native installer.
  const electronOnly = (match) => '!ifndef MGS_LEAGUE_NATIVE_PACKAGE\n' + match + '\n!endif'
  writeFileSync(path, original.replace(oldPathHelpers, electronOnly).replace(oldUninstaller, electronOnly), 'utf8')
}

function patchNativeUninstaller(path) {
  const original = readFileSync(path, 'utf8')
  const workingDirectory = /^Function un\.onInit\r?\n  SetOutPath \$INSTDIR(?=\r?$)/gm
  const deleteAppData = /^  Var \/GLOBAL isDeleteAppData\r?\n[\s\S]*?(?=^  DeleteRegKey SHELL_CONTEXT "\$\{UNINSTALL_REGISTRY_KEY\}")/gm
  for (const anchor of [workingDirectory, deleteAppData]) {
    if ([...original.matchAll(anchor)].length !== 1) throw new Error('Expected the pinned League uninstaller.')
  }
  const patched = original
    .replace(workingDirectory, (match) => [
      'Function un.onInit',
      '  !ifdef MGS_LEAGUE_NATIVE_PACKAGE',
      '    SetOutPath "$WINDIR"',
      '  !else',
      match.split(/\r?\n/)[1],
      '  !endif',
    ].join('\n'))
    .replace(deleteAppData, (match) => '  !ifndef MGS_LEAGUE_NATIVE_PACKAGE\n' + match + '  !endif\n')
  writeFileSync(path, patched, 'utf8')
}

function patchExtractionRetries(path) {
  const original = readFileSync(path, 'utf8')
  const copy = /^    CopyFiles \/SILENT "\$PLUGINSDIR\\7z-out\\\*" \$OUTDIR(?=\r?$)/gm
  const retry = /^      MessageBox MB_RETRYCANCEL\|MB_ICONEXCLAMATION "\$\(appCannotBeClosed\)" \/SD IDRETRY IDCANCEL AbortExtract7za(?=\r?$)/gm
  const uncheckedFallback = /^    # As an absolutely last resort[\s\S]*?^  AbortExtract7za:\r?\n    Quit(?=\r?$)/gm
  for (const anchor of [copy, retry, uncheckedFallback]) {
    if ([...original.matchAll(anchor)].length !== 1) throw new Error('Expected the pinned League extraction retry.')
  }
  writeFileSync(path, original
    .replace(copy, (line) => '    ClearErrors\n' + line)
    .replace(retry, () =>
      '      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "无法复制工具箱程序文件。请检查磁盘空间和安装目录的写入权限，然后重试。" /SD IDCANCEL IDCANCEL AbortExtract7za IDRETRY RetryExtract7za')
    // Retrying must go through CopyFiles again. Stock direct extraction can silently skip files.
    .replace(uncheckedFallback, () => '  AbortExtract7za:\n    SetErrorLevel 2\n    Quit'), 'utf8')
}

if (require.main === module) {
  const [, , cli, scratch, ...cliArgs] = process.argv
  const nativeShortcuts = cliArgs[0] === '--native-shortcuts'
  if (nativeShortcuts) cliArgs.shift()
  const builder = resolve(cli)
  const pinned = createRequire(builder)
  if (pinned('electron-builder/package.json').version !== '26.15.6') {
    throw new Error('The League installer template is pinned to electron-builder 26.15.6.')
  }

  // The custom script option skips stock uninstaller generation. Redirect only this builder process.
  const nsis = pinned('app-builder-lib/out/targets/nsis/nsisUtil.js')
  const templates = join(scratch, 'stock-nsis')
  const patched = patchInstallerTemplate(readFileSync(join(nsis.nsisTemplatesDir, 'installer.nsi'), 'utf8'))
  cpSync(nsis.nsisTemplatesDir, templates, { recursive: true, force: false, errorOnExist: true })
  writeFileSync(join(templates, 'installer.nsi'), patched, 'utf8')
  patchScopeTemplates(templates, nativeShortcuts)
  nsis.nsisTemplatesDir = templates
  process.argv = [process.argv[0], builder, ...cliArgs]
  require(builder)
}

module.exports = { patchInstallerTemplate, patchScopeTemplates }
