import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, readFile, stat, statfs, writeFile, appendFile } from 'node:fs/promises'
import { join, resolve, sep } from 'node:path'
import { pathToFileURL } from 'node:url'
import { mainInstallerNames } from '../../frontend/src/utils/mainUpdateIdentity.mjs'

export const repository = 'INEEDBUG/MaxGameStudio'
const limits = { beta: 1, stable: 3 }
const beta = /^(league-)?[vV](0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-beta\.(0|[1-9]\d*))?$/
const digest = /^sha256:([a-f0-9]{64})$/
const feedNames = ['latest.json', 'beta.json', 'league-latest.json', 'league-beta.json']
const readmeNames = ['README.md', 'README.zh-CN.md']

function identify(release) {
  if (release.draft) return null
  const match = beta.exec(release.tag_name)
  if (!match || Boolean(match[5]) !== release.prerelease) return null
  if (!Number.isSafeInteger(release.id) || release.id <= 0) throw new Error('Invalid release ID.')
  const version = match.slice(2, 5).join('.') + (match[5] ? '-beta.' + match[5] : '')
  const product = match[1] ? 'league' : 'main'
  const names = product === 'main' ? mainInstallerNames(version) : [`MaxGameStudioLeague_${version}_x64-setup.exe`]
  const installers = release.assets.filter(asset => names.includes(asset.name))
  if (installers.length !== 1) throw new Error(`Installer identity is missing or ambiguous: ${release.tag_name}`)
  return { release, version, product, channel: match[5] ? 'beta' : 'stable',
    parts: match.slice(2, 6).map(value => BigInt(value ?? 0)), installer: installers[0] }
}

export function planRetention(releases, protectedTags = []) {
  const buckets = new Map(), keep = [], remove = []
  for (const release of releases) {
    const info = identify(release)
    if (!info) continue
    const key = info.product + ':' + info.channel
    if (!buckets.has(key)) buckets.set(key, [])
    buckets.get(key).push(info)
  }
  for (const rows of buckets.values()) {
    rows.sort((a, b) => {
      for (let i = 0; i < 4; i++) if (a.parts[i] !== b.parts[i]) return a.parts[i] > b.parts[i] ? -1 : 1
      return 0
    })
    const retain = rows.slice(0, limits[rows[0].channel]), retire = rows.slice(retain.length)
    if (retire.some(info => protectedTags.includes(info.release.tag_name))) {
      throw new Error('An active update target is outside the retention window; no releases may be deleted.')
    }
    keep.push(...retain.map(info => info.release))
    remove.push(...retire.map(info => info.release))
  }
  return { keep, remove }
}

function assertAsset(asset, release) {
  if (!Number.isSafeInteger(asset.id) || !Number.isSafeInteger(asset.size) || asset.size <= 0 ||
      asset.state !== 'uploaded' || !digest.test(asset.digest ?? '') ||
      !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(asset.name) ||
      asset.browser_download_url !== `https://github.com/${repository}/releases/download/${release.tag_name}/${asset.name}`) {
    throw new Error(`Unverified release asset: ${release.tag_name}`)
  }
}

export function inspectFeeds(snapshot) {
  const protectedTags = [], targets = {}
  for (const [name, file] of Object.entries(snapshot.feeds)) {
    if (!file) continue
    const feed = JSON.parse(Buffer.from(file.content, 'base64').toString('utf8'))
    const product = name.startsWith('league-') ? 'league' : 'main'
    const channel = name.includes('beta') ? 'beta' : 'stable'
    const tag = (product === 'league' ? 'league-v' : 'v') + feed.version
    const release = snapshot.releases.find(row => row.tag_name === tag)
    const info = release && identify(release)
    if (!info || info.product !== product || info.channel !== channel ||
        (feed.product !== undefined && feed.product !== (product === 'main' ? 'maxgamestudio' : 'maxgamestudio-league')) ||
        (feed.channel !== undefined && feed.channel !== channel) ||
        (feed.tag_name !== undefined && feed.tag_name !== tag) || (feed.tag !== undefined && feed.tag !== tag)) {
      throw new Error(`Updater identity does not match a public release: ${name}`)
    }
    assertAsset(info.installer, release)
    if (product === 'league') {
      if (feed.url !== info.installer.browser_download_url || feed.sha256 !== digest.exec(info.installer.digest)[1]) {
        throw new Error(`Updater checksum does not match its installer: ${name}`)
      }
    } else {
      const platforms = feed.platforms ? Object.values(feed.platforms) : [feed]
      if (!platforms.length || platforms.some(platform => platform.url !== info.installer.browser_download_url ||
          typeof platform.signature !== 'string' || !platform.signature.trim())) {
        throw new Error(`Signed updater target does not match its installer: ${name}`)
      }
    }
    protectedTags.push(tag)
    const checksum = release.assets.find(asset => asset.name === info.installer.name + '.sha256' || asset.name === 'SHA256SUMS')
    if (checksum) assertAsset(checksum, release)
    targets[product + (channel === 'beta' ? 'Beta' : 'Stable')] = { version: info.version, tag,
      installer: info.installer, checksum }
  }
  if (!targets.mainStable || !targets.mainBeta || !targets.leagueBeta ||
      snapshot.latest.tag_name !== targets.mainStable.tag) throw new Error('Current independent updater feeds and global stable release must remain available.')
  return { protectedTags, targets }
}

function gh(args, input) {
  const child = spawnSync('gh', args, { encoding: 'utf8', input })
  if (child.error || child.status !== 0) throw new Error(`GitHub operation failed: ${args[0]}`)
  return child.stdout
}

function githubApi(path, { method = 'GET', body, optional = false, paginate = false } = {}) {
  const args = ['api', '--method', method, `repos/${repository}/${path}`]
  if (paginate) args.push('--paginate', '--slurp')
  if (body !== undefined) args.push('--input', '-')
  const child = spawnSync('gh', args, { encoding: 'utf8', input: body === undefined ? undefined : JSON.stringify(body) })
  if (child.error || child.status !== 0) {
    if (optional && /\(HTTP 404\)/.test(child.stderr)) return null
    throw new Error(`GitHub ${method} ${path.split('?')[0]} failed.`)
  }
  const value = child.stdout.trim() ? JSON.parse(child.stdout) : null
  return paginate ? value.flat() : value
}

export function readSnapshot(api) {
  const main = api('git/ref/heads/main').object.sha
  const updater = api('git/ref/heads/updater').object.sha
  return { releases: api('releases?per_page=100', { paginate: true }), latest: api('releases/latest'),
    feeds: Object.fromEntries(feedNames.map(name => [name, api(`contents/${name}?ref=${updater}`, { optional: true })])),
    readmes: Object.fromEntries(readmeNames.map(name => [name, api(`contents/${name}?ref=${main}`)])),
    main, tree: api('git/commits/' + main).tree.sha }
}

function identity(release) {
  return { id: release.id, tag_name: release.tag_name, draft: release.draft, prerelease: release.prerelease,
    name: release.name, body: release.body, assets: release.assets.map(asset => ({ id: asset.id, name: asset.name,
      size: asset.size, state: asset.state, digest: asset.digest, url: asset.browser_download_url })).sort((a, b) => a.id - b.id) }
}

function fingerprint(snapshot) {
  return JSON.stringify({ releases: snapshot.releases.map(identity).sort((a, b) => a.id - b.id),
    feeds: feedNames.map(name => snapshot.feeds[name]?.sha ?? null), latest: snapshot.latest.id })
}

async function hashFile(path) {
  const hash = createHash('sha256')
  for await (const bytes of createReadStream(path)) hash.update(bytes)
  return hash.digest('hex')
}

function assertPreservedTag(api, release, archived) {
  const current = api('git/ref/tags/' + release.tag_name)
  const expectedRef = 'refs/tags/' + release.tag_name
  if (archived?.ref !== expectedRef || !['commit', 'tag'].includes(archived.object?.type) ||
      !/^[a-f0-9]{40}$/.test(archived.object?.sha ?? '') || current?.ref !== expectedRef ||
      current.object?.type !== archived.object.type || current.object?.sha !== archived.object.sha) {
    throw new Error('Source tag changed after the recovery backup: ' + release.tag_name)
  }
}

export async function backupReleases(plan, directory, api, download = (release, asset, folder) =>
  gh(['release', 'download', release.tag_name, '--repo', repository, '--pattern', asset.name, '--dir', folder])) {
  const total = plan.remove.flatMap(row => row.assets).reduce((sum, asset) => sum + asset.size, 0)
  if (total > 2 * 1024 ** 3) throw new Error('Recovery backup exceeds the 2 GiB maintenance budget.')
  await mkdir(directory, { recursive: true })
  const space = await statfs(directory, { bigint: true })
  if (space.bavail * space.bsize < BigInt(total) + 256n * 1024n ** 2n) throw new Error('Not enough space for the recovery backup.')
  const records = []
  for (const release of plan.remove) {
    const folder = join(directory, String(release.id))
    await mkdir(folder)
    for (const asset of release.assets) {
      assertAsset(asset, release)
      await download(release, asset, folder)
      const file = join(folder, asset.name)
      if ((await stat(file)).size !== asset.size || await hashFile(file) !== digest.exec(asset.digest)[1]) {
        throw new Error(`Recovery asset checksum mismatch: ${release.tag_name}`)
      }
    }
    records.push({ release, tag: api('git/ref/tags/' + release.tag_name) })
  }
  await writeFile(join(directory, 'releases.json'), JSON.stringify(records, null, 2) + '\n')
}

async function updateReadmes(api, snapshot, targets) {
  const { rewriteReleaseReadme } = await import('./release-readme.mjs')
  const updates = []
  for (const path of readmeNames) {
    const original = Buffer.from(snapshot.readmes[path].content, 'base64').toString('utf8')
    const content = rewriteReleaseReadme(original, targets)
    if (content !== original) updates.push({ path, mode: '100644', type: 'blob', content })
  }
  if (!updates.length) return
  const tree = api('git/trees', { method: 'POST', body: { base_tree: snapshot.tree, tree: updates } })
  const commit = api('git/commits', { method: 'POST', body: { message: 'Refresh retained release downloads [skip ci]',
    tree: tree.sha, parents: [snapshot.main] } })
  api('git/refs/heads/main', { method: 'PATCH', body: { sha: commit.sha, force: false } })
  for (const file of updates) {
    const saved = api(`contents/${file.path}?ref=main`)
    if (Buffer.from(saved.content, 'base64').toString('utf8') !== file.content) throw new Error('README download readback failed.')
  }
}

export async function applyRetention(api, plan, directory, snapshotReader = readSnapshot, updateDocs = updateReadmes) {
  const records = JSON.parse(await readFile(join(directory, 'releases.json'), 'utf8'))
  for (const release of plan.remove) {
    if (!records.some(record => JSON.stringify(identity(record.release)) === JSON.stringify(identity(release)))) {
      throw new Error('Recovery archive does not match the planned release.')
    }
    for (const asset of release.assets) {
      assertAsset(asset, release)
      if (await hashFile(join(directory, String(release.id), asset.name)) !== digest.exec(asset.digest)[1]) throw new Error('Recovery backup changed.')
    }
  }
  let fresh = snapshotReader(api)
  if (fingerprint(fresh) !== fingerprint(plan.snapshot)) throw new Error('Release or updater state changed after planning.')
  const current = planRetention(fresh.releases, inspectFeeds(fresh).protectedTags)
  const ordered = rows => rows.map(identity).sort((a, b) => a.id - b.id)
  if (JSON.stringify(ordered(current.remove)) !== JSON.stringify(ordered(plan.remove))) {
    throw new Error('Deletion plan does not match the active retention policy.')
  }
  for (const path of readmeNames) {
    if (fresh.readmes[path].sha !== plan.snapshot.readmes[path].sha) throw new Error('README changed after planning.')
  }
  await updateDocs(api, fresh, inspectFeeds(fresh).targets)
  const removed = new Set()
  for (const release of plan.remove) {
    fresh = snapshotReader(api)
    const expected = { ...plan.snapshot, releases: plan.snapshot.releases.filter(row => !removed.has(row.id)) }
    if (fingerprint(fresh) !== fingerprint(expected)) throw new Error('Release or updater state changed during cleanup.')
    inspectFeeds(fresh)
    const archivedTag = records.find(record => record.release.id === release.id).tag
    assertPreservedTag(api, release, archivedTag)
    api('releases/' + release.id, { method: 'DELETE' })
    if (api('releases/' + release.id, { optional: true }) !== null) throw new Error('Deleted release is still present.')
    assertPreservedTag(api, release, archivedTag)
    removed.add(release.id)
  }
  fresh = snapshotReader(api)
  const active = inspectFeeds(fresh)
  if (planRetention(fresh.releases, active.protectedTags).remove.length) throw new Error('Retention count readback failed.')
  return [...removed]
}

export function hasPublishedUpdate(api, runId) {
  if (!/^\d+$/.test(String(runId))) throw new Error('Invalid upstream workflow run.')
  const run = api('actions/runs/' + runId)
  const routes = {
    '.github/workflows/release-windows.yml': { event: 'push', job: 'release',
      validRef: ref => { const match = beta.exec(ref); return match && !match[1] },
      steps: ['Create GitHub Release', 'Publish updater channel'] },
    '.github/workflows/release-league-windows.yml': { event: 'push', job: 'publish',
      validRef: ref => /^league-v/.test(ref) && beta.test(ref),
      steps: ['Publish and verify the League-only release and feed'] },
    '.github/workflows/verify-windows-upgrade.yml': { event: 'workflow_dispatch', job: 'publish',
      validRef: ref => ref === 'codex/all-users-installers', steps: ['Publish the verified beta pair'] },
  }
  const route = routes[run.path]
  if (!route || run.conclusion !== 'success' || run.head_repository?.full_name !== repository ||
      run.event !== route.event || !route.validRef(run.head_branch ?? '')) return false
  const jobs = api(`actions/runs/${runId}/jobs?per_page=100`).jobs
  return jobs.some(job => job.name === route.job && job.conclusion === 'success' &&
    route.steps.every(name => job.steps?.some(step => step.name === name && step.conclusion === 'success')))
}

async function main() {
  if (process.env.GITHUB_ACTIONS !== 'true' || process.env.RUNNER_ENVIRONMENT !== 'github-hosted' ||
      process.env.GITHUB_REPOSITORY !== repository || process.env.GITHUB_REF_NAME !== 'main' || !process.env.GH_TOKEN) {
    throw new Error('Release maintenance is restricted to the trusted default-branch hosted workflow.')
  }
  const [command, planPath, backupPath] = process.argv.slice(2)
  const temporary = resolve(process.env.RUNNER_TEMP) + sep
  if (![planPath, backupPath].every(path => resolve(path).startsWith(temporary))) throw new Error('Maintenance paths must stay in runner temporary storage.')
  if (command === 'plan') {
    const allowed = !process.env.UPSTREAM_RUN_ID || hasPublishedUpdate(githubApi, process.env.UPSTREAM_RUN_ID)
    const snapshot = readSnapshot(githubApi)
    const active = inspectFeeds(snapshot)
    const plan = { repository, snapshot, ...planRetention(snapshot.releases, active.protectedTags) }
    await writeFile(planPath, JSON.stringify(plan, null, 2) + '\n')
    if (process.env.GITHUB_OUTPUT) await appendFile(process.env.GITHUB_OUTPUT, `allowed=${allowed}\n`)
    console.log(JSON.stringify({ allowed, keep: plan.keep.map(row => row.tag_name), delete: plan.remove.map(row => row.tag_name),
      recoveryBytes: plan.remove.flatMap(row => row.assets).reduce((sum, asset) => sum + asset.size, 0) }))
  } else {
    const plan = JSON.parse(await readFile(planPath, 'utf8'))
    if (plan.repository !== repository) throw new Error('Unexpected release plan repository.')
    if (command === 'backup') await backupReleases(plan, backupPath, githubApi)
    else if (command === 'apply') console.log('Deleted release IDs: ' + JSON.stringify(await applyRetention(githubApi, plan, backupPath)))
    else throw new Error('Expected plan, backup or apply command.')
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { await main() } catch (error) { console.error(error.message); process.exitCode = 1 }
}
