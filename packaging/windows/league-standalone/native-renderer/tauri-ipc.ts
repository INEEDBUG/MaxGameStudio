interface LeagueTauriApi {
  core: {
    invoke<T>(
      command: 'league_call',
      payload: { namespace: string; method: string; args: unknown[] }
    ): Promise<T>
  }
  webviewWindow: {
    getCurrentWebviewWindow(): {
      listen(
        event: 'league-ipc-event',
        handler: (event: { payload: unknown }) => void
      ): Promise<() => void | Promise<void>>
    }
  }
}

function getTauriApi() {
  const { __TAURI__ } = globalThis as typeof globalThis & { __TAURI__?: LeagueTauriApi }
  if (!__TAURI__) {
    throw new Error('League native renderer requires the Tauri webview.')
  }
  return __TAURI__
}

export async function invokeLeagueCall<T>(namespace: string, method: string, ...args: unknown[]) {
  return getTauriApi().core.invoke<T>('league_call', { namespace, method, args })
}

export async function listenLeagueEvents(
  handler: (namespace: string, eventName: string, ...args: unknown[]) => void
) {
  let listening = true
  const currentWindow = getTauriApi().webviewWindow.getCurrentWebviewWindow()
  const unlisten = await currentWindow.listen('league-ipc-event', ({ payload }) => {
    if (!listening || !payload || typeof payload !== 'object') return

    const { namespace, eventName, args } = payload as Record<string, unknown>
    if (typeof namespace === 'string' && typeof eventName === 'string' && Array.isArray(args)) {
      handler(namespace, eventName, ...args)
    }
  })

  return async () => {
    if (!listening) return
    listening = false
    await unlisten()
  }
}
