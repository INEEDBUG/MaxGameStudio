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

function patchScopeTemplates(templates) {
  const section = join(templates, 'installSection.nsh')
  const original = readFileSync(section, 'utf8')
  const crossScopeUninstall = /^\$\{if\} \$installMode == "all"\r?\n  !insertmacro uninstallOldVersion HKEY_CURRENT_USER\r?\n  !insertmacro handleUninstallResult HKEY_CURRENT_USER\r?\n\$\{endIf\}\r?\n/gm
  const linkSetup = /^!insertmacro setLinkVars(?=\r?$)/gm
  const keepShortcuts = /^\$\{if\} \$isTryToKeepShortcuts == "true"(?=\r?$)/gm
  const sameScopeUninstall = /^!insertmacro uninstallOldVersion SHELL_CONTEXT\r?\n!insertmacro handleUninstallResult SHELL_CONTEXT(?=\r?$)/gm
  const launchLink = /^\$\{if\} \$\{FileExists\} "\$newStartMenuLink"(?=\r?$)/gm
  const commitInstall = /^!insertmacro installApplicationFiles\r?\n!insertmacro registryAddInstallInfo(?=\r?$)/gm
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
      '${if} $LeagueRepair != "1"',
      ...match.split(/\r?\n/).map((line) => '  ' + line),
      '${endif}',
    ].join('\n'))
    .replace(launchLink, (match) => match + '\n${andIf} $LeagueRepair != "1"')
    .replace(commitInstall, () => [
      '!insertmacro installApplicationFiles',
      '!insertmacro leagueBeforeRegistration',
      '!insertmacro registryAddInstallInfo',
    ].join('\n'))
  writeFileSync(section, patchedSection, 'utf8')
  patchExtractionRetries(join(templates, 'include', 'extractAppPackage.nsh'))
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
  patchScopeTemplates(templates)
  nsis.nsisTemplatesDir = templates
  process.argv = [process.argv[0], builder, ...cliArgs]
  require(builder)
}

module.exports = { patchInstallerTemplate, patchScopeTemplates }
