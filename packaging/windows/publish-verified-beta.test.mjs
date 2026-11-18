import assert from 'node:assert/strict'
import test from 'node:test'
import { approvedBetas, publishFeeds, validateRecord, verifyRelease } from './publish-verified-beta.mjs'

const sha256 = 'a'.repeat(64)
const accepted = {
  sourceSha: 'f'.repeat(40),
  verifyRunId: '100',
  candidateRunIds: ['101', '102'],
  sha256: Object.fromEntries(approvedBetas.flatMap(({ installer, candidateSidecar }) =>
    [[installer, sha256], [installer + candidateSidecar, sha256]]
  ))
}

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
