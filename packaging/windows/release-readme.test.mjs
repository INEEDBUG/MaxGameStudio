import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

import { rewriteReleaseReadme } from './release-readme.mjs'

const releases = 'https://github.com/INEEDBUG/MaxGameStudio/releases'
const mib = 1024 * 1024
const read = (name) => readFileSync(new URL('../../' + name, import.meta.url), 'utf8')

function release(version, { league = false, size, checksum = true } = {}) {
  const tag = (league ? 'league-v' : 'v') + version
  const name = (league ? 'MaxGameStudioLeague_' : 'MaxGameStudio_') + version + '_x64-setup.exe'
  const url = releases + '/download/' + tag + '/' + name
  return {
    version, tag, installer: { name, size, browser_download_url: url },
    ...(checksum && { checksum: { browser_download_url: url + '.sha256' } }),
  }
}

const targets = {
  mainBeta: release('3.9.0-beta.3', { size: 40.25 * mib }),
  leagueBeta: release('1.5.0-beta.7', { league: true, size: 100.25 * mib }),
  mainStable: release('3.8.0', { size: 137.5 * mib, checksum: false }),
}

for (const locale of [
  {
    file: 'README.md', heading: 'Downloads', next: 'Features', label: 'Download EXE',
    oldHeader: 'Download / program size', newHeader: 'Download size',
    policy: /one beta and up to three stable releases for each app.*Git tags and source history/,
    historical: /Historical installed sizes.*MaxGameStudio .*MiB.*League .*MiB/,
    mainNotes: 'Main app release notes', leagueNotes: 'League release notes',
  },
  {
    file: 'README.zh-CN.md', heading: '下载', next: '功能', label: '下载 EXE',
    oldHeader: '下载体积 / 程序占用', newHeader: '下载体积',
    policy: /每款软件保留 1 个测试版和最多 3 个正式版.*Git 标签和源代码历史/,
    historical: /历史实测程序占用.*MaxGameStudio .*MiB.*League .*MiB/,
    mainNotes: '主程序发布说明', leagueNotes: '工具箱发布说明',
  },
]) {
  test(locale.file + ' refreshes exact downloads and beta notes while preserving surrounding content', () => {
    const original = read(locale.file)
    const updated = rewriteReleaseReadme(original, targets)
    const lines = updated.split(/\r?\n/)
    const prefix = '## ' + locale.heading
    const suffix = '## ' + locale.next
    assert.equal(updated.slice(0, updated.indexOf(prefix)), original.slice(0, original.indexOf(prefix)))
    assert.equal(updated.slice(updated.indexOf(suffix)), original.slice(original.indexOf(suffix)))
    assert.ok(lines.some((line) => line.includes('./docs/screenshots/main-beta-home.jpg')))
    assert.ok(lines.some((line) => line.endsWith(' | ' + locale.newHeader + ' |')))
    assert.ok(!updated.includes(locale.oldHeader))
    for (const [target, downloadSize] of [
      [targets.mainBeta, '40.3 MiB'], [targets.leagueBeta, '100.3 MiB'], [targets.mainStable, '137.5 MiB'],
    ]) {
      const row = lines.find((line) => line.includes(target.installer.browser_download_url + ')'))
      assert.ok(row)
      assert.ok(row.includes(' | ' + target.version + ' | [' + locale.label + ']('))
      assert.ok(row.endsWith(' | ' + downloadSize + ' |'))
      assert.equal(row.includes('[SHA-256]'), !!target.checksum)
      if (target.checksum) assert.ok(row.includes(target.checksum.browser_download_url + ')'))
    }
    assert.equal(lines.filter((line) => locale.policy.test(line)).length, 1)
    assert.match(updated, locale.historical)
    assert.ok(updated.includes('[' + locale.mainNotes + '](' + releases + '/tag/' + targets.mainBeta.tag + ')'))
    assert.ok(updated.includes('[' + locale.leagueNotes + '](' + releases + '/tag/' + targets.leagueBeta.tag + ')'))
    assert.equal(rewriteReleaseReadme(updated, targets), updated)
    const nextBeta = release('4.0.0-beta.1', { size: 4 * mib })
    const future = rewriteReleaseReadme(updated, { ...targets, mainBeta: nextBeta })
    const historical = lines.find((line) => locale.historical.test(line))
    assert.ok(future.includes(historical))
    const changedLines = future.split(/\r?\n/).filter((line, index) => line !== lines[index])
    assert.equal(changedLines.length, 2)
    assert.ok(changedLines.every((line) => line.includes(nextBeta.tag)))
  })
}

test('historical versions and measured ranges come from the source README', () => {
  const mainBeta = release('3.2.0-beta.2', { size: 4 * mib })
  const leagueBeta = release('1.0.0-beta.2', { league: true, size: 3 * mib })
  const { mainStable } = targets
  const row = (label, { version, installer }, size) =>
    '| **' + label + '** | ' + version + ' | [Download EXE](' + installer.browser_download_url + ') | ' + size + ' |'
  const old = [
    '## Downloads', '',
    '| App | Version | Installer | Download / program size |',
    '| --- | --- | --- | --- |',
    row('MaxGameStudio', mainBeta, '38.6 / 107–108 MiB'),
    row('MaxGameStudio League', leagueBeta, '102.1 / 335 MiB'),
    row('MaxGameStudio stable', mainStable, '—'),
    '',
    '[Main app release notes](' + releases + '/tag/' + mainBeta.tag + ') · ' +
      '[League release notes](' + releases + '/tag/' + leagueBeta.tag + ') · [All releases](' + releases + ')',
    '',
    '## Features',
  ].join('\n')
  const updated = rewriteReleaseReadme(old, targets)
  assert.match(updated, /Historical installed sizes.*3\.2\.0-beta\.2.*107–108 MiB.*1\.0\.0-beta\.2.*335 MiB/)
})

test('historical stable repair keeps its version qualifier and exact checksum link', () => {
  const pkg = release('3.1.4', { size: 135 * mib })
  const stable = {
    ...pkg,
    installer: {
      ...pkg.installer, name: 'MaxGameStudio_3.1.4_x64-repair-setup.exe',
      browser_download_url: pkg.installer.browser_download_url.replace('_x64-setup', '_x64-repair-setup'),
    },
    checksum: { browser_download_url: releases + '/download/v3.1.4/SHA256SUMS' },
  }
  const updated = rewriteReleaseReadme(read('README.md'), { ...targets, mainStable: stable })
  assert.match(updated, /^.*\| 3\.1\.4 repair \|.*\[SHA-256\]\(.*\/SHA256SUMS\) \| 135\.0 MiB \|$/m)
})

test('throws on unrecognized README structure or unsafe target URLs', () => {
  const original = read('README.md')
  const wrongHeader = original.replace(/Download (?:\/ program )?size/, 'Size')
  const wrongSize = original.replace(/^(\| \*\*MaxGameStudio League\*\*[^\r\n]* \| )[^|\r\n]+( \|)$/m, '$1unknown$2')
  assert.throws(() => rewriteReleaseReadme(wrongHeader, targets), /README/)
  assert.throws(() => rewriteReleaseReadme(wrongSize, targets), /README/)
  assert.throws(() => rewriteReleaseReadme(original.replace('Main app release notes', 'Preview notes'), targets), /README/)
  assert.throws(() => rewriteReleaseReadme(original, {
    ...targets,
    mainBeta: { ...targets.mainBeta, installer: { ...targets.mainBeta.installer, browser_download_url: 'https://example.net/setup.exe' } },
  }), /installer/)
  assert.throws(() => rewriteReleaseReadme(original, {
    ...targets,
    mainBeta: { ...targets.mainBeta, checksum: { browser_download_url: releases + '/download/' + targets.mainBeta.tag + '/..' } },
  }), /checksum/)
})
