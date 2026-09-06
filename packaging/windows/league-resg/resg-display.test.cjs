// Isolated DOM fixtures; never reads LCU or the user's browser/profile.
const { createRequire } = require('node:module')
const path = require('node:path')
const assert = require('node:assert/strict')
const { test } = require('node:test')
const dependencies = createRequire(path.resolve(__dirname, '../../../frontend/package.json'))
const { JSDOM } = dependencies('jsdom')
const code = require('node:module').stripTypeScriptTypes(
  require('node:fs').readFileSync(path.join(__dirname, 'resg-display.ts'), 'utf8'),
  { mode: 'strip' }
)
const resgDisplayScript = new Function(
  code.replace('export function', 'function') + '\nreturn resgDisplayScript'
)()
function fixture() {
  const dom = new JSDOM(
    `<div id="app"><h1>Fixture champion</h1>
    <select class="augment-quality-select"><option value="all">All</option>
    <option value="1">Silver</option><option value="2">Gold</option><option value="3">Prismatic</option></select>
    <div class="augment-stat-list"></div><div id="equipment"></div>
    <div id="huge-combinations">Original detailed combinations</div></div>`,
    {
      url: 'https://www.bilibilitoy.com/toy/resg/build/v/16.17/champions/63',
      runScripts: 'outside-only'
    }
  )
  const doc = dom.window.document
  const select = doc.querySelector('select')
  const values = [99, 2000, 11000, 950]
  const update = () => {
    doc.querySelector('.augment-stat-list').innerHTML = values
      .map(
        (samples, i) =>
          `<div class="augment-stat-row"><span class="augment-name"><img src="https://example.com/${i}.png" alt="Tier ${select.value} Augment ${i}"><b>Augment ${i}</b></span>
      <span><strong>${80 - i * 10}%</strong><small>${samples.toLocaleString('en-US')} 场</small></span></div>`
      )
      .join('')
  }
  select.addEventListener('change', update)
  update()
  doc.querySelector('#equipment').innerHTML = ['一', '二', '三', '四', '五']
    .map(
      (stage) =>
        `<article class="item-combo-column"><h3>第${stage}件</h3>${[2.5, 9, 11, 4]
          .map(
            (pick, i) =>
              `<div class="item-combo-row"><span><img alt="Item ${i}" src="https://example.com/item${i}.png"></span><strong>${90 - i * 10}%</strong><b>${pick}%</b><em>1,234</em></div>`
          )
          .join('')}</article>`
    )
    .join('')
  return {
    dom,
    doc,
    select,
    run: (mode = 'compact', id = 63, en = false, dark = true) =>
      dom.window.eval(resgDisplayScript(mode, id, en, dark))
  }
}
test('top 3 per quality by numeric samples, builds by pick rate (not win rate)', async () => {
  const { dom, doc, select, run } = fixture()
  const result = await run()
  assert.equal(result.kind, 'compact')
  assert.deepEqual(Array.from(result.augmentCounts), [3, 3, 3])
  assert.deepEqual(Array.from(result.itemCounts), [3, 3, 3, 3, 3])
  const cards = doc.querySelectorAll('#mgs-resg-compact .augments .card')
  for (const [i, card] of Array.from(cards).entries()) {
    assert.match(card.querySelector('.names').textContent, new RegExp(`Tier ${i + 1} Augment 2`))
    assert.equal(card.querySelectorAll('.pick').length, 3)
    assert.match(card.textContent, /11,000 场/)
    assert.match(card.textContent, /胜率 60%/)
  }
  assert.equal(doc.querySelector('.equipment .names').textContent, 'Item 2')
  assert.match(doc.querySelector('.equipment .stats').textContent, /采用率 11%/)
  assert.equal(select.value, 'all')
  assert.equal(doc.body.dataset.mgsResgCompact, '63')
  dom.window.close()
})
test('all view restores original DOM, sort and filter without destructive rewriting', async () => {
  const { dom, doc, select, run } = fixture()
  select.value = '2'
  const original = doc.querySelector('#app')
  const details = doc.querySelector('#huge-combinations')
  await run()
  assert.equal(select.value, '2')
  await run('all')
  assert.equal(doc.querySelector('#app'), original)
  assert.equal(doc.querySelector('#huge-combinations'), details)
  assert.equal(doc.querySelector('#mgs-resg-compact'), null)
  assert.equal(doc.querySelector('#mgs-resg-compact-style'), null)
  assert.equal(doc.body.dataset.mgsResgCompact, undefined)
  dom.window.close()
})
test('unknown markup, invalid percentage, missing samples and wrong champion fail open', async () => {
  for (const kind of ['markup', 'percentage', 'samples', 'champion']) {
    const { dom, doc, run } = fixture()
    if (kind === 'markup') doc.querySelector('select').remove()
    if (kind === 'percentage') doc.querySelector('.item-combo-row b').textContent = '101%'
    if (kind === 'samples') doc.querySelector('.item-combo-row em').textContent = '--'
    assert.equal((await run('compact', kind === 'champion' ? 17 : 63)).kind, 'fallback')
    assert.equal(doc.querySelector('#mgs-resg-compact-style'), null)
    assert.ok(doc.querySelector('#app'))
    dom.window.close()
  }
})
test('champion navigation during extraction cannot publish stale cards', async () => {
  const { dom, doc, select, run } = fixture()
  select.addEventListener(
    'change',
    () => dom.window.history.replaceState({}, '', '/v/16.17/champions/17'),
    { once: true }
  )
  assert.equal((await run()).kind, 'fallback')
  assert.equal(doc.querySelector('#mgs-resg-compact'), null)
  dom.window.close()
})
test('resolves relative game icons against the website base, not the champion route', async () => {
  const { dom, doc, run } = fixture()
  const base = doc.createElement('base')
  base.href = 'https://www.bilibilitoy.com/toy/resg/build/'
  doc.head.append(base)
  doc
    .querySelectorAll('.item-combo-row img')
    .forEach((img) => img.setAttribute('src', 'assets/game/item.png'))
  await run()
  assert.equal(
    doc.querySelector('#mgs-resg-compact .equipment img').src,
    'https://www.bilibilitoy.com/toy/resg/build/assets/game/item.png'
  )
  dom.window.close()
})
test('empty quality does not invent rows and restores an unmounted quality control', async () => {
  const { dom, doc, select, run } = fixture()
  const originalParent = select.parentElement
  select.addEventListener('change', () => {
    if (select.value === '2') {
      doc.querySelector('.augment-stat-list').replaceChildren()
      select.remove()
    } else if (!select.isConnected) {
      originalParent.prepend(select)
    }
  })
  const result = await run()
  assert.equal(result.kind, 'compact')
  assert.deepEqual(Array.from(result.augmentCounts), [3, 0, 3])
  assert.equal(select.value, 'all')
  assert.equal(select.isConnected, true)
  assert.match(
    doc.querySelectorAll('#mgs-resg-compact .augments .card')[1].textContent,
    /暂无足够样本/
  )
  dom.window.close()
})
test('light and dark summaries remain reversible and contain text rather than injected HTML', async () => {
  const { dom, doc, run } = fixture()
  doc.querySelector('h1').textContent = '<img src=x onerror=alert(1)>'
  for (const dark of [true, false]) {
    assert.equal((await run('compact', 63, true, dark)).kind, 'compact')
    assert.equal(doc.querySelector('#mgs-resg-compact h1 img'), null)
    assert.match(doc.querySelector('#mgs-resg-compact').textContent, /Popular augments/)
    assert.ok(
      doc
        .querySelector('#mgs-resg-compact-style')
        .textContent.includes(dark ? '#eeeeee' : '#172033')
    )
    assert.equal(doc.querySelectorAll('#mgs-resg-compact').length, 1)
    await run('all')
  }
  dom.window.close()
})
