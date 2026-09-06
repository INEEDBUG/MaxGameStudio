import type { ChampSelectSession } from '@shared/types/league-client/champ-select'
import type { GameflowSession } from '@shared/types/league-client/gameflow'
import { describe, expect, it } from 'vitest'

import { isResgPage, selectResgChampion } from './resg-policy'

const session = (mode = 'KIWI') =>
  ({
    gameData: {
      queue: { gameMode: mode },
      playerChampionSelections: [{ puuid: 'self', championId: 223 }],
      teamOne: [],
      teamTwo: []
    }
  }) as unknown as GameflowSession
const selection = {
  localPlayerCellId: 2,
  myTeam: [{ cellId: 2, championId: 63 }],
  actions: [[{ actorCellId: 2, type: 'pick', championId: 17 }]]
} as unknown as ChampSelectSession

describe('RESG only follows the local Hextech ARAM champion', () => {
  it('does not guess the player from an action when the local team entry is absent', () => {
    expect(selectResgChampion('ChampSelect', session(), { ...selection, myTeam: [] }, 'self')).toBe(
      0
    )
  })
  it('uses the current bench swap, not the stale pick action', () => {
    expect(selectResgChampion('ChampSelect', session(), selection, 'self')).toBe(63)
  })
  it.each(['ARAM', 'CLASSIC', 'CHERRY', 'URF'])('does not open for %s', (mode) => {
    expect(selectResgChampion('ChampSelect', session(mode), selection, 'self')).toBe(0)
  })
  it.each(['None', 'Lobby', 'Matchmaking', 'EndOfGame', 'WatchInProgress'])(
    'does not open during %s',
    (phase) => {
      expect(selectResgChampion(phase, session(), selection, 'self')).toBe(0)
    }
  )
  it('has no stale hero after disconnect or before selection', () => {
    expect(selectResgChampion('ChampSelect', null, selection, 'self')).toBe(0)
    expect(selectResgChampion('ChampSelect', session(), null, 'self')).toBe(0)
  })
  it.each(['GameStart', 'InProgress', 'Reconnect'])('follows local game data in %s', (phase) => {
    expect(selectResgChampion(phase, session(), null, 'self')).toBe(223)
    expect(selectResgChampion(phase, session(), null, 'other')).toBe(0)
  })
})

describe('RESG navigation isolation', () => {
  it.each([
    'https://www.resg.top/',
    'https://www.bilibili.com/toy/resg/index.html',
    'https://www.bilibilitoy.com/toy/resg/new-version/v/16.17/champions/63'
  ])('allows expected host route %s', (url) => {
    expect(isResgPage(url)).toBe(true)
  })
  it.each([
    'http://www.resg.top/',
    'https://www.resg.top.evil.test/',
    'file:///D:/private',
    'https://www.bilibili.com/account',
    'https://user:pass@www.resg.top/',
    'javascript:alert(1)',
    'https://localhost:2999/',
    'https://www.resg.top:8443/'
  ])('rejects %s', (url) => {
    expect(isResgPage(url)).toBe(false)
  })
})
