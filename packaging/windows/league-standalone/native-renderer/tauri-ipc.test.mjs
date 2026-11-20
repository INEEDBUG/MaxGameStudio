import assert from 'node:assert/strict'
import test from 'node:test'

import { invokeLeagueCall, listenLeagueEvents } from './tauri-ipc.ts'

function installTauri(t, api) {
  globalThis.__TAURI__ = api
  t.after(() => delete globalThis.__TAURI__)
}

test('forwards calls exclusively through league_call and preserves each response envelope', async (t) => {
  const calls = []
  const responses = [
    { success: true, data: { enabled: true } },
    { success: false, error: { message: 'Settings unavailable' }, isAxiosError: true }
  ]
  installTauri(t, {
    core: {
      async invoke(command, payload) {
        calls.push({ command, payload })
        return responses[calls.length - 1]
      }
    }
  })

  assert.deepEqual(await invokeLeagueCall('settings-main', 'set', 'feature', false), responses[0])
  assert.deepEqual(await invokeLeagueCall('app-common-main', 'getVersion'), responses[1])
  assert.deepEqual(calls, [
    {
      command: 'league_call',
      payload: { namespace: 'settings-main', method: 'set', args: ['feature', false] }
    },
    {
      command: 'league_call',
      payload: { namespace: 'app-common-main', method: 'getVersion', args: [] }
    }
  ])
})

test('propagates invoke rejection to the caller', async (t) => {
  const denied = new Error('Unknown League command')
  installTauri(t, {
    core: {
      async invoke() {
        throw denied
      }
    }
  })

  await assert.rejects(invokeLeagueCall('invalid', 'missing'), (error) => error === denied)
})

test('awaits a current-window subscription and removes it once', async (t) => {
  const received = []
  const names = []
  let deliver
  let detach
  let finishRemoval
  let removals = 0
  installTauri(t, {
    webviewWindow: {
      getCurrentWebviewWindow() {
        return {
          listen(name, callback) {
            names.push(name)
            deliver = callback
            return new Promise((resolve) => {
              detach = () =>
                resolve(() => {
                  removals += 1
                  return new Promise((complete) => {
                    finishRemoval = complete
                  })
                })
            })
          }
        }
      }
    }
  })

  let registered = false
  const pending = listenLeagueEvents((...args) => received.push(args)).then((unsubscribe) => {
    registered = true
    return unsubscribe
  })
  assert.equal(registered, false)
  assert.deepEqual(names, ['league-ipc-event'])
  detach()
  const unsubscribe = await pending

  deliver({ payload: null })
  deliver({ payload: { namespace: 'test', eventName: 'bad', args: 'invalid' } })
  deliver({ payload: { namespace: 'test', eventName: 'valid', args: [false, null, 'data'] } })
  let removed = false
  const removal = unsubscribe().then(() => {
    removed = true
  })
  await unsubscribe()
  deliver({ payload: { namespace: 'test', eventName: 'late', args: [] } })
  assert.equal(removed, false)
  assert.deepEqual(received, [['test', 'valid', false, null, 'data']])
  assert.equal(removals, 1)
  finishRemoval()
  await removal
  assert.equal(removed, true)
})
