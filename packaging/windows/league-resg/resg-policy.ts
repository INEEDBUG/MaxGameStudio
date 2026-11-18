import type { ChampSelectSession } from '@shared/types/league-client/champ-select'
import type { GameflowSession } from '@shared/types/league-client/gameflow'

export const RESG_HOME = 'https://www.resg.top/'
export const RESG_NAMESPACE = 'window-manager-main/resg-window'
const activePhases = new Set(['ChampSelect', 'GameStart', 'InProgress', 'Reconnect'])

export function isResgPage(value: string): boolean {
  try {
    const url = new URL(value)
    if (url.protocol !== 'https:' || url.username || url.password || url.port) return false
    return ['resg.top', 'www.resg.top'].includes(url.hostname)
  } catch {
    return false
  }
}

export function isResgContent(value: string): boolean {
  return isResgPage(value) && /^\/v\/[^/]+\/champions\/\d+\/?$/.test(new URL(value).pathname)
}

export function resgChampionUrl(championId: number): string {
  if (!validChampion(championId)) throw new Error('Invalid champion')
  return new URL(`v/latest/champions/${championId}`, RESG_HOME).href
}

function validChampion(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0 && value < 100000
    ? value
    : 0
}

// Consume the existing LCU snapshot. Never create a second connection or forward account data.
export function selectResgChampion(
  phase: string | null,
  session: GameflowSession | null,
  champSelect: ChampSelectSession | null,
  puuid: string | undefined
): number {
  if (!phase || !activePhases.has(phase) || session?.gameData.queue.gameMode !== 'KIWI') return 0
  if (phase === 'ChampSelect') {
    if (!champSelect) return 0
    const self = champSelect.myTeam.find(
      (player) => player.cellId === champSelect.localPlayerCellId
    )
    if (!self) return 0
    // Bench swaps update myTeam; the old pick action may still name the previous champion.
    return (
      validChampion(self?.championId) ||
      validChampion(
        champSelect.actions
          .flat()
          .find(
            (action) =>
              action.actorCellId === champSelect.localPlayerCellId && action.type === 'pick'
          )?.championId
      )
    )
  }
  if (!puuid) return 0
  return (
    validChampion(
      session.gameData.playerChampionSelections.find((player) => player.puuid === puuid)?.championId
    ) ||
    validChampion(
      [...session.gameData.teamOne, ...session.gameData.teamTwo].find(
        (player) => player.puuid === puuid
      )?.championId
    )
  )
}

// The public hero component appears only after the detail data is loaded.
export function resgNavigationStep(championId: number): string {
  if (!validChampion(championId)) throw new Error('Invalid champion')
  return `(() => {
    const route = /^\\/v\\/[^/]+\\/champions\\/${championId}\\/?$/;
    if (!route.test(location.pathname)) return { kind: 'waiting' };
    const name = document.querySelector('#app .entity-hero .entity-identity h1')?.textContent?.trim();
    return name ? { kind: 'ready', name } : { kind: 'waiting' };
  })()`
}
