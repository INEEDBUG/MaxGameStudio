import type { ChampSelectSession } from '@shared/types/league-client/champ-select'
import type { GameflowSession } from '@shared/types/league-client/gameflow'

export const RESG_HOME = 'https://www.resg.top/'
export const RESG_NAMESPACE = 'window-manager-main/resg-window'
const activePhases = new Set(['ChampSelect', 'GameStart', 'InProgress', 'Reconnect'])

export function isResgPage(value: string): boolean {
  try {
    const url = new URL(value)
    if (url.protocol !== 'https:' || url.username || url.password || url.port) return false
    return (
      ['resg.top', 'www.resg.top'].includes(url.hostname) ||
      (['www.bilibili.com', 'www.bilibilitoy.com'].includes(url.hostname) &&
        url.pathname.startsWith('/toy/resg/'))
    )
  } catch {
    return false
  }
}

export function isResgContent(value: string): boolean {
  return isResgPage(value) && new URL(value).hostname === 'www.bilibilitoy.com'
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

// Runs in the unprivileged remote frame. Only public links/DOM are used, not Vue internals or APIs.
// Returning through the gallery remounts the site's detail component after a champion swap.
export function resgNavigationStep(championId: number, expectedName: string): string {
  if (!validChampion(championId)) throw new Error('Invalid champion')
  return `(() => {
    const id = ${championId}; const expected = ${JSON.stringify(expectedName)};
    const path = location.pathname;
    const heading = document.querySelector('h1')?.textContent?.trim() || '';
    if (path.endsWith('/champions/' + id) && expected && heading === expected)
      return { kind: 'ready', name: heading };
    const links = Array.from(document.querySelectorAll('a[href]'));
    if (/\\/champions\\//.test(path)) {
      if (path.endsWith('/champions/' + id) && expected) return { kind: 'waiting' };
      const back = links.find(a => /\\/v\\/[^/]+\\/?$/.test(new URL(a.href).pathname));
      back?.click(); return { kind: 'waiting' };
    }
    const link = links.find(a => new URL(a.href).origin === location.origin &&
      new URL(a.href).pathname.endsWith('/champions/' + id));
    if (!link) return { kind: 'waiting' };
    const name = link.querySelector('strong')?.textContent?.trim() || link.textContent.trim();
    link.click(); return { kind: 'selected', name };
  })()`
}
