import { readFileSync } from 'node:fs'
import { copyFile, readdir, realpath, stat, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { isAbsolute, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { resgDisplayProgram } from '../../league-resg/resg-display.ts'

const IPC_MODULE = 'virtual:mgs-league-native-ipc'
const IPC_SOURCE = fileURLToPath(new URL('./tauri-ipc.ts', import.meta.url))
const OPGG_COMPANION = new URL('./NativeOpggCompanion.vue', import.meta.url)
const GAME_ART_ORIGIN = 'http://league-art.localhost'
const WINDOW_ENTRIES = {
  main: 'main-window.html',
  aux: 'aux-window.html',
  opgg: 'opgg-window.html'
}

function replaceOnce(source, anchor, replacement, file) {
  const first = source.indexOf(anchor)
  if (first === -1 || source.indexOf(anchor, first + anchor.length) !== -1) {
    throw new Error(`Expected exactly one native renderer patch anchor in ${file}: ${anchor}`)
  }
  return source.slice(0, first) + replacement + source.slice(first + anchor.length)
}

const IPC_TRANSFORMS = {
  'index.ts': [
    [
      "import { ElectronAPI } from '@electron-toolkit/preload'",
      `import { listenLeagueEvents } from '${IPC_MODULE}'`
    ],
    ["import { IpcRendererEvent } from 'electron'\n", ''],
    [
      `declare global {
  interface Window {
    electron: ElectronAPI
  }
}

`,
      ''
    ],
    ['private _cancelFn: (() => void) | null', 'private _cancelFn: (() => Promise<void>) | null'],
    [
      `  private _dispatchEvent(
    _event: IpcRendererEvent,
    namespace: string,
    eventName: string,
    ...args: any[]
  ) {
    this._eventRegistry.dispatch(_event, namespace, eventName, ...args)
  }

  async onInit() {
    this._cancelFn = window.electron.ipcRenderer.on('akari-event', this._dispatchEvent)
    await window.electron.ipcRenderer.invoke('akariRendererRegister', 'register')
  }

  async onDispose() {
    this._cancelFn?.()
    this._cancelFn = null
    this._eventRegistry.clear()
    await window.electron.ipcRenderer.invoke('akariRendererRegister', 'unregister')
  }`,
      `  private _dispatchEvent(namespace: string, eventName: string, ...args: unknown[]) {
    this._eventRegistry.dispatch(namespace, eventName, ...args)
  }

  async onInit() {
    this._cancelFn = await listenLeagueEvents(this._dispatchEvent)
  }

  async onDispose() {
    const cancel = this._cancelFn
    this._cancelFn = null
    this._eventRegistry.clear()
    await cancel?.()
  }`
    ]
  ],
  'event-registry.ts': [
    ["import type { IpcRendererEvent } from 'electron'\n\n", ''],
    ['dispatch(_event: IpcRendererEvent, namespace:', 'dispatch(namespace:']
  ],
  'call-service.ts': [
    [
      "import { formatError } from '@shared/utils/errors'",
      `import { formatError } from '@shared/utils/errors'
import { invokeLeagueCall } from '${IPC_MODULE}'`
    ],
    [
      `    const result: IpcMainDataType<T> = await window.electron.ipcRenderer.invoke(
      'akariCall',
      namespace,
      fnName,
      ...args
    )`,
      '    const result = await invokeLeagueCall<IpcMainDataType<T>>(namespace, fnName, ...args)'
    ]
  ]
}

const NATIVE_STATE_TRANSFORMS = [
  ['  raw: boolean\n}', '  raw: boolean\n  revision?: number\n}'],
  [
    '  async sync(namespace: string, stateId: string, store: any) {',
    `  async sync(namespace: string, stateId: string, store: any) {
    // Tauri can deliver a state event before the older initial response.
    const revisions = new Map<string, number>()
    const accept = (key: string, revision?: number) => {
      if (revision === undefined) return true
      if (revision < (revisions.get(key) ?? -1)) return false
      revisions.set(key, revision)
      return true
    }
    const decode = (key: string, value: unknown) => {
      const championIds = key === 'currentPickableChampionIds' ||
        key === 'currentBannableChampionIds' || key === 'disabledChampionIds'
      if (namespace === 'league-client-main' && stateId === 'champSelect' && championIds &&
          Array.isArray(value) && value.every(Number.isSafeInteger)) {
        return new Set(value)
      }
      return value
    }`
  ],
  [
    '(path, value, { action, raw }) => {',
    `(path, value, { action, raw, revision }) => {
        if (!accept(path, revision)) return
        value = decode(path, value)`
  ],
  [
    'Object.entries(initial).forEach(([key, { value, config }]) => {',
    `Object.entries(initial).forEach(([key, { value, config }]) => {
      if (!accept(key, config.revision)) return
      value = decode(key, value)`
  ]
]

const DRAG_REGIONS = [
  [
    'src-main-window/components/titlebar/MainWindowTitlebar.vue',
    'class="shard-area"',
    '@mousedown.self.left',
    'main'
  ],
  [
    'src-aux-window/components/AuxWindowTitlebar.vue',
    'class="flex h-full flex-1 items-center px-2 pt-1 pb-0.5 transition-all"',
    '@mousedown.left',
    'aux'
  ],
  [
    'src-opgg-window/opgg/OpggWindowTitlebar.vue',
    'class="box-border flex h-full flex-1 items-center pt-1 pr-1 pb-0.5 pl-2 transition-all duration-300"',
    '@mousedown.left',
    'opgg'
  ]
]

const UPDATE_BUTTON_TRANSFORMS = [
  [
    `    <NButton v-if="update" size="small" type="primary" tag="a" :href="update.url"
      target="_blank" rel="noopener noreferrer">
      {{ (english ? 'Download ' : '下载 ') + update.version }}`,
    `    <NButton v-if="update" size="small" type="primary" :loading="downloading"
      :disabled="status === 'checking'" @click="openDownload">
      {{ (english ? 'Download ' : '下载 ') + update }}`
  ],
  ["import { leagueUpdate, leagueUpdateFeed } from './league-update'\n", ''],
  [
    'const update = shallowRef<ReturnType<typeof leagueUpdate>>(null)',
    `const update = shallowRef<string | null>(null)
const downloading = ref(false)`
  ],
  [
    "import { NButton } from 'naive-ui'",
    `import { NButton } from 'naive-ui'
import { useInstance } from '@renderer-shared/shards'
import { AkariIpcRenderer } from '@renderer-shared/shards/ipc'`
  ],
  ['const app = useAppCommonStore()', 'const app = useAppCommonStore()\nconst ipc = useInstance(AkariIpcRenderer)'],
  [
    "  if (status.value === 'checking') return",
    "  if (status.value === 'checking' || downloading.value) return"
  ],
  [
    `    const response = await fetch(leagueUpdateFeed(app.version), {
      cache: 'no-store', credentials: 'omit', redirect: 'error', signal: AbortSignal.timeout(6000)
    })
    if (!response.ok) throw new Error('League manifest unavailable')
    update.value = leagueUpdate(await response.json(), app.version)`,
    `    const result = await ipc.call<{ availableVersion: string | null }>('league-update-main', 'check')
    update.value = result.availableVersion`
  ],
  [
    'onMounted(() => {',
    `async function openDownload() {
  const version = update.value
  if (!version || downloading.value || status.value === 'checking') return
  downloading.value = true
  try {
    await ipc.call('league-update-main', 'openDownload', version)
  } catch {
    update.value = null
    status.value = 'error'
  } finally {
    downloading.value = false
  }
}

onMounted(() => {`
  ]
]

function nativeRendererPlugin(sourceRoot) {
  const ipcDirectory = join(sourceRoot, 'src/renderer-shared/shards/ipc').replaceAll('\\', '/')
  const transforms = new Map(
    Object.entries(IPC_TRANSFORMS).map(([file, patches]) => [`${ipcDirectory}/${file}`, patches])
  )
  const piniaSync = join(sourceRoot, 'src/renderer-shared/shards/pinia-mobx-utils/pinia-state-sync.ts')
  transforms.set(piniaSync.replaceAll('\\', '/'), NATIVE_STATE_TRANSFORMS)
  const updateButton = join(sourceRoot, 'src/renderer/src-main-window/views/league-overview/LeagueUpdateButton.vue')
  transforms.set(updateButton.replaceAll('\\', '/'), UPDATE_BUTTON_TRANSFORMS)
  const resources = join(sourceRoot, 'src/renderer-shared/providers/game-resource/akari.ts')
  const resolveFromPath = 'akari://league-client${normalizeLcuPath(normalizedSource)}'
  const resolveFromUrl = 'akari://league-client${normalizeLcuPath(`${url.pathname}${url.search}${url.hash}`)}'
  transforms.set(resources.replaceAll('\\', '/'), [
    ['(?:https?:|image:', '(?:https?:|akari:|image:'],
    [resolveFromPath, resolveFromPath.replace('akari://league-client', GAME_ART_ORIGIN)],
    [resolveFromUrl, resolveFromUrl.replace('akari://league-client', GAME_ART_ORIGIN)]
  ])
  const client = join(sourceRoot, 'src/renderer-shared/shards/league-client/index.ts')
  transforms.set(client.replaceAll('\\', '/'), [
    ["return new URL(uri, 'akari://league-client').href", `return new URL(uri, '${GAME_ART_ORIGIN}').href`]
  ])
  const opggRoot = join(sourceRoot, 'src/renderer/src-opgg-window').replaceAll('\\', '/')
  const opggView = `${opggRoot}/opgg/OpggView.vue`
  const opggHtml = join(sourceRoot, 'src/renderer', WINDOW_ENTRIES.opgg).replaceAll('\\', '/')
  transforms.set(`${opggRoot}/App.vue`, [
    ["import { provideOpgg } from './opgg/context'\n", ''],
    ['\nprovideOpgg()\n', '\n']
  ])
  transforms.set(`${opggRoot}/shards/index.ts`, [
    ["import { OpggRenderer } from './opgg'\n", ''],
    ['manager.use(OpggRenderer)\n', '']
  ])
  for (const [relative, anchor, gesture, label] of DRAG_REGIONS) {
    const file = join(sourceRoot, 'src/renderer', relative).replaceAll('\\', '/')
    transforms.set(file, [
      [anchor, `${anchor} ${gesture}="startNativeDrag"`],
      [
        '<script setup lang="ts">',
        `<script setup lang="ts">
import { useInstance as useNativeShard } from '@renderer-shared/shards'
import { AkariIpcRenderer as NativeIpc } from '@renderer-shared/shards/ipc'

const nativeWindowIpc = useNativeShard(NativeIpc)
const startNativeDrag = () => nativeWindowIpc.call('window-manager-main/${label}-window', 'startDragging')`
      ]
    ])
  }

  return {
    name: 'mgs-league-native-renderer',
    enforce: 'pre',
    resolveId(id) {
      if (id === IPC_MODULE) return IPC_SOURCE
    },
    transform(source, id) {
      const normalized = id.replaceAll('\\', '/')
      if (normalized === opggView) {
        if (!source.includes('const { currentTab } = useOpgg()')) {
          throw new Error(`Unexpected OP.GG view in ${id}`)
        }
        return { code: readFileSync(OPGG_COMPANION, 'utf8'), map: null }
      }
      const patches = transforms.get(normalized)
      if (!patches) return

      let code = source.replaceAll('\r\n', '\n')
      for (const [anchor, replacement] of patches) {
        code = replaceOnce(code, anchor, replacement, id)
      }
      return { code, map: null }
    },
    transformIndexHtml(html, { filename }) {
      const opgg = filename.replaceAll('\\', '/') === opggHtml
      if (opgg) {
        html = replaceOnce(html, "default-src https: ws: 'self' data: blob: ;", "default-src 'self' data: blob:;", filename)
        html = replaceOnce(html, "connect-src https: ws: 'self';", "connect-src 'self';", filename)
      }
      const imageSources = opgg ? "data: blob: 'self'" : "https: akari: data: blob: 'self'"
      return replaceOnce(
        html,
        'connect-src ',
        `img-src ${imageSources} ${GAME_ART_ORIGIN}; connect-src ipc: http://ipc.localhost `,
        filename
      )
    }
  }
}

const DEVELOPMENT_PLUGINS = new Set([
  'vite-plugin-inspect',
  'vite-plugin-vue-inspector',
  'vite-plugin-vue-inspector:post',
  'vite-plugin-vue-devtools'
])

export function createNativeRendererConfig(renderer, sourceRoot, outDir) {
  const root = join(sourceRoot, 'src/renderer')
  const input = Object.fromEntries(
    Object.entries(WINDOW_ENTRIES).map(([label, filename]) => [label, join(root, filename)])
  )
  const { rollupOptions, rolldownOptions, ...buildOptions } = renderer.build ?? {}
  const plugins = (renderer.plugins ?? [])
    .flat(Infinity)
    .filter((plugin) => !DEVELOPMENT_PLUGINS.has(plugin?.name))

  return {
    ...renderer,
    configFile: false,
    root,
    base: './',
    plugins: [nativeRendererPlugin(sourceRoot), ...plugins],
    build: {
      ...buildOptions,
      outDir,
      emptyOutDir: false,
      rolldownOptions: { ...rollupOptions, ...rolldownOptions, input }
    }
  }
}

async function buildRenderer(args) {
  if (args.length !== 2) {
    throw new Error(
      'Usage: node build.mjs <staged League source directory> <frontendDist directory>'
    )
  }
  const sourceRoot = await realpath(args[0])
  const outDir = resolve(args[1])
  const outputFromSource = relative(sourceRoot, outDir)
  if (
    !isAbsolute(outputFromSource) &&
    outputFromSource !== '..' &&
    !outputFromSource.startsWith(`..${sep}`)
  ) {
    throw new Error(
      'The native renderer output directory must be outside the staged League source.'
    )
  }
  // The desktop bundle embeds this full directory. Reusing one with old hashed
  // chunks would retain obsolete assets in each new installation.
  const oldFiles = await readdir(outDir).catch((error) => {
    if (error.code === 'ENOENT') return []
    throw error
  })
  if (oldFiles.length) {
    throw new Error('The native renderer output must be a new or empty directory.')
  }

  process.chdir(sourceRoot)
  process.env.NODE_ENV = 'production'
  const sourceRequire = createRequire(join(sourceRoot, 'package.json'))
  const vite = await import(pathToFileURL(sourceRequire.resolve('vite')).href)
  // The pinned config uses the CommonJS directory binding for HTML entry paths.
  // The runner can supply it without writing a bundled config into the dependency cache.
  const { module: loaded } = await vite.runnerImport(join(sourceRoot, 'electron.vite.config.ts'), {
    root: sourceRoot,
    define: { __dirname: JSON.stringify(sourceRoot) }
  })
  if (!loaded.default?.renderer) {
    throw new Error('The staged League source has no renderer configuration.')
  }

  await vite.build(createNativeRendererConfig(loaded.default.renderer, sourceRoot, outDir))
  await Promise.all(Object.values(WINDOW_ENTRIES).map((filename) => stat(join(outDir, filename))))
  await copyFile(
    join(sourceRoot, 'src/renderer-shared/assets/ico/league.ico'),
    join(outDir, 'league.ico')
  )
  await writeFile(join(outDir, 'resg-display.js'), resgDisplayProgram(), 'utf8')
  console.log(`Native League renderer built at ${outDir}`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await buildRenderer(process.argv.slice(2))
}
