import { leagueUpdate, leagueUpdateFeed } from '../../../packaging/windows/league-standalone/league-update.ts'
import { companionWindowDemand } from '../../../packaging/windows/league-standalone/companion-window-demand.ts'

// Record only the deterministic functions. Run this file with Node 24 and redirect stdout to typescript-behavior.json.
function capture(call) {
  try {
    return { value: call() }
  } catch (error) {
    return { error: error.message }
  }
}

function manifest(version) {
  const tag = `league-v${version}`
  return {
    product: 'maxgamestudio-league',
    channel: version.includes('-beta.') ? 'beta' : 'stable',
    version,
    tag,
    url: `https://github.com/INEEDBUG/MaxGameStudio/releases/download/${tag}/MaxGameStudioLeague_${version}_x64-setup.exe`
  }
}

const beta = manifest('1.0.0-beta.2')
const stable = manifest('1.0.0')
const maxSafe = '9007199254740991'
const unsafe = '9007199254740992'

const feeds = [
  '1.0.0-beta.1', '1.0.0', '0.0.0-beta.0', `${maxSafe}.0.0`, `1.0.0-beta.${maxSafe}`,
  `${unsafe}.0.0`, `1.0.0-beta.${unsafe}`, '1.0.0-beta.00', '01.0.0', '1.0',
  '1.0.0-rc.1', '1.0.0+build', '1.0.0\n', '1.0.０', ' 1.0.0', ''
].map(current => ({ current, expected: capture(() => leagueUpdateFeed(current)) }))

const updates = [
  ['newer beta', beta, '1.0.0-beta.1'],
  ['same beta', beta, '1.0.0-beta.2'],
  ['older beta', beta, '1.0.0-beta.3'],
  ['stable ignores beta', beta, '1.0.0'],
  ['beta rejects stable', stable, '1.0.0-beta.1'],
  ['newer stable', stable, '0.9.9'],
  ['same stable', stable, '1.0.0'],
  ['older stable', stable, '1.0.1'],
  ['beta sees beta component numerically', manifest('1.0.0-beta.10'), '1.0.0-beta.9'],
  ['new patch precedes beta component', manifest('1.0.1-beta.0'), `1.0.0-beta.${maxSafe}`],
  ['older minor precedes patch', manifest(`1.1.${maxSafe}`), '1.2.0'],
  ['safe upper bound patch', manifest(`1.0.${maxSafe}`), '1.0.9'],
  ['unsafe remote minor', manifest(`1.${unsafe}.0`), '1.0.0'],
  ['unsafe remote beta', manifest(`1.0.0-beta.${unsafe}`), '1.0.0-beta.1'],
  ['other product', { ...beta, product: 'maxgamestudio' }, '1.0.0-beta.1'],
  ['wrong channel', { ...beta, channel: 'stable' }, '1.0.0-beta.1'],
  ['unknown channel', { ...beta, channel: 'nightly' }, '1.0.0-beta.1'],
  ['unscoped tag', { ...beta, tag: 'v1.0.0-beta.2' }, '1.0.0-beta.1'],
  ['other product file', { ...beta, url: beta.url.replace('MaxGameStudioLeague_', 'MaxGameStudio_') }, '1.0.0-beta.1'],
  ['redirect host', { ...beta, url: beta.url.replace('github.com', 'github.com.example.org') }, '1.0.0-beta.1'],
  ['extra query', { ...beta, url: beta.url + '?download=1' }, '1.0.0-beta.1'],
  ['malformed version and mismatched tag', { ...beta, version: '01.0.0-beta.2' }, '1.0.0-beta.1'],
  ['malformed version with matching tag', manifest('01.0.0-beta.2'), '1.0.0-beta.1'],
  ['missing fields', {}, '1.0.0'],
  ['wrong version type', { ...beta, version: 1 }, '1.0.0-beta.1'],
  ['extra release metadata', { ...stable, sha256: 'unused' }, '0.9.9'],
  ['malformed installed version before field checks', {}, 'bad'],
  ['arrays reach installed version check', [], 'bad'],
  ['primitive fails before installed version check', null, 'bad'],
  ['string manifest', 'not an object', '1.0.0']
].map(([name, manifest, current]) => ({ name, manifest, current, expected: capture(() => leagueUpdate(manifest, current)) }))

const base = { enabled: true, autoShow: true, initialized: true, connected: true, ready: false, phase: 'ChampSelect' }
const states = [
  ...['enabled', 'autoShow', 'initialized', 'connected'].map(key => [key + ' false', { [key]: false, ready: true }]),
  ...['ChampSelect', 'Lobby', 'Matchmaking', 'ReadyCheck', 'None'].flatMap(phase => [
    [phase + ' before ready', { phase }],
    [phase + ' after ready', { phase, ready: true }]
  ]),
  ['spectating', { spectating: true, ready: true }],
  ['not spectating', { spectating: false }],
  ['unknown phase', { phase: 'FuturePhase' }],
  ['missing phase', { phase: undefined }],
  ['null phase', { phase: null }]
]
const windowDemands = ['aux', 'opgg'].flatMap(kind => states.map(([name, overrides]) => {
  const state = { ...base, ...overrides }
  return { kind, name, state, expected: companionWindowDemand(kind, state) }
}))

const sections = Object.entries({ feeds, updates, windowDemands }).map(([name, rows]) =>
  `  "${name}": [\n${rows.map(row => '    ' + JSON.stringify(row)).join(',\n')}\n  ]`
)
console.log(`{\n${sections.join(',\n')}\n}`)
