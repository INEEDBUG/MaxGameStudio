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
  nsis.nsisTemplatesDir = templates
  process.argv = [process.argv[0], builder, ...cliArgs]
  require(builder)
}

module.exports = { patchInstallerTemplate }
