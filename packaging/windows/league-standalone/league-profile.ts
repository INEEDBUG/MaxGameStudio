import { createHash } from 'node:crypto'
import {
  copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync,
  realpathSync, renameSync, rmSync, rmdirSync, writeFileSync
} from 'node:fs'
import { basename, dirname, isAbsolute, join, parse, relative, resolve, sep } from 'node:path'

const profileMarker = '.maxgamestudio-league-profile'

function contains(root: string, target: string) {
  const part = relative(root, target)
  return !part || (!part.startsWith('..' + sep) && part !== '..' && !isAbsolute(part))
}

export function leagueProfilePath(path: string) {
  if (!isAbsolute(path) || (process.platform === 'win32' && !/^[A-Z]:[\\/]/i.test(path))) {
    throw new Error('请选择本机磁盘上的数据文件夹。')
  }
  const full = resolve(path)
  if (parse(full).root === full) throw new Error('不能将磁盘根目录用作数据文件夹。')
  const parent = dirname(full)
  const existing = existsSync(full)
  if (existing && !lstatSync(full).isDirectory()) throw new Error('数据路径必须是普通文件夹。')
  const canonical = existing ? realpathSync(full) : join(leagueProfilePathParent(parent), basename(full))
  if (relative(full, canonical)) throw new Error('数据路径不能经过快捷方式、联接或符号链接。')
  return canonical
}

function leagueProfilePathParent(path: string): string {
  return existsSync(path) ? realpathSync(path) : join(leagueProfilePathParent(dirname(path)), basename(path))
}

function fileDigest(path: string) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function copySettings(from: string, to: string) {
  const info = lstatSync(from)
  if (info.isDirectory()) {
    mkdirSync(to)
    for (const name of readdirSync(from)) copySettings(join(from, name), join(to, name))
  } else if (info.isFile()) {
    copyFileSync(from, to)
    if (fileDigest(from) !== fileDigest(to)) throw new Error('设置文件在迁移过程中发生变化，请重试。')
  } else {
    throw new Error('设置文件夹包含不支持的链接或特殊文件。')
  }
}

export function migrateLeagueProfile(
  source: string | null,
  target: string,
  snapshotDatabase: (source: string, target: string) => void
) {
  const destination = leagueProfilePath(target)
  const original = source && leagueProfilePath(source)
  if (original && (contains(original, destination) || contains(destination, original))) {
    throw new Error('新旧数据文件夹不能相互包含。')
  }
  if (existsSync(join(destination, profileMarker))) return
  if (existsSync(destination) && readdirSync(destination).length) {
    throw new Error('目标数据文件夹已有其他文件，请选择空文件夹。')
  }
  mkdirSync(dirname(destination), { recursive: true })
  const stage = mkdtempSync(join(dirname(destination), '.mgs-league-'))
  try {
    if (original) {
      for (const name of ['base-config.json', 'AkariConfig']) {
        const from = join(original, name)
        if (existsSync(from)) copySettings(from, join(stage, name))
      }
      const from = join(original, 'LeagueAkari.db')
      if (existsSync(from)) {
        if (!lstatSync(from).isFile()) throw new Error('旧版工具箱数据库不是普通文件。')
        snapshotDatabase(from, join(stage, 'LeagueAkari.db'))
      }
    }
    writeFileSync(join(stage, profileMarker), '1\n', { flag: 'wx' })
    if (existsSync(destination)) rmdirSync(destination)
    renameSync(stage, destination)
  } finally {
    if (existsSync(stage)) rmSync(stage, { recursive: true })
  }
}
