import assert from 'node:assert/strict'
import { createHash, createPublicKey, verify } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { basename, isAbsolute } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { isDeepStrictEqual } from 'node:util'
import { MAIN_UPDATE_PRODUCT, mainInstallerNames, mainUpdateChannel } from '../../../frontend/src/utils/mainUpdateIdentity.mjs'
import { leagueUpdate, leagueUpdateFeed } from '../league-standalone/league-update.ts'

const repository = 'INEEDBUG/MaxGameStudio'
const rawFeeds = `https://raw.githubusercontent.com/${repository}/updater`
const mainConfig = new URL('../../../frontend/src-tauri/tauri.conf.json', import.meta.url)
const binaries = {
  main: 'cs2-insight-agent-desktop.exe',
  league: 'MaxGameStudioLeagueToolbox.exe',
}
const identifiers = {
  main: 'com.cs2insightagent.app',
  league: 'com.maxgamestudio.league.toolbox',
}
const mainPages = ['/', '/index.html']
const leaguePages = ['/main-window.html']
const ipcTimeout = 8000

function base64(value, expectedLength) {
  assert.equal(typeof value, 'string', 'Missing Minisign data.')
  const bytes = Buffer.from(value, 'base64')
  assert.equal(bytes.toString('base64') === value, true, 'Invalid Minisign encoding.')
  if (expectedLength !== undefined) assert.equal(bytes.length, expectedLength, 'Incorrect Minisign record size.')
  return bytes
}

export function verifyMainSignatureComment(encodedSignature, encodedKey, filename) {
  const [, publicKey] = base64(encodedKey).toString('utf8').trimEnd().split(/\r?\n/)
  const [, signature, trustedComment, globalSignature] = base64(encodedSignature).toString('utf8').trimEnd().split(/\r?\n/)
  const key = base64(publicKey, 42)
  const record = base64(signature, 74)
  assert.ok(record.subarray(2, 10).equals(key.subarray(2, 10)), 'Main signature uses an unexpected key.')
  assert.match(record.subarray(0, 2).toString('ascii'), /^E[dD]$/)
  const prefix = 'trusted comment: '
  assert.ok(trustedComment?.startsWith(prefix), 'Missing signed Main installer comment.')
  const comment = trustedComment.slice(prefix.length)
  assert.deepEqual(comment.split('\t').filter(field => field.startsWith('file:')), [`file:${filename}`])
  const spkiPrefix = Buffer.from('302a300506032b6570032100', 'hex')
  const verificationKey = createPublicKey({ key: Buffer.concat([spkiPrefix, key.subarray(10)]), type: 'spki', format: 'der' })
  // The global signature authenticates the signed comment and detached signature.
  // Verifying that detached signature against installer bytes is download-time work.
  assert.ok(verify(null, Buffer.concat([record.subarray(10), Buffer.from(comment)]),
    verificationKey, base64(globalSignature, 64)), 'Main installer signature comment did not verify.')
}

export function validateMainFeed(manifest, current, publicKey) {
  const channel = mainUpdateChannel(current)
  assert.ok(channel && mainUpdateChannel(manifest?.version) === channel, 'Unexpected Main feed channel.')
  assert.equal(manifest.product, MAIN_UPDATE_PRODUCT, 'Unexpected Main feed product.')
  assert.equal(manifest.channel, channel)
  assert.equal(manifest.tag_name, `v${manifest.version}`)
  const platform = manifest.platforms?.['windows-x86_64']
  const allowed = mainInstallerNames(manifest.version)
  const filename = allowed.find(name => platform?.url ===
    `https://github.com/${repository}/releases/download/${manifest.tag_name}/${name}`)
  assert.ok(filename, 'Unexpected Main Windows installer link.')
  verifyMainSignatureComment(platform.signature, publicKey, filename)
  const parts = value => value.replace('-beta.', '.').split('.').map(BigInt)
  const installed = parts(current)
  const difference = parts(manifest.version).map((part, index) => part - installed[index]).find(part => part !== 0n)
  return difference > 0n ? manifest.version : null
}

export function validateLeagueFeed(manifest, current) {
  const channel = mainUpdateChannel(current)
  assert.ok(channel && manifest?.channel === channel, 'Unexpected League feed channel.')
  assert.match(manifest.sha256 ?? '', /^[a-fA-F0-9]{64}$/, 'Missing League installer SHA-256.')
  return leagueUpdate(manifest, current)?.version ?? null
}

async function waitFor(description, milliseconds, predicate) {
  const stop = Date.now() + milliseconds
  do {
    const value = await predicate()
    if (value) return value
    await delay(150)
  } while (Date.now() < stop)
  const detail = typeof description === 'function' ? description() : description
  throw new Error(`Timed out waiting for ${detail}.`)
}

async function readFeed(url) {
  const response = await fetch(url, {
    headers: { Accept: 'application/json', 'Cache-Control': 'no-cache' },
    redirect: 'error', signal: AbortSignal.timeout(ipcTimeout),
  })
  assert.equal(response.status, 200, 'The public feed could not be read.')
  let json = ''
  for await (const part of response.body.pipeThrough(new TextDecoderStream())) {
    json += part
    assert.ok(json.length <= 64 * 1024, 'The public feed is unexpectedly large.')
  }
  return JSON.parse(json)
}

function browserErrorKind(exception) {
  const name = exception?.className
  return ['Error', 'EvalError', 'ReferenceError', 'TypeError', 'SyntaxError'].includes(name) ? name : 'evaluation-error'
}

export function createCdpSession(socket) {
  let sequence = 0
  const send = (method, params, milliseconds = 25_000) => new Promise((resolve, reject) => {
    const id = ++sequence
    const finish = (callback, value) => {
      clearTimeout(timeout)
      socket.removeEventListener('message', receive)
      callback(value)
    }
    const receive = ({ data }) => {
      let reply
      try { reply = JSON.parse(data) } catch { return }
      if (reply?.id !== id) return
      if (reply.error || !reply.result) {
        finish(reject, new Error('The installed updater debugger rejected the request.'))
      } else {
        finish(resolve, reply.result)
      }
    }
    const timeout = setTimeout(() => finish(reject, new Error('The installed updater debugger request timed out.')), milliseconds)
    socket.addEventListener('message', receive)
    try {
      socket.send(JSON.stringify({ id, method, params }))
    } catch {
      finish(reject, new Error('The installed updater debugger disconnected.'))
    }
  })
  const evaluate = async (expression, milliseconds) => {
    const reply = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, silent: true }, milliseconds)
    if (reply.exceptionDetails) {
      throw new Error(`The installed updater evaluation failed in WebView2 (${browserErrorKind(reply.exceptionDetails.exception)}).`)
    }
    assert.ok(reply.result, 'The installed updater debugger returned no evaluation result.')
    return reply.result.value
  }
  return { send, evaluate }
}

async function connectPage(role, port) {
  const paths = role === 'main' ? mainPages : leaguePages
  const target = await waitFor('the installed app WebView2 page', 45_000, async () => {
    const response = await fetch(`http://127.0.0.1:${port}/json/list`, {
      redirect: 'error', signal: AbortSignal.timeout(1000),
    }).catch(() => null)
    if (!response?.ok) return null
    const pages = (await response.json()).filter(page => {
      const url = new URL(page.url)
      return page.type === 'page' && url.origin === 'http://tauri.localhost' && paths.includes(url.pathname)
    })
    assert.ok(pages.length <= 1, 'The local debugger exposed ambiguous app pages.')
    return pages[0]
  })
  const debuggerUrl = new URL(target.webSocketDebuggerUrl)
  assert.equal(debuggerUrl.origin, `ws://127.0.0.1:${port}`)
  assert.match(debuggerUrl.pathname, /^\/devtools\/page\/[a-zA-Z0-9-]+$/)
  assert.equal(debuggerUrl.search + debuggerUrl.hash + debuggerUrl.username + debuggerUrl.password, '')
  const socket = new WebSocket(debuggerUrl)
  try {
    await waitFor('the loopback WebView2 debugger', 5_000, () => {
      assert.ok(socket.readyState !== WebSocket.CLOSED, 'The local debugger disconnected.')
      return socket.readyState === WebSocket.OPEN
    })
    return { socket, cdp: createCdpSession(socket) }
  } catch (error) {
    socket.close()
    throw error
  }
}

const identityCheck = `(async () => {
  const invoke = (command) => window.__TAURI_INTERNALS__.invoke(command)
  return { id: await invoke('plugin:app|identifier'), version: await invoke('plugin:app|version') }
})()`

function mainCheck(allowDowngrades) {
  return `(async () => {
    const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args)
    const update = await invoke('plugin:updater|check', { timeout: ${ipcTimeout}, allowDowngrades: ${allowDowngrades} })
    if (update === null) return null
    await invoke('plugin:resources|close', { rid: update.rid })
    return { currentVersion: update.currentVersion, version: update.version, rawJson: update.rawJson }
  })()`
}

const leagueReadiness = `(() => {
  if (document.readyState === 'loading') return 'document-loading'
  if (typeof window.__TAURI__?.core?.invoke !== 'function') return 'bridge-unavailable'
  return document.querySelector('#app[data-v-app]') ? 'ready' : 'mount-pending'
})()`

const leagueIpcFailures = ['ipc-denied', 'ipc-command-missing', 'ipc-invalid-arguments', 'ipc-rejected']

function leagueCall(namespace, method) {
  const args = JSON.stringify({ namespace, method, args: [] })
  return `(async () => {
    try {
      return { response: await window.__TAURI__.core.invoke('league_call', ${args}) }
    } catch (error) {
      const message = typeof error === 'string' ? error : error?.message
      if (/not allowed|permission denied/i.test(message)) return { failure: 'ipc-denied' }
      if (/command .*not found/i.test(message)) return { failure: 'ipc-command-missing' }
      if (/invalid args/i.test(message)) return { failure: 'ipc-invalid-arguments' }
      return { failure: 'ipc-rejected' }
    }
  })()`
}

const leagueBootstrapShards = new Set([
  'akari-ipc-renderer', 'akari-api-renderer', 'app-common-renderer', 'auto-gameflow-renderer',
  'auto-misc-renderer', 'auto-select-renderer', 'client-installation-renderer', 'extra-assets-renderer',
  'game-client-renderer', 'keyboard-shortcuts-renderer', 'league-client-peek-renderer', 'league-client-renderer',
  'league-client-ux-renderer', 'logger-renderer', 'main-window-ui-renderer', 'ongoing-game-renderer',
  'pinia-mobx-utils-renderer', 'player-tabs-renderer', 'renderer-debug-renderer', 'riot-client-renderer',
  'saved-player-renderer', 'self-hosted-lcu-data-renderer', 'self-update-renderer', 'setting-utils-renderer',
  'setup-in-app-scope-renderer', 'sgp-renderer', 'simple-notifications-renderer', 'storage-renderer', 'window-manager-renderer',
])

function leagueStartupEvents() {
  const shards = new Set()
  const errors = new Set()
  const assetFailures = new Set()
  const parsedModules = new Set()
  const bundledScript = input => {
    const url = typeof input === 'string' && URL.parse(input)
    return url?.origin === 'http://tauri.localhost' && /^\/assets\/[-\w.]+\.m?js$/i.test(url.pathname)
      ? url.pathname : null
  }
  const captureError = exception => {
    errors.add(browserErrorKind(exception))
    const description = exception?.description
    if (typeof description !== 'string') return
    const shard = /(?:Failed to initialize shard|Failed to trigger onFinish hook of shard) ([a-z-]+)(?=\s|$)/.exec(description)?.[1]
    if (leagueBootstrapShards.has(shard)) shards.add(shard)
    if (/dynamically imported module|does not provide an export named|Failed to resolve module specifier/i.test(description)) {
      errors.add('module-load')
    }
  }
  const receive = ({ data }) => {
    let event
    try { event = JSON.parse(data) } catch { return }
    if (event?.method === 'Runtime.consoleAPICalled') {
      const { type, args } = event.params ?? {}
      if (type === 'error' && ['MaxGameStudio 无法正确加载：', 'League Akari 无法正确加载：'].includes(args?.[0]?.value)) {
        errors.add('bootstrap-rejected')
        captureError(args[1])
      }
    } else if (event?.method === 'Runtime.exceptionThrown') {
      captureError(event.params?.exceptionDetails?.exception)
    } else if (event?.method === 'Debugger.scriptParsed' || event?.method === 'Debugger.scriptFailedToParse') {
      const script = bundledScript(event.params?.url)
      if (script && event.method === 'Debugger.scriptParsed') parsedModules.add(script)
      else if (script) errors.add('script-parse')
    } else if (event?.method === 'Log.entryAdded') {
      const entry = event.params?.entry
      if (entry?.source === 'security' && /content security policy/i.test(entry.text)) errors.add('csp-blocked')
      if (entry?.source === 'network' && entry.level === 'error' && typeof entry.url === 'string') {
        const url = URL.parse(entry.url)
        if (url?.origin === 'http://tauri.localhost' && /^\/assets\/[-\w.]+\.(?:css|m?js)$/i.test(url.pathname)) {
          assetFailures.add(url.pathname)
        }
      }
    }
  }
  return { receive, summary: () =>
    `shards=${[...shards].sort().join(',') || 'none'} errors=${[...errors].sort().join(',') || 'none'} assetErrors=${assetFailures.size} parsedModules=${parsedModules.size}` }
}

const leagueDocumentProbe = `(async () => {
  const root = document.getElementById('app')
  const shell = !root ? 'missing' : root.childElementCount ? 'populated' : 'empty'
  const scripts = [...document.querySelectorAll('script[type="module"][src]')]
  const entry = scripts.map(script => new URL(script.src)).find(url =>
    url.origin === window.location.origin && /^\\/assets\\/main-[-\\w.]+\\.js$/.test(url.pathname))
  const resources = performance.getEntriesByType('resource').filter(resource => {
    const url = new URL(resource.name)
    return url.origin === window.location.origin && /^\\/assets\\/[-\\w.]+\\.m?js$/i.test(url.pathname)
  }).length
  let entryState = entry ? 'unavailable' : 'missing'
  if (entry) {
    try {
      const response = await fetch(entry, {cache: 'no-store', redirect: 'error', signal: AbortSignal.timeout(3000)})
      const mime = response.headers.get('content-type') || ''
      entryState = !response.ok ? 'http-error' : /\\b(?:javascript|ecmascript)\\b/i.test(mime)
        ? 'javascript' : 'unexpected-content-type'
      await response.body?.cancel()
    } catch { entryState = 'fetch-rejected' }
  }
  return { shell, entryState, resources }
})()`

const leagueEventPermission = `(async () => {
  try {
    const main = window.__TAURI__.webviewWindow.getCurrentWebviewWindow()
    const unlisten = await main.listen('league-ipc-event', () => {})
    await unlisten()
    return 'accepted'
  } catch (error) {
    const message = typeof error === 'string' ? error : error?.message
    return /not allowed|permission denied/i.test(message) ? 'denied' : 'failed'
  }
})()`

export async function verifyLeaguePage(socket, { send, evaluate }, current, readinessTimeout, report = console.log) {
  const { receive, summary } = leagueStartupEvents()
  socket.addEventListener('message', receive)
  try {
    // Runtime and Log replay buffered messages when enabled, including early startup failures.
    const domains = await Promise.allSettled(['Runtime.enable', 'Log.enable'].map(method => send(method, {}, ipcTimeout)))
    try {
      return await readLeagueUpdater(evaluate, current, readinessTimeout)
    } catch (error) {
      // Enable only after failure to collect scripts WebView2 has already parsed.
      const [versionReply, eventReply, documentReply, debuggerReply] = await Promise.allSettled([
        evaluate(leagueCall('app-common-main', 'getVersion'), ipcTimeout),
        evaluate(leagueEventPermission, ipcTimeout),
        evaluate(leagueDocumentProbe, ipcTimeout),
        send('Debugger.enable', {}, ipcTimeout),
      ])
      const reply = versionReply.value
      let version = 'unavailable'
      if (reply?.response?.success === true) version = reply.response.data === current ? 'match' : 'mismatch'
      else if (leagueIpcFailures.includes(reply?.failure)) version = reply.failure
      const events = ['accepted', 'denied', 'failed'].includes(eventReply.value) ? eventReply.value : 'unavailable'
      const streams = domains.filter(result => result.status === 'fulfilled').length
      const { shell, entryState, resources } = documentReply.value ?? {}
      const document = `shell=${['missing', 'empty', 'populated'].includes(shell) ? shell : 'unknown'} ` +
        `entry=${['missing', 'javascript', 'http-error', 'unexpected-content-type', 'fetch-rejected', 'unavailable'].includes(entryState) ? entryState : 'unknown'} ` +
        `resources=${Number.isSafeInteger(resources) && resources >= 0 ? resources : 'unknown'}`
      report(`LEAGUE_STARTUP_DIAGNOSTICS: ${summary()} version=${version} events=${events} ${document} streams=${streams}/2 debugger=${debuggerReply.status}`)
      throw error
    }
  } finally {
    socket.removeEventListener('message', receive)
  }
}

export async function readLeagueUpdater(evaluate, current, readinessTimeout = 15_000) {
  let state = 'not-observed'
  await waitFor(() => `the League main renderer (${state})`, readinessTimeout, async () => {
    const readiness = await evaluate(leagueReadiness)
    assert.ok(['document-loading', 'bridge-unavailable', 'mount-pending', 'ready'].includes(readiness),
      'The League readiness probe returned an invalid result.')
    state = readiness
    return state === 'ready'
  })
  const invoke = async (label, namespace, method) => {
    const reply = await evaluate(leagueCall(namespace, method))
    const failure = reply?.failure
    if (leagueIpcFailures.includes(failure)) {
      throw new Error(`The League ${label} call failed (${failure}).`)
    }
    const response = reply?.response
    assert.ok(response?.success === true || response?.success === false, `The League ${label} response is invalid.`)
    return response
  }
  const version = await invoke('version', 'app-common-main', 'getVersion')
  assert.ok(version.success && version.data === current, 'League reported an unexpected version.')
  return waitFor('the League native updater to become available', 15_000, async () => {
    const result = await invoke('updater', 'league-update-main', 'check')
    if (result.success) {
      assert.ok(result.data && typeof result.data === 'object', 'The League updater response is missing its data.')
      return result.data
    }
    const message = result.error?.message
    if (message === 'League 更新检查正在进行。') return null
    const category = {
      '无法初始化 League 更新检查。': 'feed-initialization',
      '无法连接 League 更新源。': 'feed-transport',
      'League 更新检查超时。': 'feed-timeout',
      'League 更新清单响应过大。': 'feed-size',
    }
    const kind = Object.hasOwn(category, message) ? category[message]
      : /^League 更新源返回 HTTP \d{3}。$/.test(message) ? 'feed-http'
      : typeof message === 'string' && message.startsWith('League 更新清单验证失败。') ? 'feed-manifest'
      : 'check-rejected'
    throw new Error(`The League updater call failed (${kind}).`)
  })
}

export async function verifyLeagueNavigation(evaluate) {
  await waitFor('the installed League Overview and both game icons', 15_000, () => evaluate(`(() => {
    const overview = document.querySelector('.league-overview')
    const sidebarIcon = document.querySelector('img.league-sidebar-mark')
    return !!overview?.querySelector('img.league-game-mark')?.naturalWidth &&
      !!sidebarIcon?.naturalWidth
  })()`))

  const click = async selector => {
    const clicked = await evaluate(`(() => {
      const element = document.querySelector(${JSON.stringify(selector)})
      if (!element) return false
      element.click()
      return true
    })()`)
    assert.equal(clicked, true, 'An installed League navigation action was unavailable.')
  }

  const expectPlayerLookup = () => waitFor('the installed League player lookup view', 10_000,
    () => evaluate(`window.location.hash === '#/player-search' &&
      !!document.querySelector('.league-player-search [role="status"]')?.textContent.trim()`))

  await click('.league-overview [aria-labelledby="league-lookup"] button')
  await expectPlayerLookup()
  await click('[data-key="league-overview"]')
  await waitFor('the installed League Overview', 10_000,
    () => evaluate("window.location.hash === '#/overview' && !!document.querySelector('.league-overview')"))
  await click('[data-key="league-lookup"]')
  await expectPlayerLookup()
}

const mainBackendCheck = `(async () => {
  const invoke = (command) => window.__TAURI_INTERNALS__.invoke(command)
  await invoke('ensure_backend')
  const base = 'http://127.0.0.1:19871/api/'
  const get = (route, headers) => fetch(base + route, { headers, cache: 'no-store', signal: AbortSignal.timeout(5000) })
  const healthResponse = await get('health')
  const health = healthResponse.ok ? await healthResponse.json() : null
  const anonymous = await get('config')
  await anonymous.body?.cancel()
  const token = await invoke('backend_session_token')
  const configResponse = await get('config', { 'X-CS2-Insight-Token': token })
  const config = configResponse.ok ? await configResponse.json() : null
  return {
    healthStatus: healthResponse.status,
    ready: health?.status === 'ok',
    version: health?.version,
    anonymousStatus: anonymous.status,
    configStatus: configResponse.status,
    hasConfig: typeof config?.locale === 'string' && typeof config?.effective_locale === 'string'
  }
})()`

async function verifyInstalled(role, executable, expectedHash, current, portText, mode = 'updates') {
  assert.ok(process.platform === 'win32' && process.env.GITHUB_ACTIONS === 'true' &&
    process.env.RUNNER_ENVIRONMENT === 'github-hosted', 'Installed updater verification requires a disposable Windows runner.')
  assert.ok(Object.hasOwn(binaries, role) && isAbsolute(executable) && basename(executable) === binaries[role])
  assert.ok(['updates', 'idle'].includes(mode), 'Unknown installed app probe mode.')
  assert.match(expectedHash ?? '', /^[a-f0-9]{64}$/)
  const port = Number(portText)
  assert.ok(Number.isInteger(port) && port > 0 && port < 65536 && String(port) === portText)
  const channel = mainUpdateChannel(current)
  assert.ok(channel, 'Expected an exact stable or beta version.')
  const feed = role === 'main' ? `${rawFeeds}/${channel === 'beta' ? 'beta' : 'latest'}.json` : leagueUpdateFeed(current)
  const binary = await readFile(executable)
  assert.equal(createHash('sha256').update(binary).digest('hex'), expectedHash, 'Installed executable changed.')
  assert.ok(binary.includes(identifiers[role]), 'The installed executable lacks the expected product identifier.')
  assert.ok(binary.includes(feed), 'The expected feed is missing from the installed executable.')
  if (mode === 'idle') {
    const { socket, cdp } = await connectPage(role, port)
    try {
      const home = role === 'main'
        ? "!!document.querySelector('#home-title')?.getClientRects().length && !!document.querySelector('#home-tools-title')"
        : "!!document.querySelector('#app[data-v-app] .league-overview')?.getClientRects().length"
      await waitFor('the actual Home view in the process about to be sampled', 30_000, () => cdp.evaluate(home))
    } finally {
      socket.close()
    }
    console.log(`NATIVE_IDLE_SCENE_VERIFIED: ${role} ${current}, visible Home layout in the sampled process.`)
    return
  }
  const manifest = await readFeed(feed)
  let expected
  if (role === 'main') {
    const publicKey = JSON.parse(await readFile(mainConfig, 'utf8')).plugins.updater.pubkey
    assert.ok(binary.includes(publicKey), 'The expected public updater key is missing from the installed Main binary.')
    expected = validateMainFeed(manifest, current, publicKey)
    assert.notEqual(manifest.version, current, 'Main needs a different published version to observe its native manifest metadata.')
  } else {
    expected = validateLeagueFeed(manifest, current)
  }

  const { socket, cdp } = await connectPage(role, port)
  const { evaluate } = cdp
  try {
    if (role === 'main') {
      assert.deepEqual(await evaluate(identityCheck), { id: identifiers[role], version: current })
      // This check-only option exposes metadata for a different, possibly older release.
      // The default check still decides which version the app may offer.
      const witness = await evaluate(mainCheck(true))
      assert.equal(witness?.currentVersion, current)
      assert.equal(witness?.version, manifest.version)
      assert.ok(isDeepStrictEqual(witness?.rawJson, manifest), 'Main returned a different feed response.')
      const normal = await evaluate(mainCheck(false))
      if (expected === null) assert.equal(normal, null, 'Main offered a non-newer release.')
      else assert.ok(isDeepStrictEqual(normal, witness), 'Main did not offer the expected signed release.')
      assert.deepEqual(await evaluate(mainBackendCheck), {
        healthStatus: 200, ready: true, version: current, anonymousStatus: 401, configStatus: 200, hasConfig: true,
      })
    } else {
      const result = await verifyLeaguePage(socket, cdp, current)
      assert.ok(isDeepStrictEqual(result, {
        currentVersion: current, feed: { product: 'maxgamestudio-league', channel, url: feed }, availableVersion: expected,
      }), 'League returned unexpected update feed metadata.')
      await verifyLeagueNavigation(evaluate)
      console.log('NATIVE_LEAGUE_NAVIGATION_VERIFIED: both bundled game icons and both player lookup entries.')
    }
  } finally {
    socket.close()
  }
  console.log(`NATIVE_UPDATER_VERIFIED: ${role} ${current}, ${feed}, offered ${expected ?? 'none'}`)
}

if (import.meta.main) {
  await verifyInstalled(...process.argv.slice(2))
}
