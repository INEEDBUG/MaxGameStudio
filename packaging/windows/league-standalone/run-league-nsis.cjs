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
  if ([...original.matchAll(crossScopeUninstall)].length !== 1) {
    throw new Error('Expected one pinned cross-scope uninstall block.')
  }
  // In over-the-shoulder UAC the elevated HKCU belongs to the credential administrator.
  writeFileSync(section, original.replace(crossScopeUninstall, ''), 'utf8')
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
