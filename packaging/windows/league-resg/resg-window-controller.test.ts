import { observable, reaction, runInAction } from 'mobx'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { WindowManagerMainContext } from '../context'
import { ResgWindowController } from './resg-window-controller'

const fake = vi.hoisted(() => ({
  windows: [] as any[],
  views: [] as any[],
  prewarming: false,
  hang: false
}))
vi.mock('@main/mgs-prewarm', () => ({ isPrewarming: () => fake.prewarming }))
vi.mock('@resources/LA_ICON.ico?asset', () => ({ default: 'test-icon' }))
vi.mock('electron', async () => {
  const { EventEmitter } = await import('node:events')
  class Contents extends EventEmitter {
    destroyed = false
    url = ''
    session = Object.assign(new EventEmitter(), {
      setPermissionCheckHandler: vi.fn(),
      setPermissionRequestHandler: vi.fn()
    })
    mainFrame = {
      framesInSubtree: [
        {
          url: 'https://www.bilibilitoy.com/toy/resg/build/index.html',
          detached: false,
          executeJavaScript: vi.fn(async (script: string) => {
            if (fake.hang) return new Promise(() => {})
            if (script.includes('mgs-resg-compact-style')) return { kind: 'compact' }
            this.calls++
            return this.calls % 2
              ? { kind: 'selected', name: 'Fixture Champion' }
              : { kind: 'ready', name: 'Fixture Champion' }
          })
        }
      ]
    }
    calls = 0
    setWindowOpenHandler = vi.fn()
    loadURL = vi.fn(async (url: string) => {
      this.url = url
    })
    getURL() {
      return this.url
    }
    isDestroyed() {
      return this.destroyed
    }
    close() {
      this.destroyed = true
    }
  }
  class Window extends EventEmitter {
    webContents = new Contents()
    contentView = { addChildView: vi.fn() }
    visible = false
    destroyed = false
    options: any
    constructor(options: any) {
      super()
      this.options = options
      fake.windows.push(this)
    }
    show() {
      this.visible = true
    }
    showInactive() {
      this.visible = true
    }
    isVisible() {
      return this.visible && !this.destroyed
    }
    isMinimized() {
      return false
    }
    restore() {}
    setTitle() {}
    loadURL(url: string) {
      return this.webContents.loadURL(url)
    }
    getContentSize() {
      return [700, 800]
    }
    getNormalBounds() {
      return { x: 10, y: 10, width: 700, height: 800 }
    }
    isDestroyed() {
      return this.destroyed
    }
    destroy() {
      this.destroyed = true
      this.emit('closed')
    }
  }
  class View {
    webContents = new Contents()
    visible = false
    options: any
    constructor(options: any) {
      this.options = options
      fake.views.push(this)
    }
    setVisible(value: boolean) {
      this.visible = value
    }
    setBounds() {}
  }
  return {
    BrowserWindow: Window,
    WebContentsView: View,
    screen: { getAllDisplays: () => [{ workArea: { x: 0, y: 0, width: 1920, height: 1080 } }] },
    shell: { openExternal: vi.fn() }
  }
})

describe('RESG window behavior', () => {
  let controller: ResgWindowController
  let data: any
  let calls: Map<string, (...args: any[]) => any>
  let disposers: Array<() => void>
  let mainContents: any
  let saved: Map<string, any>
  const invoke = (name: string, ...args: any[]) =>
    calls.get(name)!({ sender: mainContents, senderFrame: mainContents.mainFrame }, ...args)
  beforeEach(async () => {
    vi.useFakeTimers()
    fake.windows.length = 0
    fake.views.length = 0
    fake.prewarming = false
    fake.hang = false
    calls = new Map()
    saved = new Map()
    disposers = []
    mainContents = { mainFrame: {} }
    data = observable({
      gameflow: {
        phase: 'ChampSelect',
        session: {
          gameData: {
            gameId: 101,
            queue: { gameMode: 'KIWI' },
            playerChampionSelections: [],
            teamOne: [],
            teamTwo: []
          }
        }
      },
      champSelect: {
        session: { localPlayerCellId: 1, myTeam: [{ cellId: 1, championId: 63 }], actions: [] }
      },
      summoner: { me: { puuid: 'self' } }
    })
    const context = {
      leagueClient: { data },
      appCommon: { settings: { theme: 'dark', locale: 'zh-CN' } },
      windowManager: {
        mainWindow: { window: { webContents: mainContents } },
        state: { isManagerFinishedInit: true }
      },
      logger: { warn: vi.fn() },
      ipc: {
        sendEvent: vi.fn(),
        onCall: (_ns: string, name: string, cb: any) => calls.set(name, cb)
      },
      settingFactory: {
        register: (_ns: string, _schema: any, settings: any) => ({
          applyToState: async () => {},
          set: async (key: string, value: any) => {
            saved.set(key, value)
            runInAction(() => {
              settings[key] = value
            })
          }
        })
      },
      mobxUtils: { reaction: (...args: any[]) => disposers.push((reaction as any)(...args)) }
    } as unknown as WindowManagerMainContext
    controller = new ResgWindowController(context)
    await controller.onInit()
  })
  afterEach(() => {
    controller.onDispose()
    disposers.forEach((dispose) => dispose())
    vi.useRealTimers()
  })

  it('defaults off without creating a browser or fetching the site', () => {
    expect(controller.getSnapshot().status).toBe('disabled')
    expect(fake.windows).toHaveLength(0)
  })
  it('remembers display choice, permits reverting, and does not open a disabled window', async () => {
    expect(controller.getSnapshot().displayMode).toBe('all')
    await invoke('setDisplayMode', 'compact')
    expect(saved.get('displayMode')).toBe('compact')
    expect(fake.windows).toHaveLength(0)
    await invoke('setEnabled', true)
    await vi.advanceTimersByTimeAsync(400)
    await invoke('setDisplayMode', 'all')
    await vi.advanceTimersByTimeAsync(20)
    expect(saved.get('displayMode')).toBe('all')
    expect(controller.getSnapshot()).toMatchObject({ status: 'ready', displayMode: 'all' })
    await expect(invoke('setDisplayMode', 'unknown')).rejects.toThrow()
  })
  it('applies the latest view choice during navigation and retains it on reopen', async () => {
    await invoke('setEnabled', true)
    await invoke('setDisplayMode', 'compact')
    await invoke('setDisplayMode', 'all')
    await invoke('setDisplayMode', 'compact')
    await vi.advanceTimersByTimeAsync(400)
    const frame = fake.views[0].webContents.mainFrame.framesInSubtree[0]
    const displayScripts = frame.executeJavaScript.mock.calls
      .map((call: string[]) => call[0])
      .filter((script: string) => script.includes('mgs-resg-compact-style'))
    expect(displayScripts.at(-1)).toContain('const mode = "compact"')
    await invoke('close')
    await invoke('show')
    await vi.advanceTimersByTimeAsync(400)
    expect(controller.getSnapshot()).toMatchObject({ status: 'ready', displayMode: 'compact' })
  })
  it('independently persists opt-in and hides data until the hero is verified', async () => {
    await invoke('setEnabled', true)
    expect(saved.get('enabled')).toBe(true)
    expect(fake.views[0].visible).toBe(false)
    await vi.advanceTimersByTimeAsync(400)
    expect(controller.getSnapshot()).toMatchObject({
      visible: true,
      status: 'ready',
      championId: 63
    })
    expect(fake.views[0].visible).toBe(true)
    expect(fake.views[0].options.webPreferences).toMatchObject({
      sandbox: true,
      nodeIntegration: false,
      contextIsolation: true
    })
    expect(fake.views[0].options.webPreferences.preload).toBeUndefined()
  })
  it('destroys remote contents on disable and prevents late results from reopening it', async () => {
    await invoke('setEnabled', true)
    const view = fake.views[0]
    await invoke('setEnabled', false)
    await vi.advanceTimersByTimeAsync(500)
    expect(view.webContents.destroyed).toBe(true)
    expect(controller.getSnapshot()).toMatchObject({
      visible: false,
      enabled: false,
      status: 'disabled'
    })
  })
  it('manual close remains closed across hero swaps, but manual reopen works', async () => {
    await invoke('setEnabled', true)
    await invoke('close')
    runInAction(() => {
      data.champSelect.session.myTeam[0].championId = 223
    })
    expect(controller.getSnapshot().visible).toBe(false)
    expect(fake.windows).toHaveLength(1)
    await invoke('show')
    await vi.advanceTimersByTimeAsync(400)
    expect(controller.getSnapshot()).toMatchObject({
      visible: true,
      championId: 223,
      status: 'ready'
    })
  })
  it('disables automatic popup without disabling manual open', async () => {
    await invoke('setAutoShow', false)
    await invoke('setEnabled', true)
    expect(fake.windows).toHaveLength(0)
    await invoke('show')
    expect(fake.windows).toHaveLength(1)
  })
  it('closes immediately on unrelated mode and refuses manual opening', async () => {
    await invoke('setEnabled', true)
    runInAction(() => {
      data.gameflow.session.gameData.queue.gameMode = 'ARAM'
    })
    await invoke('show')
    expect(controller.getSnapshot()).toMatchObject({ available: false, visible: false })
    expect(fake.views[0].webContents.destroyed).toBe(true)
  })
  it('keeps remote pages and other renderers out of the privileged controller', () => {
    expect(() => calls.get('setEnabled')!({ sender: {}, senderFrame: {} }, true)).toThrow(
      'Untrusted'
    )
    expect(() => calls.get('show')!({ sender: mainContents, senderFrame: {} })).toThrow('Untrusted')
  })
  it('does not open or fetch during host prewarm', async () => {
    fake.prewarming = true
    await invoke('setEnabled', true)
    await invoke('show')
    expect(fake.windows).toHaveLength(0)
  })
  it('bounds a stalled remote frame and permits retry', async () => {
    fake.hang = true
    await invoke('setEnabled', true)
    await vi.advanceTimersByTimeAsync(3100)
    expect(controller.getSnapshot().status).toBe('error')
    fake.hang = false
    await invoke('retry')
    await vi.advanceTimersByTimeAsync(400)
    expect(controller.getSnapshot().status).toBe('ready')
  })
})
