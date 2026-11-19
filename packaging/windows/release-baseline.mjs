import { pathToFileURL } from 'node:url'

import {
  MAIN_UPDATE_PRODUCT, mainInstallerNames, mainUpdateChannel,
} from '../../frontend/src/utils/mainUpdateIdentity.mjs'

const repository = 'INEEDBUG/MaxGameStudio'
const repair314Sha256 = '583201413a2b31d94f3009766a22dfa6cda65ddb6eba3847d3895eefdf863c1e'
const sha256Pattern = /^sha256:([a-f0-9]{64})$/
const selection = {
  'main-stable': { feed: 'latest.json', channel: 'stable' },
  'league-beta': { feed: 'league-beta.json', channel: 'beta' },
}

export function decodeReleaseBaselineFeed(file) {
  if (file?.encoding !== 'base64' || typeof file.content !== 'string') {
    throw new Error('Expected an updater feed from the GitHub Contents API.')
  }
  return JSON.parse(Buffer.from(file.content, 'base64').toString('utf8'))
}

function feedIdentity(kind, manifest) {
  const source = selection[kind]
  if (!Object.hasOwn(selection, kind) || mainUpdateChannel(manifest?.version) !== source.channel) {
    throw new Error('Unexpected updater baseline channel or version.')
  }

  const { version } = manifest
  const league = kind === 'league-beta'
  const tag = (league ? 'league-v' : 'v') + version
  const name = league ? 'MaxGameStudioLeague_' + version + '_x64-setup.exe' : mainInstallerNames(version)[0]
  const url = 'https://github.com/' + repository + '/releases/download/' + tag + '/' + name
  if (league) {
    if (manifest.product !== 'maxgamestudio-league' || manifest.channel !== 'beta' ||
        manifest.tag !== tag || manifest.url !== url || !/^[a-f0-9]{64}$/.test(manifest.sha256)) {
      throw new Error('Unexpected League beta feed identity.')
    }
  } else {
    if ((manifest.product !== undefined && manifest.product !== MAIN_UPDATE_PRODUCT) ||
        (manifest.channel !== undefined && manifest.channel !== 'stable') ||
        (manifest.tag_name !== undefined && manifest.tag_name !== tag)) {
      throw new Error('Unexpected Main stable feed identity.')
    }
    const windows = manifest.platforms == null ? [manifest] :
      [manifest.platforms['windows-x86_64-nsis'], manifest.platforms['windows-x86_64']].filter(Boolean)
    if (!windows.length || windows.some((platform) =>
      platform.url !== url || typeof platform.signature !== 'string' || !platform.signature.trim())) {
      throw new Error('Unexpected installer in Main stable feed platform metadata.')
    }
  }
  return { version, tag, name, url }
}

export function selectReleaseBaseline(kind, manifest, release) {
  const identity = feedIdentity(kind, manifest)
  const league = kind === 'league-beta'
  if (release?.tag_name !== identity.tag || release.draft !== false || release.prerelease !== league) {
    throw new Error('Published Release does not match the updater baseline.')
  }

  const assets = release.assets?.filter((asset) => asset.name === identity.name) ?? []
  const [asset] = assets
  const sha256 = sha256Pattern.exec(asset?.digest)?.[1]
  if (assets.length !== 1 || asset.state !== 'uploaded' || asset.browser_download_url !== identity.url ||
      !Number.isSafeInteger(asset.size) || asset.size <= 0 || !sha256 ||
      (league && manifest.sha256 !== sha256)) {
    throw new Error('Published baseline installer metadata does not match the updater feed.')
  }
  if (kind === 'main-stable' && identity.version === '3.1.4' && sha256 !== repair314Sha256) {
    throw new Error('The 3.1.4 repair installer does not match its verified SHA-256.')
  }
  return { ...identity, size: asset.size, sha256 }
}

async function readGitHub(path) {
  const headers = { Accept: 'application/vnd.github+json', 'User-Agent': 'MaxGameStudio-upgrade-verifier' }
  if (process.env.GH_TOKEN) headers.Authorization = 'Bearer ' + process.env.GH_TOKEN
  const response = await fetch('https://api.github.com/repos/' + repository + '/' + path, {
    headers, redirect: 'error', signal: AbortSignal.timeout(15000),
  })
  if (!response.ok) throw new Error('GitHub baseline metadata request failed (' + response.status + ').')
  return response.json()
}

export async function resolveReleaseBaseline(kind, read = readGitHub) {
  if (!Object.hasOwn(selection, kind)) throw new Error('Expected baseline kind main-stable or league-beta.')
  const feed = decodeReleaseBaselineFeed(await read('contents/' + selection[kind].feed + '?ref=updater'))
  const { tag } = feedIdentity(kind, feed)
  const release = await read(kind === 'main-stable' ? 'releases/latest' : 'releases/tags/' + tag)
  return selectReleaseBaseline(kind, feed, release)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    console.log(JSON.stringify(await resolveReleaseBaseline(process.argv[2])))
  } catch (error) {
    console.error(error.message)
    process.exitCode = 1
  }
}
