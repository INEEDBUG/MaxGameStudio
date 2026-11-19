import assert from 'node:assert/strict'
import test from 'node:test'
import { approvedBetas, publishFeeds, stableState, validatePreviousBeta, validateRecord, verifyRelease } from './publish-verified-beta.mjs'

const sha256 = 'a'.repeat(64)
const accepted = {
  sourceSha: 'f'.repeat(40),
  verifyRunId: '100',
  candidateRunIds: ['101', '102'],
  sha256: Object.fromEntries(approvedBetas.flatMap(({ installer, candidateSidecar }) =>
    [[installer, sha256], [installer + candidateSidecar, sha256]]
  ))
}

test('beta promotion protects the public stable snapshot for later releases too', () => {
  const version = '3.4.1'
  const tag = 'v' + version
  const name = 'MaxGameStudio_' + version + '_x64-setup.exe'
  const url = 'https://github.com/INEEDBUG/MaxGameStudio/releases/download/' + tag + '/' + name
  const manifest = {
    version, product: 'maxgamestudio', channel: 'stable', tag_name: tag,
    platforms: { 'windows-x86_64': { url, signature: 'signature-metadata' } },
  }
  const responses = {
    'releases/latest': {
      id: 734,
      tag_name: tag,
      draft: false,
      prerelease: false,
      assets: [{ name, size: 160, digest: 'sha256:' + sha256, state: 'uploaded', browser_download_url: url }],
    },
    'contents/latest.json?ref=updater': {
      sha: 'b'.repeat(40),
      encoding: 'base64',
      content: Buffer.from(JSON.stringify(manifest)).toString('base64'),
    },
    'contents/league-latest.json?ref=updater': null,
  }
  const read = (path) => responses[path]
  assert.deepEqual(stableState(read), [734, 'b'.repeat(40), null])
  responses['releases/latest'].tag_name = 'v3.4.0'
  assert.throws(() => stableState(read), /does not match the updater baseline/)
})

test('accepts only the exact versions and files sealed for this verifier run', () => {
  assert.doesNotThrow(() => validateRecord(accepted, '100'))
  assert.throws(() => validateRecord(accepted, '200'))
  assert.throws(() => validateRecord({ ...accepted, sourceSha: 'main' }, '100'))
  assert.throws(() => validateRecord({ ...accepted, sha256: { ...accepted.sha256, 'additional.exe': sha256 } }, '100'))
  const omitted = { ...accepted.sha256 }
  delete omitted[approvedBetas[0].installer + '.sig']
  assert.throws(() => validateRecord({ ...accepted, sha256: omitted }, '100'))
})

test('reuses an existing prerelease only when every allowed asset has the same digest', () => {
  const beta = approvedBetas[0]
  const expected = [beta.installer, beta.installer + '.sha256'].map((name) => ({ name, sha256, size: 50 }))
  const release = {
    draft: false, prerelease: true, tag_name: beta.tag,
    assets: expected.map(({ name, sha256, size }) => ({
      name, size, state: 'uploaded', digest: 'sha256:' + sha256,
      browser_download_url: `https://github.com/INEEDBUG/MaxGameStudio/releases/download/${beta.tag}/${name}`
    }))
  }
  assert.doesNotThrow(() => verifyRelease(release, beta, expected))
  assert.throws(() => verifyRelease({ ...release, prerelease: false }, beta, expected))
  assert.throws(() => verifyRelease({ ...release, assets: [{ ...release.assets[0], digest: 'sha256:' + 'b'.repeat(64) }, release.assets[1]] }, beta, expected))
})

test('changes only the beta feeds in one fast-forward commit', () => {
  const calls = []
  const responses = [{ tree: { sha: 'stable-tree' } }, { sha: 'new-tree' }, { sha: 'new-commit' }, {}]
  const api = (...args) => { calls.push(args); return responses.shift() }
  assert.throws(() => publishFeeds(api, 'stable-commit', [{ path: 'latest.json', content: '{}' }]))
  assert.equal(calls.length, 0)
  const files = approvedBetas.map(({ feed }) => ({ path: feed, content: '{}' }))
  publishFeeds(api, 'stable-commit', files)
  assert.equal(calls[1][1].body.base_tree, 'stable-tree')
  assert.deepEqual(calls[1][1].body.tree.map(({ path }) => path), ['beta.json', 'league-beta.json'])
  assert.deepEqual(calls[2][1].body.parents, ['stable-commit'])
  assert.deepEqual(calls[3], ['git/refs/heads/updater', { method: 'PATCH', body: { sha: 'new-commit', force: false } }])
})

test('promotes each known beta predecessor without accepting another product or channel', () => {
  for (const [index, beta] of approvedBetas.entries()) {
    const product = index === 0 ? 'maxgamestudio' : 'maxgamestudio-league'
    const previousVersion = index === 0 ? '3.2.0-beta.1' : '1.0.0-beta.1'
    const prefix = index === 0 ? 'v' : 'league-v'
    const old = { product, channel: 'beta', version: previousVersion, tag: prefix + previousVersion }
    assert.doesNotThrow(() => validatePreviousBeta(old, beta))
    assert.doesNotThrow(() => validatePreviousBeta({ ...old, version: beta.version, tag: beta.tag }, beta))
    assert.throws(() => validatePreviousBeta({ ...old, product: index === 0 ? 'maxgamestudio-league' : 'maxgamestudio' }, beta))
    assert.throws(() => validatePreviousBeta({ ...old, channel: 'stable' }, beta))
    assert.throws(() => validatePreviousBeta({ ...old, tag: prefix + '0.0.1-beta.1' }, beta))
    assert.throws(() => validatePreviousBeta({ ...old, version: '9.0.0-beta.1', tag: prefix + '9.0.0-beta.1' }, beta))
  }
})
