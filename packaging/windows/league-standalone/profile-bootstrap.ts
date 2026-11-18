import Database from 'better-sqlite3'
import { app, dialog } from 'electron'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path'

import { leagueProfilePath, migrateLeagueProfile } from './league-profile'

const control = join(app.getPath('appData'), 'MaxGameStudioLeague')
const activeFile = join(control, 'data-location.txt')
const selectionFile = join(control, 'installer-data-location.txt')

function readLocation(file: string) {
  return existsSync(file) ? readFileSync(file, 'utf16le').replace(/^\uFEFF/, '').trim() : ''
}

function legacyProfile() {
  // The Main installer owns this locator. Read the current location once; never update it.
  const command = `
    [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
    $entry = Get-ItemProperty -LiteralPath 'HKCU:\\Software\\MaxGameStudio\\Storage' -ErrorAction SilentlyContinue
    if ($null -ne $entry) { [string]$entry.LocationV1 }
    exit 0
  `
  const description = execFileSync(
    join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe'),
    ['-NoLogo', '-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(command, 'utf16le').toString('base64')],
    { encoding: 'utf8', windowsHide: true, timeout: 6000, maxBuffer: 32 * 1024 }
  ).trim()
  const location = description ? JSON.parse(description) : null
  const from = location?.mode === 'legacy_in_place'
    ? location.paths?.league_runtime
    : typeof location?.root === 'string' ? join(location.root, 'league-runtime') : null
  if (typeof from === 'string' && existsSync(from)) return from
  const fallback = join(app.getPath('appData'), 'MaxGameStudio', 'league-runtime')
  return existsSync(fallback) ? fallback : null
}

function snapshotDatabase(source: string, target: string) {
  const database = new Database(source, { readonly: true, fileMustExist: true, timeout: 4000 })
  try {
    database.prepare('VACUUM INTO ?').run(target)
  } finally {
    database.close()
  }
  const saved = new Database(target, { readonly: true, fileMustExist: true })
  try {
    if (saved.pragma('quick_check', { simple: true }) !== 'ok') {
      throw new Error('旧版工具箱数据库未通过检查。')
    }
  } finally {
    saved.close()
  }
}

export function prepareLeagueProfile() {
  try {
    mkdirSync(control, { recursive: true })
    const active = readLocation(activeFile)
    const target = leagueProfilePath(readLocation(selectionFile) || active || join(control, 'Profile'))
    const installRoot = resolve(dirname(process.execPath))
    const insideProgram = relative(installRoot, target)
    if (!isAbsolute(insideProgram) && insideProgram !== '..' && !insideProgram.startsWith('..' + sep)) {
      throw new Error('数据文件夹必须与程序安装文件夹分开。')
    }
    if (active && !existsSync(active)) throw new Error('原数据文件夹暂时无法访问。')
    if (!active || relative(active, target)) {
      migrateLeagueProfile(active || legacyProfile(), target, snapshotDatabase)
      const next = activeFile + '.next'
      writeFileSync(next, '\uFEFF' + target, 'utf16le')
      renameSync(next, activeFile)
    }
    app.setPath('userData', target)
    app.setPath('sessionData', target)
    const logs = join(target, 'logs')
    mkdirSync(logs, { recursive: true })
    app.setAppLogsPath(logs)
    if (!app.commandLine.hasSwitch('log-file') && !process.env.ELECTRON_LOG_FILE) {
      app.commandLine.appendSwitch('log-file', join(logs, 'debug.log'))
    }
    return true
  } catch (error) {
    const message = error instanceof Error && /^[\u4E00-\u9FFF]/.test(error.message)
      ? error.message
      : '请先关闭旧版工作台，再确认数据磁盘可用，或重新运行安装向导选择空文件夹。'
    dialog.showErrorBox('英雄联盟工具箱数据未就绪', message + '\n原始数据仍保留。')
    return false
  }
}
