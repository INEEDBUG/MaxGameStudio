import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createReadStream, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { createGithubUpdaterManifest, readUpdaterSignature } from '../../frontend/scripts/updater-manifest.mjs'

const repository = 'INEEDBUG/MaxGameStudio'
const repoApi = `repos/${repository}`
export const approvedBetas = [
  {
    version: '3.2.0-beta.2',
    previousVersion: '3.2.0-beta.1',
    tag: 'v3.2.0-beta.2',
    installer: 'MaxGameStudio_3.2.0-beta.2_x64-setup.exe',
    candidateSidecar: '.sig',
    title: 'MaxGameStudio 3.2.0-beta.2',
    feed: 'beta.json',
  },
  {
    version: '1.0.0-beta.2',
    previousVersion: '1.0.0-beta.1',
    tag: 'league-v1.0.0-beta.2',
    installer: 'MaxGameStudioLeague_1.0.0-beta.2_x64-setup.exe',
    candidateSidecar: '.sha256',
    title: 'MaxGameStudio League 1.0.0-beta.2',
    feed: 'league-beta.json',
  },
]
const candidateFiles = approvedBetas.flatMap(({ installer, candidateSidecar }) => [
  installer, installer + candidateSidecar,
])
const verificationFile = 'verified-beta.json'
const shaPattern = /^[a-f0-9]{64}$/
const assetUrl = (beta, name) => `https://github.com/${repository}/releases/download/${beta.tag}/${name}`

export function validateRecord(record, runId) {
  if (!record || !/^\d+$/.test(runId ?? '') || record.verifyRunId !== runId ||
      !/^[a-f0-9]{40}$/.test(record.sourceSha ?? '') ||
      !Array.isArray(record.candidateRunIds) || record.candidateRunIds.length !== 2 ||
      !record.candidateRunIds.every((id) => typeof id === 'string' && /^\d+$/.test(id)) ||
      Object.keys(record.sha256 ?? {}).length !== candidateFiles.length ||
      !candidateFiles.every((name) => shaPattern.test(record.sha256?.[name] ?? ''))) {
    throw new Error('Invalid verified beta record for this workflow run.')
  }
}

export function verifyRelease(release, beta, expected) {
  if (release.tag_name !== beta.tag || release.draft !== false || release.prerelease !== true ||
      release.assets?.length !== expected.length || !expected.every((file) =>
        release.assets.some((asset) => asset.name === file.name && asset.state === 'uploaded' &&
          asset.size === file.size && asset.digest === `sha256:${file.sha256}` &&
          asset.browser_download_url === assetUrl(beta, file.name)))) {
    throw new Error(`Release ${beta.tag} does not match the verified prerelease assets.`)
  }
}

async function assetDetails(path) {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return { path, name: basename(path), size: statSync(path).size, sha256: hash.digest('hex') }
}

function gh(args, body, optional = false) {
  const child = spawnSync('gh', args, {
    encoding: 'utf8',
    input: body === undefined ? undefined : JSON.stringify(body),
  })
  if (child.error) throw child.error
  if (child.status !== 0) {
    if (optional && /\(HTTP 404\)/.test(child.stderr)) return null
    throw new Error(`GitHub ${args[0]} failed: ${child.stderr.trim() || child.status}`)
  }
  return child.stdout
}

function api(path, { method = 'GET', body, optional = false } = {}) {
  const args = ['api', '--method', method, `${repoApi}/${path}`]
  if (body !== undefined) args.push('--input', '-')
  const output = gh(args, body, optional)
  return output === null ? null : JSON.parse(output)
}

function existingTag(beta, sourceSha) {
  const ref = api(`git/ref/tags/${beta.tag}`, { optional: true })
  if (!ref) return false
  const target = ref.object.type === 'tag' ? api(`git/tags/${ref.object.sha}`).object : ref.object
  if (target.type !== 'commit' || target.sha !== sourceSha) {
    throw new Error(`Tag ${beta.tag} already points to another commit.`)
  }
  return true
}

function stableState() {
  const latest = api('releases/latest')
  if (latest.tag_name !== 'v3.1.4') throw new Error('The expected stable Release is no longer latest.')
  return [
    latest.id,
    api('contents/latest.json?ref=updater').sha,
    api('contents/league-latest.json?ref=updater', { optional: true })?.sha ?? null,
  ]
}

export function validatePreviousBeta(value, beta) {
  const product = beta.candidateSidecar === '.sig' ? 'maxgamestudio' : 'maxgamestudio-league'
  const prefix = beta.candidateSidecar === '.sig' ? 'v' : 'league-v'
  if (value.product !== product || value.channel !== 'beta' ||
      ![beta.previousVersion, beta.version].includes(value.version) ||
      (value.tag_name ?? value.tag) !== prefix + value.version) {
    throw new Error(`Refusing to replace a different or unrecognized beta feed: ${beta.feed}.`)
  }
}

function betaManifest(beta, base) {
  const file = api(`contents/${beta.feed}?ref=${base}`, { optional: true })
  if (!file) return null
  const text = Buffer.from(file.content, 'base64').toString('utf8')
  const value = JSON.parse(text)
  validatePreviousBeta(value, beta)
  return text
}

export function publishFeeds(callApi, base, files) {
  if (!files.length) return
  if (!files.every((file) => approvedBetas.some((beta) => beta.feed === file.path))) {
    throw new Error('Only the beta feed files can be changed.')
  }
  const parent = callApi(`git/commits/${base}`)
  const tree = callApi('git/trees', {
    method: 'POST',
    body: {
      base_tree: parent.tree.sha,
      tree: files.map(({ path, content }) => ({ path, content, mode: '100644', type: 'blob' })),
    },
  })
  const commit = callApi('git/commits', {
    method: 'POST',
    body: { message: 'Publish the verified Windows beta pair', tree: tree.sha, parents: [base] },
  })
  callApi('git/refs/heads/updater', { method: 'PATCH', body: { sha: commit.sha, force: false } })
}

async function publish(directory) {
  if (!directory || process.env.GITHUB_ACTIONS !== 'true' || process.env.GITHUB_EVENT_NAME !== 'workflow_dispatch' ||
      process.env.GITHUB_REPOSITORY !== repository || process.env.GITHUB_REF_NAME !== 'codex/all-users-installers' ||
      !process.env.GH_TOKEN) {
    throw new Error('Publishing is limited to the authorized verifier workflow and repository.')
  }
  const bundle = resolve(directory)
  const entries = readdirSync(bundle, { withFileTypes: true })
  if (entries.length !== candidateFiles.length + 1 || !entries.every((entry) =>
    entry.isFile() && [...candidateFiles, verificationFile].includes(entry.name))) {
    throw new Error('Unexpected files in the verified beta bundle.')
  }
  const record = JSON.parse(readFileSync(join(bundle, verificationFile), 'utf8'))
  validateRecord(record, process.env.GITHUB_RUN_ID)
  const details = new Map()
  for (const name of candidateFiles) {
    const file = await assetDetails(join(bundle, name))
    if (file.sha256 !== record.sha256[name] || !file.size) throw new Error(`Verified file changed: ${name}.`)
    details.set(name, file)
  }

  const [main, league] = approvedBetas
  const notesPath = new URL(`../../docs/releases/${main.tag}.zh-CN.md`, import.meta.url)
  const leagueNotesPath = new URL(`../../docs/releases/${league.tag}.zh-CN.md`, import.meta.url)
  const mainNotes = readFileSync(notesPath, 'utf8')
  const userReleaseNotes = JSON.parse(readFileSync(new URL(`../../docs/update-notes/${main.tag}.json`, import.meta.url), 'utf8'))
  if (userReleaseNotes.releaseStatus !== 'beta') throw new Error('Main beta release notes are required.')
  const signature = readUpdaterSignature(join(bundle, main.installer + '.sig'))
  const manifest = createGithubUpdaterManifest({
    version: main.version, repository, channel: 'beta', tag: main.tag, installerName: main.installer,
    signature, notes: mainNotes, userReleaseNotes,
  })
  const mainChecksum = join(bundle, main.installer + '.sha256')
  writeFileSync(mainChecksum, `${details.get(main.installer).sha256} *${main.installer}\n`, { flag: 'wx' })
  const leagueChecksum = details.get(league.installer + '.sha256')
  if (readFileSync(leagueChecksum.path, 'utf8').trim() !== `${details.get(league.installer).sha256} *${league.installer}`) {
    throw new Error('The verified League checksum does not match its installer.')
  }
  const filesByProduct = [
    [details.get(main.installer), await assetDetails(mainChecksum)],
    [details.get(league.installer), leagueChecksum],
  ]

  const stable = stableState()
  const base = api('git/ref/heads/updater').object.sha
  const previous = approvedBetas.map((beta) => betaManifest(beta, base))
  const tags = approvedBetas.map((beta) => existingTag(beta, record.sourceSha))
  const releases = approvedBetas.map((beta, index) => {
    const release = api(`releases/tags/${beta.tag}`, { optional: true })
    if (release) {
      if (!tags[index]) throw new Error(`Existing release is missing its tag: ${beta.tag}.`)
      verifyRelease(release, beta, filesByProduct[index])
    }
    return release
  })

  for (const [index, beta] of approvedBetas.entries()) {
    if (!tags[index]) {
      api('git/refs', { method: 'POST', body: { ref: `refs/tags/${beta.tag}`, sha: record.sourceSha } })
    }
    if (!releases[index]) {
      gh(['release', 'create', beta.tag, ...filesByProduct[index].map((file) => file.path),
        '--repo', repository, '--title', beta.title, '--latest=false', '--prerelease', '--verify-tag',
        '--notes-file', fileURLToPath(index === 0 ? notesPath : leagueNotesPath)])
    }
    releases[index] = api(`releases/tags/${beta.tag}`)
    if (!existingTag(beta, record.sourceSha)) throw new Error(`Published tag missing: ${beta.tag}.`)
    verifyRelease(releases[index], beta, filesByProduct[index])
  }
  if (JSON.stringify(stableState()) !== JSON.stringify(stable)) throw new Error('Stable state changed during release.')

  if (!Number.isFinite(Date.parse(releases[0].published_at))) throw new Error('Published Main release date is missing.')
  manifest.pub_date = releases[0].published_at
  const leagueManifest = {
    product: 'maxgamestudio-league', channel: 'beta', version: league.version, tag: league.tag,
    url: assetUrl(league, league.installer), sha256: details.get(league.installer).sha256,
  }
  const feeds = [manifest, leagueManifest].map((data, index) => ({
    path: approvedBetas[index].feed, content: `${JSON.stringify(data, null, 2)}\n`,
  }))
  publishFeeds(api, base, feeds.filter((file, index) => file.content !== previous[index]))
  for (const file of feeds) {
    const saved = api(`contents/${file.path}?ref=updater`)
    if (Buffer.from(saved.content, 'base64').toString('utf8') !== file.content) {
      throw new Error(`Beta feed readback mismatch: ${file.path}.`)
    }
  }
  if (JSON.stringify(stableState()) !== JSON.stringify(stable)) throw new Error('Stable state changed during feed promotion.')
  console.log(`Published verified beta installers from ${record.sourceSha}.`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await publish(process.argv[2])
}
