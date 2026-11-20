import assert from 'node:assert/strict'
import { stripTypeScriptTypes } from 'node:module'
import { join, resolve } from 'node:path'
import test from 'node:test'

import { createNativeRendererConfig } from './build.mjs'

const stagedSource = resolve('staged-league-fixture')
const outDir = resolve('native-webview-dist')

test('selects the three bundled windows and allows local Tauri IPC in their CSP', () => {
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
    html.replace("connect-src 'self'", "connect-src ipc: http://ipc.localhost 'self'")
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
