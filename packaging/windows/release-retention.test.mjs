import assert from 'node:assert/strict'
import test from 'node:test'
import { createHash } from 'node:crypto'
import { mkdtemp, writeFile, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { planRetention, inspectFeeds, backupReleases, applyRetention, hasPublishedUpdate } from './release-retention.mjs'

export function release(id, tag, overrides = {}) {
  const league = tag.startsWith('league-v')
  const version = tag.replace(/^(?:league-)?v/, '')
  const name = !league && version === '3.1.4' ? 'MaxGameStudio_3.1.4_x64-repair-setup.exe' :
    `${league ? 'MaxGameStudioLeague' : 'MaxGameStudio'}_${version}_x64-setup.exe`
  return { id, tag_name: tag, draft: false, prerelease: tag.includes('-beta.'),
    assets: [{ id: id * 10, name, size: 10, state: 'uploaded', digest: 'sha256:' + 'a'.repeat(64),
      browser_download_url: `https://github.com/INEEDBUG/MaxGameStudio/releases/download/${tag}/${name}` }], ...overrides }
}

test('each app keeps one numeric latest beta and three stable releases', () => {
  const releases = [release(1, 'v3.2.0-beta.2'), release(2, 'v3.2.0-beta.10'),
    release(3, 'league-v1.0.0-beta.1'), release(4, 'league-v1.0.0-beta.2'),
    ...['3.1.4', '3.1.3', '3.0.5', '2.5.16'].map((v, i) => release(10 + i, 'v' + v)),
    ...['1.0.0', '1.0.1', '1.0.2', '1.1.0'].map((v, i) => release(20 + i, 'league-v' + v))]
  assert.deepEqual(planRetention(releases, ['v3.2.0-beta.10', 'league-v1.0.0-beta.2', 'v3.1.4']).remove.map(r => r.id).sort((a, b) => a - b), [1, 3, 13, 20])
})

test('active updater targets outside the cap stop deletion', () => {
  assert.throws(() => planRetention([release(1, 'v3.2.0-beta.1'), release(2, 'v3.2.0-beta.2')], ['v3.2.0-beta.1']), /active update/i)
})

test('drafts and unrelated package releases are not installer cleanup targets', () => {
  const unrelated = { id: 99, tag_name: 'demoparser-v0.41.4', draft: false, prerelease: false, assets: [] }
  const draft = release(98, 'v8.0.0-beta.1', { draft: true })
  assert.deepEqual(planRetention([unrelated, draft, release(1, 'v3.2.0-beta.2')], []).remove, [])
})

test('release IDs cannot become API paths', () => {
  assert.throws(() => planRetention([release('1/other', 'v3.2.0-beta.1'), release(2, 'v3.2.0-beta.2')]), /release id/i)
})

const payload = Buffer.from('0123456789')
const hash = createHash('sha256').update(payload).digest('hex')
function fixture() {
  const releases = [release(1, 'v3.2.0-beta.1'), release(2, 'v3.2.0-beta.2'), release(3, 'v3.1.4'), release(4, 'league-v1.0.0-beta.2')]
  for (const row of releases) row.assets[0].digest = 'sha256:' + hash
  const feed = (row, league = false) => ({ sha: 'feed-' + row.id, content: Buffer.from(JSON.stringify(league ? {
    version: '1.0.0-beta.2', product: 'maxgamestudio-league', channel: 'beta', tag: row.tag_name,
    url: row.assets[0].browser_download_url, sha256: hash,
  } : { version: row.tag_name.slice(1), platforms: { 'windows-x86_64': {
    url: row.assets[0].browser_download_url, signature: 'synthetic signed updater',
  } } })).toString('base64') })
  const world = { releases, feeds: { 'latest.json': feed(releases[2]), 'beta.json': feed(releases[1]),
    'league-beta.json': feed(releases[3], true), 'league-latest.json': null }, latest: releases[2],
    readmes: { 'README.md': { sha: 'english' }, 'README.zh-CN.md': { sha: 'chinese' } } }
  const { protectedTags } = inspectFeeds(world)
  return { world, plan: { snapshot: structuredClone(world), ...planRetention(releases, protectedTags) } }
}

test('feed verification rejects dangling links and cross-product manifests', () => {
  const { world } = fixture()
  world.feeds['league-beta.json'].content = Buffer.from(JSON.stringify({ version: '1.0.0-beta.2',
    product: 'maxgamestudio', url: world.releases[1].assets[0].browser_download_url })).toString('base64')
  assert.throws(() => inspectFeeds(world), /updater identity/i)
})

async function recoveredFixture() {
  const data = fixture()
  const directory = await mkdtemp(join(tmpdir(), 'mgs-release-retention-'))
  const calls = []
  const api = (path, options = {}) => {
    if (path.startsWith('git/ref/tags/')) return { ref: 'refs/tags/' + path.slice('git/ref/tags/'.length), object: { type: 'commit', sha: 'b'.repeat(40) } }
    if (path.startsWith('releases/')) {
      const id = Number(path.split('/')[1])
      if (options.method === 'DELETE') {
        calls.push('DELETE ' + id)
        data.world.releases = data.world.releases.filter(row => row.id !== id)
        return null
      }
      return data.world.releases.find(row => row.id === id) ?? null
    }
    throw new Error('Unexpected API operation: ' + path)
  }
  await backupReleases(data.plan, directory, api, async (_release, asset, folder) => {
    await writeFile(join(folder, asset.name), payload)
  })
  return { ...data, directory, calls, api }
}

test('verified recovery and README refresh precede exact release deletion, never tags', async () => {
  const f = await recoveredFixture()
  const deleted = await applyRetention(f.api, f.plan, f.directory, () => structuredClone(f.world), async () => f.calls.push('README'))
  assert.deepEqual(deleted, [1])
  assert.deepEqual(f.calls, ['README', 'DELETE 1'])
  const records = JSON.parse(await readFile(join(f.directory, 'releases.json'), 'utf8'))
  assert.equal(records[0].tag.object.sha, 'b'.repeat(40))
})

test('corrupt recovery refuses deletion before README or API mutations', async () => {
  const f = await recoveredFixture()
  await writeFile(join(f.directory, '1', f.plan.remove[0].assets[0].name), 'corrupt')
  await assert.rejects(applyRetention(f.api, f.plan, f.directory, () => structuredClone(f.world), async () => f.calls.push('README')), /backup changed/i)
  assert.deepEqual(f.calls, [])
})

test('a changed active feed after planning refuses all deletion', async () => {
  const f = await recoveredFixture()
  f.world.feeds['beta.json'].sha = 'newer feed'
  await assert.rejects(applyRetention(f.api, f.plan, f.directory, () => structuredClone(f.world), async () => f.calls.push('README')), /state changed/i)
  assert.deepEqual(f.calls, [])
})

test('moving a source tag after recovery prevents deletion', async () => {
  const f = await recoveredFixture()
  const changedTagApi = (path, options) => path.startsWith('git/ref/tags/') ?
    { ref: 'refs/tags/v3.2.0-beta.1', object: { type: 'commit', sha: 'c'.repeat(40) } } : f.api(path, options)
  await assert.rejects(applyRetention(changedTagApi, f.plan, f.directory, () => structuredClone(f.world), async () => f.calls.push('README')), /tag/i)
  assert.equal(f.calls.some(call => call.startsWith('DELETE')), false)
})

test('README update failure cannot retire a linked download', async () => {
  const f = await recoveredFixture()
  await assert.rejects(applyRetention(f.api, f.plan, f.directory, () => structuredClone(f.world), async () => { throw new Error('README update failed') }), /README update failed/)
  assert.deepEqual(f.calls, [])
})

test('a modified deletion plan cannot remove the current updater release', async () => {
  const { world, plan } = fixture()
  plan.remove = [world.releases.find(row => row.id === 2)]
  const directory = await mkdtemp(join(tmpdir(), 'mgs-release-retention-plan-'))
  const calls = []
  const api = (path, options = {}) => {
    if (options.method === 'DELETE') calls.push('DELETE')
    return { ref: 'refs/tags/' + path.slice('git/ref/tags/'.length), object: { type: 'commit', sha: 'b'.repeat(40) } }
  }
  await backupReleases(plan, directory, api, async (_r, asset, folder) => writeFile(join(folder, asset.name), payload))
  await assert.rejects(applyRetention(api, plan, directory, () => structuredClone(world), async () => calls.push('README')))
  assert.deepEqual(calls, [])
})

test('a checksum mismatch cannot create a successful recovery manifest', async () => {
  const { plan } = fixture()
  const directory = await mkdtemp(join(tmpdir(), 'mgs-release-retention-bad-'))
  await assert.rejects(backupReleases(plan, directory, () => ({}), async (_r, asset, folder) => {
    await writeFile(join(folder, asset.name), 'bad')
  }), /checksum mismatch/i)
})

test('only successful trusted publication runs enable automatic cleanup', () => {
  const run = { conclusion: 'success', head_repository: { full_name: 'INEEDBUG/MaxGameStudio' },
    event: 'workflow_dispatch', head_branch: 'codex/all-users-installers', path: '.github/workflows/verify-windows-upgrade.yml' }
  const jobs = { jobs: [{ name: 'publish', conclusion: 'success', steps: [{ name: 'Publish the verified beta pair', conclusion: 'success' }] }] }
  const api = path => path.includes('/jobs?') ? jobs : run
  assert.equal(hasPublishedUpdate(api, 123), true)
  jobs.jobs[0].steps[0].conclusion = 'skipped'
  assert.equal(hasPublishedUpdate(api, 123), false)
  jobs.jobs[0].steps[0].conclusion = 'success'
  run.head_branch = 'codex/untrusted-test'
  assert.equal(hasPublishedUpdate(api, 123), false)
  run.head_branch = 'codex/all-users-installers'
  run.head_repository.full_name = 'untrusted/fork'
  assert.equal(hasPublishedUpdate(api, 123), false)
})
