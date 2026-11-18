const repository = 'INEEDBUG/MaxGameStudio'
const product = 'maxgamestudio-league'
const versionPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-beta\.(0|[1-9]\d*))?$/

function parseVersion(value: string) {
  const match = versionPattern.exec(value)
  if (!match) throw new Error('Invalid League version')
  const numbers = match.slice(1, 4).map(Number)
  const beta = match[4] === undefined ? null : Number(match[4])
  if (!numbers.every(Number.isSafeInteger) || (beta !== null && !Number.isSafeInteger(beta))) {
    throw new Error('Invalid League version')
  }
  return { numbers, beta }
}

export function leagueUpdateFeed(current: string): string {
  const channel = parseVersion(current).beta === null ? 'latest' : 'beta'
  return `https://raw.githubusercontent.com/${repository}/updater/league-${channel}.json`
}

export function leagueUpdate(info: unknown, current: string) {
  if (!info || typeof info !== 'object') throw new Error('Invalid League update manifest')
  const { product: manifestProduct, channel, version, tag, url } = info as Record<string, unknown>
  const installed = parseVersion(current)
  const installedChannel = installed.beta === null ? 'stable' : 'beta'
  if (
    manifestProduct !== product ||
    (channel !== 'beta' && channel !== 'stable') ||
    typeof version !== 'string' ||
    tag !== `league-v${version}` ||
    url !== `https://github.com/${repository}/releases/download/${tag}/MaxGameStudioLeague_${version}_x64-setup.exe`
  ) {
    throw new Error('Invalid League update manifest')
  }
  const remote = parseVersion(version)
  if (channel !== (remote.beta === null ? 'stable' : 'beta')) {
    throw new Error('Invalid League update channel')
  }
  if (channel !== installedChannel) {
    if (installedChannel === 'stable') return null
    throw new Error('Invalid League update channel')
  }
  const difference = remote.numbers
    .map((value, index) => value - installed.numbers[index])
    .find((value) => value !== 0)
  return (difference ?? (remote.beta ?? 0) - (installed.beta ?? 0)) > 0
    ? { version, url: url as string }
    : null
}
