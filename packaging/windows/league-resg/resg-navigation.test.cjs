const assert = require('node:assert/strict')
const { readFileSync } = require('node:fs')
const { createRequire, stripTypeScriptTypes } = require('node:module')
const path = require('node:path')
const { test } = require('node:test')

const dependencies = createRequire(path.resolve(__dirname, '../../../frontend/package.json'))
const { JSDOM } = dependencies('jsdom')
const policy = stripTypeScriptTypes(readFileSync(path.join(__dirname, 'resg-policy.ts'), 'utf8'))
const { isResgContent, resgChampionUrl, resgNavigationStep } = new Function(
  policy.replace(/^export /gm, '') + '\nreturn {isResgContent, resgChampionUrl, resgNavigationStep}'
)()

test('queries the current first-party RESG hero directly', () => {
  assert.equal(resgChampionUrl(63), 'https://www.resg.top/v/latest/champions/63')
  assert.equal(isResgContent('https://www.resg.top/v/16.22/champions/63'), true)
  assert.equal(isResgContent('https://www.bilibilitoy.com/toy/resg/build/index.html'), false)
})

test('waits for the loaded hero on the matching versioned route', () => {
  const dom = new JSDOM('<div id="app"><h1>RESG</h1></div>', {
    url: 'https://www.resg.top/v/latest/champions/63', runScripts: 'outside-only'
  })
  const read = () => JSON.parse(JSON.stringify(dom.window.eval(resgNavigationStep(63))))
  try {
    assert.deepEqual(read(), { kind: 'waiting' })
    dom.window.history.replaceState(null, '', '/v/16.22/champions/17')
    dom.window.document.querySelector('#app').innerHTML = `
      <section class="entity-hero"><div class="entity-identity"><h1>复仇焰魂</h1></div></section>`
    assert.deepEqual(read(), { kind: 'waiting' })
    dom.window.history.replaceState(null, '', '/v/16.22/champions/63')
    assert.deepEqual(read(), { kind: 'ready', name: '复仇焰魂' })
  } finally {
    dom.window.close()
  }
})
