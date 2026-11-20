import {
  isResgContent,
  isResgPage,
  resgChampionUrl,
  selectResgChampion
} from '../../../packaging/windows/league-resg/resg-policy.ts'

const self = 'fixture-self'
const other = 'fixture-other'
const pick = (championId, overrides = {}) => ({ actorCellId: 4, type: 'pick', championId, ...overrides })
const selection = (championId, actions = []) => ({
  localPlayerCellId: 4,
  myTeam: [{ cellId: 3, championId: 150 }, { cellId: 4, championId }],
  actions: [actions]
})
const game = (gameMode, selections = [], teamOne = [], teamTwo = []) => ({
  gameData: { queue: { gameMode }, playerChampionSelections: selections, teamOne, teamTwo }
})

const cases = [
  ['current champion precedes an old pick action', 'ChampSelect', game('KIWI'), selection(22, [pick(12)]), self],
  ['pick action fills an empty current slot', 'ChampSelect', game('KIWI'), selection(0, [pick(12)]), self],
  ['pick action ignores another player and a ban', 'ChampSelect', game('KIWI'), selection(-1, [
    pick(12, {actorCellId: 3}), pick(23, {type: 'ban'}), pick(27)
  ]), self],
  ['no local member', 'ChampSelect', game('KIWI'), { ...selection(0, [pick(12)]), myTeam: [] }, self],
  ['normal ARAM uses its own provider', 'ChampSelect', game('ARAM'), selection(22), self],
  ['unrelated queue', 'ChampSelect', game('CLASSIC'), selection(22), self],
  ['lobby waits', 'Lobby', game('KIWI'), selection(22), self],
  ['missing queue', 'ChampSelect', null, selection(22), self],
  ['invalid fractional ID', 'ChampSelect', game('KIWI'), selection(27.5), self],
  ['unknown high ID', 'ChampSelect', game('KIWI'), selection(100000), self],
  ['game start uses resolved player selection', 'GameStart', game('KIWI', [{ puuid: self, championId: 73 }]), null, self],
  ['in-progress falls back to the second team', 'InProgress', game('KIWI', [{puuid: other, championId: 72}], [],
    [{puuid: self, championId: 89}]), null, self],
  ['reconnect prefers selected hero to team list', 'Reconnect', game('KIWI', [{puuid: self, championId: 84}],
    [{puuid: self, championId: 37}]), null, self],
  ['missing current player', 'InProgress', game('KIWI', [{puuid: self, championId: 73}]), null, null]
].map(([name, phase, session, champSelect, puuid]) => ({
  name, phase, session, champSelect, puuid,
  expected: selectResgChampion(phase, session, champSelect, puuid) || null
}))

const destinations = [
  'https://resg.top/',
  'https://www.resg.top/v/latest/champions/22',
  'https://www.resg.top/v/live/champions/22/?foo=bar',
  'https://www.resg.top:443/v/live/champions/22',
  'https://www.resg.top:444/v/live/champions/22',
  'https://example.net/v/latest/champions/22',
  'https://www.resg.top.example.net/v/latest/champions/22',
  'https://example.net@www.resg.top/v/latest/champions/22',
  'http://www.resg.top/v/latest/champions/22',
  'https://www.resg.top/v/latest/champions/22//',
  'https://www.resg.top//v/latest/champions/22',
  'https://www.resg.top/v/latest/champions/%32%32',
  'https://www.resg.top/v/latest/champions/٢٢',
  'not a url'
].map(url => ({ url, page: isResgPage(url), content: isResgContent(url) }))

const charts = [1, 22, 99999, 0, 100000].map(champion => {
  let expected = null
  try {
    expected = resgChampionUrl(champion)
  } catch {
    // Unrecognized champion IDs cannot be opened.
  }
  return { champion, expected }
})

console.log(JSON.stringify({ cases, destinations, charts }, null, 2))
