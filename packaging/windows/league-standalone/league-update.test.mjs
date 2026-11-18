import assert from 'node:assert/strict'
import test from 'node:test'

import { leagueUpdate, leagueUpdateFeed } from './league-update.ts'

const version = '1.0.0-beta.2'
const tag = `league-v${version}`
const url = `https://github.com/INEEDBUG/MaxGameStudio/releases/download/${tag}/MaxGameStudioLeague_${version}_x64-setup.exe`
const release = { product: 'maxgamestudio-league', channel: 'beta', version, tag, url }

test('beta clients select the independent League feed and exact signed-release asset path', () => {
  assert.match(leagueUpdateFeed('1.0.0-beta.1'), /\/updater\/league-beta\.json$/)
  assert.match(leagueUpdateFeed('1.0.0'), /\/updater\/league-latest\.json$/)
  assert.deepEqual(leagueUpdate(release, '1.0.0-beta.1'), { version, url })
  assert.equal(leagueUpdate(release, '1.0.0-beta.2'), null)
  assert.equal(leagueUpdate(release, '1.0.0-beta.3'), null)
  assert.equal(leagueUpdate(release, '1.0.0'), null)
})

test('rejects other products, unscoped tags, unexpected files, and third-party URLs', () => {
  for (const delta of [
    { product: 'maxgamestudio' }, { channel: 'stable' }, { tag: 'v1.0.0-beta.2' },
    { url: url.replace('MaxGameStudioLeague_', 'MaxGameStudio_') },
    { url: url.replace('github.com', 'github.com.example.org') },
    { url: url + '?download=1' }, { version: '01.0.0-beta.2' }
  ]) {
    assert.throws(() => leagueUpdate({ ...release, ...delta }, '1.0.0-beta.1'))
  }
})
