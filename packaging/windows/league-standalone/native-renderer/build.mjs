import { copyFile, readdir, realpath, stat } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { isAbsolute, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const IPC_MODULE = 'virtual:mgs-league-native-ipc'
const IPC_SOURCE = fileURLToPath(new URL('./tauri-ipc.ts', import.meta.url))
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

function nativeRendererPlugin(sourceRoot) {
  const ipcDirectory = join(sourceRoot, 'src/renderer-shared/shards/ipc').replaceAll('\\', '/')
  const transforms = new Map(
    Object.entries(IPC_TRANSFORMS).map(([file, patches]) => [`${ipcDirectory}/${file}`, patches])
  )

  return {
    name: 'mgs-league-native-renderer',
    enforce: 'pre',
    resolveId(id) {
      if (id === IPC_MODULE) return IPC_SOURCE
    },
    transform(source, id) {
      const patches = transforms.get(id.replaceAll('\\', '/'))
      if (!patches) return

      let code = source.replaceAll('\r\n', '\n')
      for (const [anchor, replacement] of patches) {
        code = replaceOnce(code, anchor, replacement, id)
      }
      return { code, map: null }
    },
    transformIndexHtml(html, { filename }) {
      return replaceOnce(html, 'connect-src ', 'connect-src ipc: http://ipc.localhost ', filename)
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
  console.log(`Native League renderer built at ${outDir}`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await buildRenderer(process.argv.slice(2))
}
