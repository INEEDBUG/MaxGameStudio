import assert from 'node:assert/strict'
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
