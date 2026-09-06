import { isPrewarming } from '@main/mgs-prewarm'
import icon from '@resources/LA_ICON.ico?asset'
import { BrowserWindow, type IpcMainInvokeEvent, WebContentsView, screen, shell } from 'electron'
import { comparer, makeAutoObservable } from 'mobx'
import { z } from 'zod'

import type { WindowManagerMainContext } from '../context'
import { type ResgDisplayMode, resgDisplayScript } from './resg-display'
import {
  RESG_HOME,
  RESG_NAMESPACE,
  isResgContent,
  isResgPage,
  resgNavigationStep,
  selectResgChampion
} from './resg-policy'

class ResgSettings {
  enabled = false
  autoShow = true
  displayMode: ResgDisplayMode = 'all'
  bounds: { x: number; y: number; width: number; height: number } | null = null
  constructor() {
    makeAutoObservable(this)
  }
}

export class ResgWindowController {
  readonly settings = new ResgSettings()
  private readonly _service
  private _window: BrowserWindow | null = null
  private _view: WebContentsView | null = null
  private _championId = 0
  private _autoShown = false
  private _generation = 0
  private _status: 'disabled' | 'waiting' | 'loading' | 'ready' | 'error' = 'disabled'
  private _error: string | null = null
  private _expectedName = ''
  private _displayFallback = false
  private _navigation: Promise<void> = Promise.resolve()

  constructor(private readonly _context: WindowManagerMainContext) {
    this._service = _context.settingFactory.register(
      RESG_NAMESPACE,
      {
        enabled: { default: false, schema: z.boolean() },
        autoShow: { default: true, schema: z.boolean() },
        displayMode: { default: 'all', schema: z.enum(['all', 'compact']) },
        bounds: {
          default: null,
          schema: z
            .object({
              x: z.number().int(),
              y: z.number().int(),
              width: z.number().int().min(480).max(10000),
              height: z.number().int().min(480).max(10000)
            })
            .nullable()
        }
      },
      this.settings
    )
  }

  getSnapshot() {
    return {
      enabled: this.settings.enabled,
      autoShow: this.settings.autoShow,
      displayMode: this.settings.displayMode,
      displayFallback: this._displayFallback,
      visible: this._window?.isVisible() ?? false,
      available: this._championId > 0,
      championId: this._championId,
      status: this._status,
      error: this._error
    }
  }

  async onInit() {
    await this._service.applyToState()
    const bind = (name: string, callback: (...args: unknown[]) => unknown) => {
      this._context.ipc.onCall(
        RESG_NAMESPACE,
        name,
        (event: IpcMainInvokeEvent, ...args: unknown[]) => {
          // Only the trusted main renderer can control this feature. Remote contents never get IPC.
          if (
            event.sender !== this._context.windowManager.mainWindow.window?.webContents ||
            event.senderFrame !== event.sender.mainFrame
          )
            throw new Error('Untrusted RESG caller')
          return callback(...args)
        }
      )
    }
    bind('getSnapshot', () => this.getSnapshot())
    bind('setEnabled', async (value) => {
      await this._service.set('enabled', z.boolean().parse(value))
      return this.getSnapshot()
    })
    bind('setAutoShow', async (value) => {
      await this._service.set('autoShow', z.boolean().parse(value))
      return this.getSnapshot()
    })
    bind('setDisplayMode', async (value) => {
      await this.setDisplayMode(z.enum(['all', 'compact']).parse(value))
      return this.getSnapshot()
    })
    bind('show', () => {
      this.show()
      return this.getSnapshot()
    })
    bind('close', () => {
      this.close()
      return this.getSnapshot()
    })
    bind('retry', () => {
      this.retry()
      return this.getSnapshot()
    })
    this._context.mobxUtils.reaction(
      () => {
        const data = this._context.leagueClient.data
        return [
          this.settings.enabled,
          this.settings.autoShow,
          selectResgChampion(
            data.gameflow.phase,
            data.gameflow.session,
            data.champSelect.session,
            data.summoner.me?.puuid
          ),
          data.gameflow.phase,
          data.gameflow.session?.gameData.gameId,
          this._context.windowManager.state.isManagerFinishedInit
        ] as const
      },
      ([enabled, autoShow, championId, phase, , ready]) => {
        const previous = this._championId
        this._championId = championId
        if (!ready || isPrewarming()) return
        if (!enabled || !championId) {
          this._destroy()
          this._status = enabled ? 'waiting' : 'disabled'
          if (
            !enabled ||
            !['ChampSelect', 'GameStart', 'InProgress', 'Reconnect'].includes(phase || '')
          )
            this._autoShown = false
        } else if (!this._window && autoShow && !this._autoShown) {
          this.show(false)
        } else if (this._window && championId !== previous) {
          this._scheduleNavigation()
        } else if (!this._window) this._status = 'waiting'
        this._publish()
      },
      { fireImmediately: true, equals: comparer.structural }
    )
    this._context.mobxUtils.reaction(
      () =>
        [this._context.appCommon.settings.theme, this._context.appCommon.settings.locale] as const,
      () => {
        this._renderShell()
        this._scheduleDisplay()
      },
      { equals: comparer.shallow }
    )
  }

  show(focus = true) {
    if (!this.settings.enabled || !this._championId || isPrewarming()) return
    this._autoShown = true
    if (this._window) {
      if (this._window.isMinimized()) this._window.restore()
      focus ? this._window.show() : this._window.showInactive()
      return this._publish()
    }
    const bounds = this.settings.bounds
    const onScreen =
      bounds &&
      screen.getAllDisplays().some((display) => {
        const area = display.workArea
        return (
          bounds.x + 100 > area.x &&
          bounds.x < area.x + area.width - 100 &&
          bounds.y >= area.y &&
          bounds.y < area.y + area.height - 80
        )
      })
    const win = (this._window = new BrowserWindow({
      width: 700,
      height: 800,
      ...(onScreen ? bounds : {}),
      minWidth: 480,
      minHeight: 480,
      show: false,
      title: 'RESG · MaxGameStudio',
      icon,
      autoHideMenuBar: true,
      webPreferences: {
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        partition: 'mgs-resg-shell'
      }
    }))
    const view = (this._view = new WebContentsView({
      webPreferences: {
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        webSecurity: true,
        partition: 'mgs-resg-remote',
        backgroundThrottling: true
      }
    }))
    win.contentView.addChildView(view)
    view.setVisible(false)
    const session = view.webContents.session
    session.setPermissionCheckHandler(() => false)
    session.setPermissionRequestHandler((_contents, _permission, callback) => callback(false))
    // Replace the listener on the dedicated session; no accumulation across close/reopen.
    session.removeAllListeners('will-download')
    session.on('will-download', (event) => event.preventDefault())
    for (const contents of [win.webContents, view.webContents]) {
      contents.setWindowOpenHandler(() => ({ action: 'deny' }))
      contents.on('will-attach-webview', (event) => event.preventDefault())
    }
    view.webContents.on('will-navigate', (event, url) => {
      if (!isResgPage(url)) event.preventDefault()
    })
    view.webContents.on('will-redirect', (event, url) => {
      if (!isResgPage(url)) event.preventDefault()
    })
    view.webContents.on('will-frame-navigate', (event) => {
      // The site's original wrapper iframe is allowed; unrelated top-level destinations are not.
      if (event.isMainFrame && !isResgPage(event.url)) event.preventDefault()
    })
    view.webContents.on('render-process-gone', () => {
      if (this._view === view) this._fail('page-unavailable')
    })
    win.webContents.on('will-navigate', (event, url) => {
      event.preventDefault()
      if (url === 'mgs-resg:retry') this.retry()
      if (url === 'mgs-resg:display-all' || url === 'mgs-resg:display-compact') {
        void this.setDisplayMode(url.endsWith('compact') ? 'compact' : 'all').catch(() => {
          this._context.logger.warn('RESG display preference was not saved')
        })
      }
      if (url === 'mgs-resg:browser')
        void shell.openExternal(RESG_HOME).catch(() => this._fail('page-unavailable'))
    })
    const resize = () => {
      const [width, height] = win.getContentSize()
      view.setBounds({ x: 0, y: 82, width, height: Math.max(0, height - 82) })
    }
    win.on('resize', resize)
    win.on('close', () => this._saveBounds())
    win.on('closed', () => {
      if (this._window === win) {
        this._destroy()
        this._status = 'waiting'
        this._publish()
      }
    })
    resize()
    this._status = 'loading'
    this._renderShell()
    focus ? win.show() : win.showInactive()
    this._scheduleNavigation(true)
  }

  close() {
    this._autoShown = true // Respect a manual close until the next match, including bench swaps.
    this._saveBounds()
    this._destroy()
    this._status = this.settings.enabled ? 'waiting' : 'disabled'
    this._publish()
  }

  retry() {
    if (!this._window) this.show()
    else this._scheduleNavigation(true)
  }

  async setDisplayMode(mode: ResgDisplayMode) {
    await this._service.set('displayMode', z.enum(['all', 'compact']).parse(mode))
    this._renderShell()
    this._publish()
    // While navigating, the latest preference is applied before revealing the hero.
    this._scheduleDisplay()
  }

  private _scheduleDisplay() {
    const generation = this._generation
    this._navigation = this._navigation
      .catch(() => {})
      .then(async () => {
        const view = this._view
        if (!view || generation !== this._generation || this._status !== 'ready') return
        view.setVisible(false)
        await this._applyDisplay()
        if (generation !== this._generation || this._view !== view) return
        view.setVisible(true)
        this._renderShell()
        this._publish()
      })
  }

  private async _applyDisplay() {
    const frame = this._view?.webContents.mainFrame.framesInSubtree.find(
      (frame) => !frame.detached && isResgContent(frame.url)
    )
    if (!frame) return
    const script = (mode: ResgDisplayMode) =>
      resgDisplayScript(
        mode,
        this._championId,
        this._context.appCommon.settings.locale.startsWith('en'),
        this._context.appCommon.settings.theme !== 'light'
      )
    let timer: ReturnType<typeof setTimeout> | undefined
    try {
      const result = await Promise.race([
        frame.executeJavaScript(script(this.settings.displayMode)),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error('Display timeout')), 3000)
        })
      ])
      this._displayFallback =
        typeof result === 'object' &&
        result !== null &&
        'kind' in result &&
        result.kind === 'fallback'
    } catch {
      this._displayFallback = this.settings.displayMode === 'compact'
      // A changed/unavailable website must retain the full view as a fallback.
      void frame.executeJavaScript(script('all')).catch(() => {})
    } finally {
      clearTimeout(timer)
    }
  }

  private _saveBounds() {
    if (this._window && !this._window.isDestroyed()) {
      void this._service
        .set('bounds', this._window.getNormalBounds())
        .catch(() => this._context.logger.warn('RESG window position was not saved'))
    }
  }

  private _destroy() {
    ++this._generation
    const win = this._window
    const view = this._view
    this._window = null
    this._view = null
    // Electron does not destroy child WebContentsView contents with its parent automatically.
    if (view && !view.webContents.isDestroyed()) view.webContents.close()
    if (win && !win.isDestroyed()) win.destroy()
    this._error = null
    this._expectedName = ''
    this._displayFallback = false
  }

  onDispose() {
    this._saveBounds()
    this._destroy()
  }

  private _publish() {
    this._context.ipc.sendEvent(RESG_NAMESPACE, 'changed', this.getSnapshot())
  }

  private _fail(reason: string) {
    this._view?.setVisible(false)
    this._status = 'error'
    this._error = reason
    this._renderShell()
    this._publish()
  }

  private _scheduleNavigation(reload = false) {
    const generation = ++this._generation
    const id = this._championId
    this._status = 'loading'
    this._error = null
    this._expectedName = ''
    this._view?.setVisible(false)
    this._renderShell()
    this._publish()
    this._navigation = this._navigation
      .catch(() => {})
      .then(async () => {
        const view = this._view
        if (!view || generation !== this._generation) return
        const current = () => generation === this._generation && !view.webContents.isDestroyed()
        try {
          // A timeout covers navigation too; an unreachable website must not stall later requests.
          const deadline = Date.now() + 25000
          if (reload || !view.webContents.getURL()) {
            void view.webContents.loadURL(RESG_HOME).catch(() => {})
          }
          let expected = ''
          while (current() && Date.now() < deadline) {
            const frame = view.webContents.mainFrame.framesInSubtree.find((frame) =>
              isResgContent(frame.url)
            )
            if (frame && !frame.detached) {
              let timer: ReturnType<typeof setTimeout> | undefined
              const result = (await Promise.race([
                frame.executeJavaScript(resgNavigationStep(id, expected)),
                new Promise<never>((_, reject) => {
                  timer = setTimeout(() => reject(new Error('Frame timeout')), 3000)
                })
              ]).finally(() => clearTimeout(timer))) as { kind: string; name?: string }
              if (!current()) return
              if (result.kind === 'selected') expected = result.name || ''
              if (result.kind === 'ready' && expected) {
                this._expectedName = expected
                await this._applyDisplay()
                if (!current()) return
                this._status = 'ready'
                view.setVisible(true)
                this._renderShell()
                this._publish()
                return
              }
            }
            await new Promise((resolve) => setTimeout(resolve, 180))
          }
          if (current()) this._fail('champion-unavailable')
        } catch {
          if (current()) this._fail('page-unavailable')
        }
      })
  }

  private _renderShell() {
    const win = this._window
    if (!win || win.isDestroyed()) return
    const english = this._context.appCommon.settings.locale.startsWith('en')
    const dark = this._context.appCommon.settings.theme !== 'light'
    const escape = (text: string) =>
      text.replace(/[&<>"']/g, (character) => `&#${character.charCodeAt(0)};`)
    const title =
      this._status === 'ready'
        ? this._expectedName
        : this._status === 'error'
          ? english
            ? 'Unable to load champion data. Retry or open the website.'
            : '英雄数据加载失败，可重试或在浏览器打开网站。'
          : english
            ? 'Loading current Hextech ARAM champion…'
            : '正在加载当前海克斯大乱斗英雄…'
    win.setTitle(`RESG · ${title} · MaxGameStudio`)
    const html = `<!doctype html><html lang="${english ? 'en' : 'zh-CN'}"><meta charset="utf-8">
      <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">
      <style>body{margin:0;font:14px system-ui;background:${dark ? '#111' : '#fff'};color:${dark ? '#eee' : '#172033'}}
      header{height:42px;box-sizing:border-box;padding:10px;display:flex;gap:14px;align-items:center}
      span{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}a{color:${dark ? '#fbbf24' : '#1d4ed8'}}
      a:focus-visible{outline:2px solid currentColor;outline-offset:3px}p{margin:32px;line-height:1.6}</style>
      <header><span role="status">${escape(title)}</span><a href="mgs-resg:retry">${english ? 'Retry' : '重试'}</a>
      <a href="mgs-resg:browser">${english ? 'Browser' : '浏览器'}</a></header>
      <nav aria-label="${english ? 'Data display' : '数据展示'}">
      <a href="mgs-resg:display-all" aria-current="${this.settings.displayMode === 'all'}">${english ? 'All data' : '全部数据'}</a>
      <a href="mgs-resg:display-compact" aria-current="${this.settings.displayMode === 'compact'}">${english ? 'Popular picks' : '常用精选'}</a>
      <span role="status">${this._displayFallback ? (english ? 'Page changed · showing all data' : '页面数据暂不适配，已显示全部') : english ? 'Choice remembered' : '自动记住选择'}</span></nav>
      <style>nav{height:40px;box-sizing:border-box;padding:0 10px 8px;display:flex;align-items:center;gap:8px;font-size:12px}
      nav a{padding:4px 9px;border:1px solid ${dark ? '#52525b' : '#cbd5e1'};border-radius:5px;text-decoration:none}
      nav a[aria-current=true]{background:${dark ? '#f59e0b' : '#2563eb'};color:${dark ? '#171717' : '#fff'};font-weight:600}
      nav span{color:${dark ? '#b5b5bd' : '#535d70'}}</style>
      <p>${escape(title)}</p></html>`
    void win.loadURL('data:text/html;charset=utf-8,' + encodeURIComponent(html)).catch(() => {})
  }
}
