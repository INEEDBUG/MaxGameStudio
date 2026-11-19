import assert from 'node:assert/strict'
import test from 'node:test'
import { resolveReleaseBaseline, selectReleaseBaseline } from './release-baseline.mjs'

const digest = '7'.repeat(64)
const repairDigest = '583201413a2b31d94f3009766a22dfa6cda65ddb6eba3847d3895eefdf863c1e'
const apiUrl = 'https://github.com/INEEDBUG/MaxGameStudio/releases/download/'

function published(version, { league = false, repair = false } = {}) {
  const sha256 = repair ? repairDigest : digest
  const channel = league ? 'beta' : 'stable'
  const product = league ? 'maxgamestudio-league' : 'maxgamestudio'
  const tag = (league ? 'league-v' : 'v') + version
  const name = (league ? 'MaxGameStudioLeague_' : 'MaxGameStudio_') + version +
    (repair ? '_x64-repair-setup.exe' : '_x64-setup.exe')
  const url = apiUrl + tag + '/' + name
  const platform = { url, signature: 'signing-metadata' }
  const feed = league ? { product, channel, version, tag, url, sha256 } :
    { version, product, channel, tag_name: tag, platforms: { 'windows-x86_64': platform } }
  const asset = { name, browser_download_url: url, size: 36, state: 'uploaded', digest: 'sha256:' + sha256 }
  const release = { id: 401, tag_name: tag, draft: false, prerelease: league, assets: [asset] }
  return { feed, asset, release, expected: { version, tag, name, url, size: 36, sha256 } }
}

test('legacy 3.1.4 feed selects only the repair installer alongside the superseded original', () => {
  const { feed, release, expected } = published('3.1.4', { repair: true })
  const original = published('3.1.4').asset
  release.assets.unshift(original)
  delete feed.product
  delete feed.channel
  delete feed.tag_name
  assert.deepEqual(selectReleaseBaseline('main-stable', feed, release), expected)
  feed.platforms['windows-x86_64-nsis'] = { url: original.browser_download_url, signature: 'old' }
  assert.throws(() => selectReleaseBaseline('main-stable', feed, release), /platform/)
})

test('the 3.1.4 repair baseline rejects replaced bytes even under the original asset URL', () => {
  const { asset, feed, release } = published('3.1.4', { repair: true })
  asset.digest = 'sha256:' + digest
  assert.throws(() => selectReleaseBaseline('main-stable', feed, release), /3\.1\.4 repair/)
})

test('later stable and beta baselines are read from live feed identity rather than fixed versions', async () => {
  const main = published('3.4.0')
  const league = published('1.1.0-beta.3', { league: true })
  const calls = []
  const api = async (path) => {
    calls.push(path)
    if (path === 'releases/latest') return main.release
    if (path === 'releases/tags/league-v1.1.0-beta.3') return league.release
    const manifest = {
      'contents/latest.json?ref=updater': main.feed,
      'contents/league-beta.json?ref=updater': league.feed,
    }[path]
    assert.ok(manifest, 'Unexpected GitHub API path: ' + path)
    return { encoding: 'base64', content: Buffer.from(JSON.stringify(manifest)).toString('base64') }
  }
  assert.deepEqual(await resolveReleaseBaseline('main-stable', api), main.expected)
  assert.deepEqual(await resolveReleaseBaseline('league-beta', api), league.expected)
  assert.deepEqual(calls, [
    'contents/latest.json?ref=updater', 'releases/latest',
    'contents/league-beta.json?ref=updater', 'releases/tags/league-v1.1.0-beta.3',
  ])
})

test('selection rejects release mismatch, wrong channel status, incomplete asset and cross-product metadata', () => {
  const { feed, asset, release } = published('1.0.0-beta.2', { league: true })
  for (const wrong of [
    { tag_name: 'league-v1.0.0-beta.1' },
    { draft: true },
    { prerelease: false },
    { assets: [asset, asset] },
    { assets: [{ ...asset, digest: null }] },
    { assets: [{ ...asset, size: 0 }] },
    { assets: [{ ...asset, state: 'new' }] },
    { assets: [{ ...asset, browser_download_url: asset.browser_download_url + '?other' }] },
  ]) {
    assert.throws(() => selectReleaseBaseline('league-beta', feed, { ...release, ...wrong }))
  }

  for (const wrong of [
    { product: 'maxgamestudio' },
    { channel: 'stable' },
    { tag: 'v1.0.0-beta.2' },
    { url: 'https://example.com/package.exe' },
    { sha256: '8'.repeat(64) },
  ]) {
    assert.throws(() => selectReleaseBaseline('league-beta', { ...feed, ...wrong }, release))
  }
})

test('malformed League version cannot form a release lookup path', async () => {
  let apiCalls = 0
  await assert.rejects(resolveReleaseBaseline('league-beta', async () => {
    ++apiCalls
    return {
      encoding: 'base64',
      content: Buffer.from(JSON.stringify({ version: '1.0.0-beta.2/removed' })).toString('base64'),
    }
  }), /version/)
  assert.equal(apiCalls, 1)
})
