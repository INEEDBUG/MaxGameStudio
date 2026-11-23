const assert = require('node:assert/strict')
const { cpSync, existsSync, mkdtempDisposableSync, readdirSync, readFileSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const test = require('node:test')
const { patchScopeTemplates } = require('./run-league-nsis.cjs')

// Pass the pinned League checkout that also supplies the installer build's dependencies.
const source = process.argv[2] || process.env.MAXGAMESTUDIO_LEAGUE_SOURCE_DIR

test('the native installer replaces the stock shortcut calls and preserves their arguments and branches', (t) => {
  assert.ok(source, 'Pass the League source directory with its locked node_modules.')
  const builder = join(source, 'node_modules', 'app-builder-lib')
  assert.equal(JSON.parse(readFileSync(join(builder, 'package.json'), 'utf8')).version, '26.15.6')
  const scratch = mkdtempDisposableSync(join(tmpdir(), 'mgs-native-shell-links-'))
  t.after(() => scratch.remove())
  const native = join(scratch.path, 'native')
  const electron = join(scratch.path, 'electron')
  for (const dir of [native, electron]) {
    cpSync(join(builder, 'templates', 'nsis'), dir, { recursive: true })
    patchScopeTemplates(dir, dir === native)
  }

  const filenames = ['include/installer.nsh', 'uninstaller.nsh']
  const counts = { SetLnkAUMI: 0, UninstShortcut: 0, UninstAppUserModelId: 0 }
  for (const filename of filenames) {
    const stock = readFileSync(join(electron, filename), 'utf8')
    const replaced = readFileSync(join(native, filename), 'utf8')
    assert.doesNotMatch(replaced, /WinShell::/i, filename)
    const expected = stock.replace(/WinShell::(\w+)/g, (_, operation) => {
      counts[operation]++
      return '!insertmacro MgsLeague' + operation
    })
    assert.equal(replaced.replace(/^!include "native-shell-links\.nsh"\r?\n/, ''), expected)
    assert.match(replaced, /^!include "native-shell-links\.nsh"\r?\n/)
  }
  assert.deepEqual(counts, { SetLnkAUMI: 5, UninstShortcut: 4, UninstAppUserModelId: 1 })
  for (const filename of readdirSync(native, { recursive: true }).filter(name => /\.ns[hi]$/i.test(name))) {
    assert.doesNotMatch(readFileSync(join(native, filename), 'utf8'), /WinShell::/i, filename)
  }
  assert.equal(existsSync(join(electron, 'include', 'native-shell-links.nsh')), false)
  assert.equal(
    readFileSync(join(native, 'include', 'native-shell-links.nsh'), 'utf8'),
    readFileSync(join(__dirname, 'native-shell-links.nsh'), 'utf8'),
  )
})
