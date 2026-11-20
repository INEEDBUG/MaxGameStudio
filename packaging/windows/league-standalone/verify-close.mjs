import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createServer } from 'node:net'
import { basename, isAbsolute, relative } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const [executable, profile] = process.argv.slice(2)
const temporaryPath = profile && process.env.RUNNER_TEMP && relative(process.env.RUNNER_TEMP, profile)
assert.ok(
  process.platform === 'win32' && process.env.GITHUB_ACTIONS === 'true' &&
  process.env.RUNNER_ENVIRONMENT === 'github-hosted' &&
  executable && isAbsolute(executable) && basename(executable) === 'MaxGameStudioLeagueToolbox.exe' &&
  temporaryPath && !isAbsolute(temporaryPath) && !temporaryPath.startsWith('..'),
  'Packaged close verification requires an isolated Windows runner profile.'
)

async function waitFor(step, timeout, test) {
  const until = Date.now() + timeout
  do {
    const result = await test()
    if (result) return result
    await delay(150)
  } while (Date.now() < until)
  throw new Error(`Timed out waiting for ${step}.`)
}

async function unusedPort() {
  const server = createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  const { port } = server.address()
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()))
  return port
}

const closeSelector = '.traffic-buttons .close'

async function closeInstalledToolbox(remember) {
  const port = await unusedPort()
  const app = spawn(executable, [
    `--user-data-dir=${profile}`,
    `--remote-debugging-port=${port}`,
    '--remote-debugging-address=127.0.0.1'
  ], { windowsHide: true, stdio: 'ignore' })
  let launchError
  app.on('error', error => { launchError = error })
  let socket
  try {
    const debugUrl = await waitFor('the packaged main window', 45_000, async () => {
      if (launchError) throw launchError
      assert.equal(app.exitCode, null, 'The packaged toolbox quit before loading its main window.')
      const response = await fetch(`http://127.0.0.1:${port}/json/list`, {
        signal: AbortSignal.timeout(1000),
      }).catch(() => null)
      if (!response?.ok) return null
      const targets = await response.json()
      return targets.find(target => target.type === 'page' && /\/main-window\.html(?:[#?]|$)/.test(target.url))
        ?.webSocketDebuggerUrl
    })
    const url = new URL(debugUrl)
    assert.equal(url.origin, `ws://127.0.0.1:${port}`)
    socket = new WebSocket(url)
    await waitFor('the local test channel', 5_000, () => socket.readyState === WebSocket.OPEN)

    let sequence = 0
    const evaluate = expression => new Promise((resolve, reject) => {
      const id = ++sequence
      const finish = callback => value => {
        clearTimeout(timeout)
        socket.removeEventListener('message', receive)
        callback(value)
      }
      const receive = event => {
        const message = JSON.parse(event.data)
        if (message.id !== id) return
        if (message.error || message.result.exceptionDetails) {
          finish(reject)(new Error('The packaged renderer could not complete the close action.'))
        } else {
          finish(resolve)(message.result.result.value)
        }
      }
      const timeout = setTimeout(finish(reject), 5_000, new Error('No packaged renderer response.'))
      socket.addEventListener('message', receive)
      socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true } }))
    })

    await waitFor('the real title-bar close button', 30_000, () =>
      evaluate(`Boolean(document.querySelector(${JSON.stringify(closeSelector)}))`))
    // Defer the click so the test command can return before the app exits.
    await evaluate(`void setTimeout(() => document.querySelector(${JSON.stringify(closeSelector)}).click(), 100)`)

    if (remember) {
      const dialog = "document.querySelector('.close-confirmation-header')"
      const title = await waitFor('the close confirmation', 10_000, () =>
        evaluate(`${dialog}?.textContent?.trim()`))
      assert.equal(title, '关闭英雄联盟工具箱')
      const choices = await evaluate(`(() => {
        const card = ${dialog}.closest('.n-card')
        return Array.from(card.querySelectorAll('.n-radio'), item => item.textContent.trim())
      })()`)
      assert.deepEqual(choices, ['隐藏到系统托盘', '彻底关闭'])
      await evaluate(`(() => {
        const card = ${dialog}.closest('.n-card')
        Array.from(card.querySelectorAll('.n-radio')).find(item => item.textContent.trim() === '彻底关闭').click()
        card.querySelector('.n-checkbox').click()
        const confirm = Array.from(card.querySelectorAll('button')).find(item => item.textContent.trim() === '确定')
        setTimeout(() => confirm.click(), 100)
      })()`)
    }
    await waitFor('the toolbox process to quit', 20_000, () => app.exitCode !== null || app.signalCode !== null)
    assert.equal(app.exitCode, 0, 'The installed toolbox did not quit cleanly.')
  } finally {
    socket?.close()
    if (app.exitCode === null && app.signalCode === null) app.kill()
  }
}

await closeInstalledToolbox(true)
await closeInstalledToolbox(false)
console.log('LEAGUE_CLOSE_DIALOG_AND_REMEMBERED_EXIT_VERIFIED')
