// Isolated Electron acceptance test. Does not connect to LCU, launch games, or use user profiles.
// node smoke-resg.cjs <staged-source> <new-output-directory>
const fs = require('node:fs')
const path = require('node:path')
const assert = require('node:assert/strict')
const { createRequire } = require('node:module')
const source = path.resolve(process.argv[2])
const output = path.resolve(process.argv[3])
const dependency = createRequire(path.join(source, 'package.json'))

if (!process.versions.electron) {
  if (fs.existsSync(output)) throw new Error('Use a new, isolated output directory')
  fs.mkdirSync(output, { recursive: true })
  dependency('esbuild').buildSync({
    entryPoints: [
      path.join(source, 'src/main/shards/window-manager/resg-window/resg-window-controller.ts')
    ],
    outfile: path.join(output, 'controller.cjs'),
    bundle: true,
    platform: 'node',
    format: 'cjs',
    external: ['electron', 'mobx', 'zod', '@main/mgs-prewarm', '@resources/LA_ICON.ico?asset']
  })
  const environment = { ...process.env, TEMP: output, TMP: output }
  delete environment.ELECTRON_RUN_AS_NODE
  const child = require('node:child_process').spawnSync(
    dependency('electron'),
    [__filename, source, output],
    {
      windowsHide: true,
      timeout: 100000,
      encoding: 'utf8',
      env: environment
    }
  )
  const result = path.join(output, 'result.json')
  if (fs.existsSync(result)) console.log(fs.readFileSync(result, 'utf8'))
  if (child.error) console.error(child.error.message)
  if (child.status !== 0) console.error(child.stderr?.slice(-2500))
  process.exit(child.status === 0 && fs.existsSync(result) ? 0 : 1)
}

const { app, BrowserWindow, webContents } = require('electron')
const Module = require('node:module')
const originalLoad = Module._load
Module._load = function (request, parent, isMain) {
  if (request === '@main/mgs-prewarm') return { isPrewarming: () => false }
  if (request === '@resources/LA_ICON.ico?asset') return path.join(source, 'resources/LA_ICON.ico')
  if (request === 'mobx' || request === 'zod')
    return originalLoad.call(this, dependency.resolve(request), parent, isMain)
  return originalLoad.call(this, request, parent, isMain)
}
app.setPath('userData', path.join(output, 'profile'))
app.setPath('sessionData', path.join(output, 'profile'))
app.setPath('temp', output)
app.disableHardwareAcceleration()
// Exercise real Chromium and native lifecycle without covering/focusing the user's game.
BrowserWindow.prototype.show = function () {}
BrowserWindow.prototype.showInactive = function () {}
const results = []
function finish(error) {
  fs.writeFileSync(
    path.join(output, 'result.json'),
    JSON.stringify(
      { passed: !error, checks: results, error: error ? String(error.stack || error) : null },
      null,
      2
    )
  )
  app.exit(error ? 1 : 0)
}
process.on('uncaughtException', finish)
process.on('unhandledRejection', finish)
setTimeout(() => finish(new Error('Smoke timeout')), 90000)
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
app.whenReady().then(async () => {
  const { ResgWindowController } = require(path.join(output, 'controller.cjs'))
  // Same MobX instance as the bundled controller, exposed via its observable settings below.
  // Use production reactions through the pinned runtime; esbuild must keep mobx external.
  const { observable, reaction, runInAction } = dependency('mobx')
  const main = new BrowserWindow({ show: false, webPreferences: { sandbox: true } })
  const data = observable({
    gameflow: {
      phase: 'ChampSelect',
      session: {
        gameData: {
          gameId: 1,
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
    summoner: { me: { puuid: 'fixture-only' } }
  })
  const handlers = new Map()
  const disposers = []
  const appearance = observable({ theme: 'dark', locale: 'zh-CN' })
  const context = {
    leagueClient: { data },
    appCommon: { settings: appearance },
    windowManager: { mainWindow: { window: main }, state: { isManagerFinishedInit: true } },
    logger: { warn() {} },
    ipc: { onCall: (_namespace, name, callback) => handlers.set(name, callback), sendEvent() {} },
    settingFactory: {
      register: (_namespace, _schema, settings) => ({
        applyToState: async () => {},
        set: async (key, value) =>
          runInAction(() => {
            settings[key] = value
          })
      })
    },
    mobxUtils: { reaction: (...args) => disposers.push(reaction(...args)) }
  }
  const controller = new ResgWindowController(context)
  const call = (name, ...args) =>
    handlers.get(name)(
      { sender: main.webContents, senderFrame: main.webContents.mainFrame },
      ...args
    )
  const ready = async () => {
    const deadline = Date.now() + 30000
    while (Date.now() < deadline) {
      const snapshot = controller.getSnapshot()
      if (snapshot.status === 'ready') return snapshot
      if (snapshot.status === 'error') throw new Error('RESG status: ' + snapshot.error)
      await delay(200)
    }
    throw new Error('Champion never became ready')
  }
  const remote = () =>
    webContents
      .getAllWebContents()
      .find((contents) => /^https:\/\/(www\.)?(resg\.top|bilibili\.com)/.test(contents.getURL()))
  try {
    await controller.onInit()
    assert.equal(remote(), undefined)
    results.push('disabled: no remote renderer')
    await call('setEnabled', true)
    await ready()
    for (const [id, name] of [
      [63, '复仇焰魂'],
      [223, '河流之王'],
      [17, '迅捷斥候']
    ]) {
      runInAction(() => {
        data.champSelect.session.myTeam[0].championId = id
      })
      await ready()
      const frame = remote().mainFrame.framesInSubtree.find((frame) =>
        frame.url.startsWith('https://www.bilibilitoy.com/toy/resg/')
      )
      assert.ok(frame)
      const actual = await frame.executeJavaScript(
        `({name:document.querySelector('h1')?.textContent?.trim(),node:typeof require,bridge:typeof window.electron,path:location.pathname})`
      )
      assert.equal(actual.name, name)
      assert.equal(actual.node, 'undefined')
      assert.equal(actual.bridge, 'undefined')
      assert.ok(actual.path.endsWith('/champions/' + id))
      results.push({ championId: id, heading: actual.name, isolated: true })
      for (const theme of id === 63 ? ['dark', 'light'] : ['dark']) {
        runInAction(() => {
          appearance.theme = theme
        })
        if (id === 63) {
          const shellWindow = BrowserWindow.getAllWindows().find((win) =>
            win.getTitle().startsWith('RESG')
          )
          assert.ok(shellWindow)
          await shellWindow.webContents.executeJavaScript(
            `document.querySelector('a[href="mgs-resg:display-compact"]').click()`
          )
          for (let i = 0; i < 50 && controller.getSnapshot().displayMode !== 'compact'; i++)
            await delay(50)
          assert.equal(
            controller.getSnapshot().displayMode,
            'compact',
            'native toolbar click changes the saved mode'
          )
        } else {
          await call('setDisplayMode', 'compact')
        }
        const deadline = Date.now() + 6000
        let compact
        do {
          compact = await frame.executeJavaScript(`({
            id:document.body.dataset.mgsResgCompact,
            name:document.querySelector('#mgs-resg-compact h1')?.textContent,
            counts:Array.from(document.querySelectorAll('#mgs-resg-compact .card')).map(card=>card.querySelectorAll('.pick').length)
          })`)
          if (compact.id === String(id)) break
          await delay(100)
        } while (Date.now() < deadline)
        assert.equal(compact.id, String(id), 'compact view must support the live website')
        assert.equal(compact.name, name)
        assert.ok(compact.counts.length >= 4 && compact.counts.every((count) => count <= 3))
        assert.equal(controller.getSnapshot().displayFallback, false)
        fs.writeFileSync(
          path.join(output, 'icon-diagnostic.json'),
          JSON.stringify(
            await frame.executeJavaScript(
              `({base:document.baseURI,icons:Array.from(document.querySelectorAll('#mgs-resg-compact img')).slice(0,4).map(img=>({src:img.src,complete:img.complete,width:img.naturalWidth}))})`
            )
          )
        )
        // decode() can wait for rendering in an intentionally hidden WebContentsView.
        // Natural dimensions prove successful image loading without a visible surface.
        let iconsLoaded = false
        for (let i = 0; i < 60; i++) {
          iconsLoaded = await frame.executeJavaScript(
            `Array.from(document.querySelectorAll('#mgs-resg-compact img')).every(img=>img.complete && img.naturalWidth > 0)`
          )
          if (iconsLoaded) break
          await delay(200)
        }
        assert.equal(iconsLoaded, true, 'compact icons must actually load')
        results.push({ compactChampion: id, theme, counts: compact.counts })
        if (id === 63) {
          const html = await frame.executeJavaScript(
            `'<!doctype html><meta charset="utf-8">' + document.getElementById('mgs-resg-compact-style').outerHTML + document.getElementById('mgs-resg-compact').outerHTML`
          )
          fs.writeFileSync(path.join(output, 'compact-' + theme + '.html'), html)
        }
      }
      await call('setDisplayMode', 'all')
      for (let i = 0; i < 50; i++) {
        if (await frame.executeJavaScript(`!document.getElementById('mgs-resg-compact')`)) break
        await delay(50)
      }
      assert.equal(
        await frame.executeJavaScript(`!!document.getElementById('mgs-resg-compact')`),
        false
      )
      assert.equal(
        await frame.executeJavaScript(`document.querySelector('.augment-quality-select')?.value`),
        'all'
      )
      results.push('full page restored with original quality filter')
    }
    // Hidden Chromium contents may have no compositor surface. DOM checks above are mandatory;
    // an optional screenshot is not allowed to skip the lifecycle checks below.
    try {
      const image = await remote().capturePage()
      fs.writeFileSync(path.join(output, 'resg-verified.png'), image.toPNG())
    } catch {
      results.push('hidden screenshot unavailable; DOM validation completed')
    }
    const oldContents = remote()
    await call('close')
    for (let i = 0; i < 50 && !oldContents.isDestroyed(); i++) await delay(20)
    assert.ok(oldContents.isDestroyed())
    runInAction(() => {
      data.champSelect.session.myTeam[0].championId = 63
    })
    await delay(300)
    assert.equal(remote(), undefined)
    results.push('manual close destroys renderer and suppresses swap popup')
    await call('show')
    await ready()
    const reopenedContents = remote()
    runInAction(() => {
      data.gameflow.session.gameData.queue.gameMode = 'ARAM'
    })
    for (let i = 0; i < 50 && !reopenedContents.isDestroyed(); i++) await delay(20)
    assert.equal(remote(), undefined)
    results.push('ordinary ARAM: closes renderer and disables champion following')
    controller.onDispose()
    disposers.forEach((dispose) => dispose())
    main.destroy()
    finish()
  } catch (error) {
    controller.onDispose()
    finish(error)
  }
})
