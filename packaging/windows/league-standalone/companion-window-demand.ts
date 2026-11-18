interface CompanionWindowState {
  enabled: boolean
  initialized: boolean
  connected: boolean
  autoShow: boolean
  ready: boolean
  phase: string | null | undefined
  spectating?: boolean
}

export function companionWindowDemand(kind: 'aux' | 'opgg', state: CompanionWindowState) {
  if (!state.enabled || !state.initialized || !state.connected || !state.autoShow) {
    return 'ignore'
  }

  if (state.phase === 'ChampSelect') {
    if (kind === 'aux' && state.spectating) return 'ignore'
    return state.ready ? 'show' : 'prepare'
  }

  if (kind === 'opgg') return 'ignore'
  if (state.phase === 'Lobby' || state.phase === 'Matchmaking' || state.phase === 'ReadyCheck') {
    return state.ready ? 'show' : 'prepare'
  }

  // Keep the idle decision stable as a manually opened renderer becomes ready.
  return 'hide'
}
