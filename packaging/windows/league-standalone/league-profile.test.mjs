import assert from 'node:assert/strict'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { migrateLeagueProfile } from './league-profile.ts'

test('stages app settings, keeps the source intact, and can reopen its committed destination', () => {
  const root = mkdtempSync(join(tmpdir(), 'mgs-league-profile-test-'))
  try {
    const source = join(root, 'old')
    const target = join(root, 'new')
    mkdirSync(join(source, 'AkariConfig'), { recursive: true })
    mkdirSync(target)
    writeFileSync(join(source, 'base-config.json'), '{"logLevel":"info"}')
    writeFileSync(join(source, 'AkariConfig', 'example.json'), '{}')
    writeFileSync(join(source, 'Cookies'), 'fixture')
    const snapshot = () => assert.fail('no database to snapshot')
    migrateLeagueProfile(source, target, snapshot)
    assert.equal(readFileSync(join(target, 'base-config.json'), 'utf8'), '{"logLevel":"info"}')
    assert.ok(existsSync(join(target, 'AkariConfig', 'example.json')))
    assert.ok(!existsSync(join(target, 'Cookies')))
    assert.ok(existsSync(join(source, 'base-config.json')))
    migrateLeagueProfile(source, target, snapshot)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('an occupied destination or failed database snapshot preserves both paths', () => {
  const root = mkdtempSync(join(tmpdir(), 'mgs-league-profile-test-'))
  try {
    const source = join(root, 'old')
    const target = join(root, 'new')
    mkdirSync(source)
    mkdirSync(target)
    const file = join(source, 'LeagueAkari.db')
    writeFileSync(file, 'fixture database')
    const snapshot = () => { throw new Error('snapshot failed') }
    assert.throws(() => migrateLeagueProfile(source, source, snapshot), /相互包含/)
    assert.throws(() => migrateLeagueProfile(source, target, snapshot), /snapshot failed/)
    assert.equal(readFileSync(file, 'utf8'), 'fixture database')
    writeFileSync(join(target, 'other.txt'), 'keep')
    assert.throws(() => migrateLeagueProfile(source, target, snapshot), /目标数据文件夹/)
    assert.equal(readFileSync(join(target, 'other.txt'), 'utf8'), 'keep')
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
