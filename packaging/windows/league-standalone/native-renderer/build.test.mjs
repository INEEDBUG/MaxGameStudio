import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { stripTypeScriptTypes } from 'node:module'
import { join, resolve } from 'node:path'
import test from 'node:test'

import { createNativeRendererConfig } from './build.mjs'

const stagedSource = resolve('staged-league-fixture')
const outDir = resolve('native-webview-dist')

test('the native renderer substitutes local vector artwork for bundled brand and game images', () => {
  const plugin = createNativeRendererConfig({}, stagedSource, outDir).plugins[0]
  const shared = join(stagedSource, 'src/renderer-shared')
  const launchers = ['league', 'riotclient', 'wegame'].map((name) =>
    plugin.load?.(`${join(shared, 'assets/ico', `${name}.ico`)}?url`)
  )
  for (const launcher of launchers) {
    assert.match(launcher ?? '', /^export default "data:image\/svg\+xml,/)
  }
  assert.equal(new Set(launchers).size, 3)

  const oldArtwork = [
    'assets/logo/maxgamestudio-logo.png',
    'assets/ranked-icons/bronze.png',
    'assets/ranked-icons-large/challenger.png',
    'assets/champions/bravery-circle.png',
    'components/match-card/map-images/12.png'
  ]
  for (const original of oldArtwork) {
    assert.match(plugin.load?.(join(shared, original)) ?? '', /^export default "data:image\/svg\+xml,/)
  }
  assert.equal(plugin.load?.(join(shared, 'assets/champions/unrelated.png')), undefined)

  const originalLogo = '<template><svg><path id="original-brand" /></svg></template>'
  for (const logo of ['AkariLogo', 'OpggIcon']) {
    const component = `${join(shared, 'assets/icon', logo)}.vue`
    const result = plugin.transform(originalLogo, component)?.code ?? originalLogo
    assert.match(result, /<svg/)
    assert.doesNotMatch(result, /original-brand|assets\/logo\/|data:image\/png/)
  }
})

test('native code keeps icon bindings while replacing the Font Awesome package', () => {
  const plugin = createNativeRendererConfig({}, stagedSource, outDir).plugins[0]
  const about = join(stagedSource, 'src/renderer/src-main-window/components/settings-modal/AboutPane.vue')
  const original = "import { Github as GithubIcon, Star as StarIcon } from '@vicons/fa'\n"
  const result = plugin.transform(original, about)?.code ?? original
  assert.equal(result, "import { CodeSlashOutline as GithubIcon, StarOutline as StarIcon } from '@vicons/ionicons5'\n")
})

test('selects the three bundled windows and allows local Tauri IPC and game art in their CSP', () => {
  const vue = { name: 'vite:vue' }
  const devtools = { name: 'vite-plugin-vue-devtools' }
  const config = createNativeRendererConfig({ plugins: [vue, [devtools]] }, stagedSource, outDir)

  assert.equal(config.root, join(stagedSource, 'src/renderer'))
  assert.equal(config.base, './')
  assert.deepEqual(config.build.rolldownOptions.input, {
    main: join(config.root, 'main-window.html'),
    aux: join(config.root, 'aux-window.html'),
    opgg: join(config.root, 'opgg-window.html')
  })
  assert.equal(config.build.outDir, outDir)
  assert.equal(config.build.emptyOutDir, false)
  assert.equal(config.plugins.includes(vue), true)
  assert.equal(config.plugins.includes(devtools), false)

  const html =
    '<meta http-equiv="Content-Security-Policy" content="script-src \'self\'; connect-src \'self\';">'
  assert.equal(
    config.plugins[0].transformIndexHtml(html, {
      filename: config.build.rolldownOptions.input.main
    }),
    html.replace("connect-src 'self'", "img-src https: akari: data: blob: 'self' http://league-art.localhost; connect-src ipc: http://ipc.localhost 'self'")
  )
})

test('native OP.GG keeps the local shell and opens the official guide only through an explicit action', () => {
  const config = createNativeRendererConfig({}, stagedSource, outDir)
  const plugin = config.plugins[0]
  const opgg = join(config.root, 'src-opgg-window')
  const app = "import { provideOpgg } from './opgg/context'\n\nprovideOpgg()\n"
  const shards = "import { OpggRenderer } from './opgg'\nmanager.use(OpggRenderer)\nmanager.use(AkariIpcRenderer)\n"
  assert.doesNotMatch(plugin.transform(app, join(opgg, 'App.vue')).code, /provideOpgg/)
  assert.equal(plugin.transform(shards, join(opgg, 'shards/index.ts')).code, 'manager.use(AkariIpcRenderer)\n')
  const view = plugin.transform('const { currentTab } = useOpgg()', join(opgg, 'opgg/OpggView.vue')).code
  assert.match(view, /@click="openGuide"/)
  assert.match(view, /ipc\.call\(namespace, 'openGuide'\)/)
  assert.doesNotMatch(view, /https?:|\bfetch\b|axios|provideOpgg/)
  const meta = "default-src https: ws: 'self' data: blob: ; connect-src https: ws: 'self';"
  const html = plugin.transformIndexHtml(meta, { filename: config.build.rolldownOptions.input.opgg })
  assert.match(html, /default-src 'self' data: blob:;/)
  assert.match(html, /connect-src ipc: http:\/\/ipc\.localhost 'self';/)
  assert.doesNotMatch(html, /https:|ws:/)
})

test('League startup skips the unused upstream logo preference and still loads app state', () => {
  const renderer = createNativeRendererConfig({}, stagedSource, outDir).plugins[0]
  const shard = join(stagedSource, 'src/renderer-shared/shards/app-common')
  const fixture = `import type { SettingUtilsRenderer } from '../setting-utils'
import { APP_COMMON_RENDERER_NAMESPACE, MAIN_SHARD_NAMESPACE } from './context'

export async function syncAppCommonRendererState(
  piniaMobxUtils: PiniaMobxUtilsRenderer,
  settingUtils: SettingUtilsRenderer,
  getVersion: () => Promise<string>
) {
  const store = useAppCommonStore()
  store.version = await getVersion()

  await settingUtils.savedGetterVue(
    APP_COMMON_RENDERER_NAMESPACE,
    'tempAkariSubscriptionInfo',
    () => store.tempAkariSubscriptionInfo,
    (v) => (store.tempAkariSubscriptionInfo = v)
  )

  await piniaMobxUtils.sync(MAIN_SHARD_NAMESPACE, 'state', store)
  await piniaMobxUtils.sync(MAIN_SHARD_NAMESPACE, 'settings', store.settings)
}
`
  const entry = `await syncAppCommonRendererState(this._piniaMobxUtils, this._settingUtils, () =>
      this.getVersion()
    )`
  const source = renderer.transform(fixture, join(shard, 'settings-sync.ts'))?.code ?? fixture
  assert.doesNotMatch(source, /tempAkariSubscriptionInfo|settingUtils|APP_COMMON_RENDERER_NAMESPACE/)
  assert.match(source, /store\.version = await getVersion\(\)/)
  assert.match(source, /await piniaMobxUtils\.sync\(MAIN_SHARD_NAMESPACE, 'state', store\)/)
  assert.match(source, /await piniaMobxUtils\.sync\(MAIN_SHARD_NAMESPACE, 'settings', store\.settings\)/)
  assert.equal(
    renderer.transform(entry, join(shard, 'index.ts'))?.code,
    'await syncAppCommonRendererState(this._piniaMobxUtils, () =>\n      this.getVersion()\n    )'
  )
})

test('later client events take precedence over a delayed initial state', async () => {
  const renderer = createNativeRendererConfig({}, stagedSource, outDir).plugins[0]
  const filename = join(stagedSource, 'src/renderer-shared/shards/pinia-mobx-utils/pinia-state-sync.ts')
  const fixture = `
interface PropConfig {
  raw: boolean
}
class PiniaStateSync {
  constructor(private readonly _context: any) {}
  async sync(namespace: string, stateId: string, store: any) {
    this._context.ipc.onEvent(
      'mobx-utils-main',
      'update',
      (path, value, { action, raw }) => {
        if (action === 'update' || action === 'create') {
          store[path] = value
        } else if (action === 'delete') {
          delete store[path]
        }
      }
    )
    const initial: Record<string, { value: any; config: PropConfig }> = await this._context.ipc.call()
    Object.entries(initial).forEach(([key, { value, config }]) => {
      store[key] = value
    })
  }
}`
  const transformed = renderer.transform(fixture, filename)?.code ?? fixture
  const script = stripTypeScriptTypes(transformed, { mode: 'transform' })
  const PiniaStateSync = new Function(`${script}\nreturn PiniaStateSync`)()
  let onUpdate
  let resolveInitial
  const ipc = {
    onEvent(_namespace, _name, callback) { onUpdate = callback },
    call() { return new Promise((resolve) => { resolveInitial = resolve }) },
  }
  const store = {}
  const pending = new PiniaStateSync({ ipc }).sync('league-client-main', 'gameflow', store)
  onUpdate('phase', 'InProgress', { action: 'update', raw: false, revision: 3 })
  resolveInitial({
    phase: { value: 'ChampSelect', config: { raw: false, revision: 2 } },
    session: { value: null, config: { raw: false, revision: 2 } },
  })
  await pending
  assert.deepEqual(store, { phase: 'InProgress', session: null })
  onUpdate('phase', 'ChampSelect', { action: 'update', raw: false, revision: 2 })
  assert.equal(store.phase, 'InProgress')
  onUpdate('phase', null, { action: 'update', raw: false, revision: 4 })
  assert.equal(store.phase, null)

  const selected = {}
  const selecting = new PiniaStateSync({ ipc }).sync('league-client-main', 'champSelect', selected)
  resolveInitial({
    currentPickableChampionIds: { value: [2, 30], config: { raw: false } },
    gridChampions: { value: [], config: { raw: false } },
  })
  await selecting
  assert.deepEqual(selected.currentPickableChampionIds, new Set([2, 30]))
  assert.deepEqual(selected.gridChampions, [])
  onUpdate('currentBannableChampionIds', [1], { action: 'update', raw: false, revision: 2 })
  assert.deepEqual(selected.currentBannableChampionIds, new Set([1]))
})

test('connects the main title bar drag gesture to the native main window only', () => {
  const renderer = createNativeRendererConfig({}, stagedSource, outDir).plugins[0]
  const component = '<template><div class="shard-area"></div></template>\n<script setup lang="ts">\n</script>'
  const titlebar = join(stagedSource, 'src/renderer/src-main-window/components/titlebar/MainWindowTitlebar.vue')
  const native = renderer.transform(component, titlebar)?.code ?? component
  assert.match(native, /class="shard-area" @mousedown\.self\.left="startNativeDrag"/)
  assert.match(native, /call\('window-manager-main\/main-window', 'startDragging'\)/)
  assert.equal(renderer.transform(component, join(stagedSource, 'other.vue')), undefined)
})

test('checks through Rust and downloads only the checked version from the native updater', async () => {
  const renderer = createNativeRendererConfig({}, stagedSource, outDir).plugins[0]
  const component = readFileSync(new URL('../LeagueUpdateButton.vue', import.meta.url), 'utf8')
  const filename = join(stagedSource, 'src/renderer/src-main-window/views/league-overview/LeagueUpdateButton.vue')
  const native = renderer.transform(component, filename)?.code ?? component
  assert.doesNotMatch(native, /\bfetch\(|:href=|league-update'/)
  assert.match(native, /@click="openDownload"/)

  const scripts = [...native.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)]
    .map(([, script]) => script.replace(/^import .*$/gm, ''))
    .join('\n')
  const script = stripTypeScriptTypes(scripts, { mode: 'transform' })
  const createButton = new Function(
    'ref', 'shallowRef', 'computed', 'onMounted', 'useAppCommonStore', 'useSelfUpdateStore',
    'useInstance', 'AkariIpcRenderer', `${script}\nreturn { check, openDownload, status, update }`
  )
  const ref = (value) => ({ value })
  const calls = []
  let mounted
  let availableVersion = '1.0.0-beta.10'
  const makeButton = (autoCheckUpdates) => createButton(
    ref, ref, (getter) => getter, (callback) => { mounted = callback },
    () => ({ settings: { locale: 'zh-CN' } }), () => ({ settings: { autoCheckUpdates } }),
    () => ({ async call(...args) {
      calls.push(args)
      if (args[1] === 'check') return { availableVersion }
    } }), {}
  )
  const button = makeButton(false)
  mounted()
  assert.deepEqual(calls, [])
  await button.check()
  assert.equal(button.update.value, '1.0.0-beta.10')
  await button.openDownload()
  assert.deepEqual(calls, [
    ['league-update-main', 'check'],
    ['league-update-main', 'openDownload', '1.0.0-beta.10']
  ])

  availableVersion = null
  const automatic = makeButton(true)
  mounted()
  await new Promise(setImmediate)
  assert.equal(calls.length, 3)
  assert.equal(automatic.status.value, 'idle')
  assert.equal(automatic.update.value, null)
})
