const releases = 'https://github.com/INEEDBUG/MaxGameStudio/releases'
const mib = 1024 * 1024
const beta = /\d+\.\d+\.\d+-beta\.\d+/
const stable = /\d+\.\d+\.\d+/

const languages = [
  {
    heading: '## Downloads',
    columns: '| App | Version | Installer | ',
    sizeHeading: 'Download size',
    oldSizeHeading: 'Download / program size',
    appNames: ['MaxGameStudio', 'MaxGameStudio League', 'MaxGameStudio stable'],
    download: 'Download EXE',
    repair: ' repair',
    notes: ['Main app release notes', 'League release notes', 'All releases'],
    historicalPrefix: 'Historical installed sizes were ',
    historical: (measurements) => 'Historical installed sizes were ' + measurements.join(' and ') + '.',
    policy: 'GitHub keeps one beta and up to three stable releases for each app, while Git tags and source history remain available.',
  },
  {
    heading: '## 下载',
    columns: '| 软件 | 版本 | 安装包 | ',
    sizeHeading: '下载体积',
    oldSizeHeading: '下载体积 / 程序占用',
    appNames: ['MaxGameStudio 主程序', 'MaxGameStudio League 工具箱', 'MaxGameStudio 正式版'],
    download: '下载 EXE',
    repair: ' 修复版',
    notes: ['主程序发布说明', '工具箱发布说明', '全部版本'],
    historicalPrefix: '历史实测程序占用为 ',
    historical: (measurements) => '历史实测程序占用为 ' + measurements.join('，') + '。',
    policy: 'GitHub 上每款软件保留 1 个测试版和最多 3 个正式版，同时保留 Git 标签和源代码历史。',
  },
]

const unsupported = () => new Error('Unrecognized README downloads section.')
const link = (label, url) => '[' + label + '](' + url + ')'
const downloadPrefix = (tag) => releases + '/download/' + tag + '/'
const releaseTag = (index, version) => (index === 1 ? 'league-v' : 'v') + version
const versionPattern = (index) => new RegExp('^(?:' + (index === 2 ? stable.source : beta.source) + ')$')

function isAssetLink(url, tag) {
  const prefix = downloadPrefix(tag)
  return typeof url === 'string' && url.startsWith(prefix) && /^[A-Za-z0-9._-]+$/.test(url.slice(prefix.length))
}

function notesLine(language, tags) {
  return language.notes.map((label, index) => link(label, index < 2 ? releases + '/tag/' + tags[index] : releases))
    .join(' · ')
}

function readOldRow(line, index, language, legacy) {
  const [, name, displayVersion, installer, size] = /^\| (.+?) \| (.+?) \| (.+?) \| (.+?) \|$/.exec(line ?? '') ?? []
  if (name !== '**' + language.appNames[index] + '**' || !displayVersion) throw unsupported()
  const version = index === 2 && displayVersion.endsWith(language.repair)
    ? displayVersion.slice(0, -language.repair.length)
    : displayVersion
  if (!versionPattern(index).test(version)) throw unsupported()
  const tag = releaseTag(index, version)
  const downloadLabel = '[' + language.download + ']('
  const [exe, checksum, ...unexpected] = installer.split(' · ')
  if (unexpected.length || !exe.startsWith(downloadLabel) || !exe.endsWith(')') ||
      !isAssetLink(exe.slice(downloadLabel.length, -1), tag) ||
      (checksum !== undefined && (!checksum.startsWith('[SHA-256](') || !checksum.endsWith(')') ||
        !isAssetLink(checksum.slice('[SHA-256]('.length, -1), tag)))) {
    throw unsupported()
  }

  if (!legacy) {
    if (!/^\d+(?:\.\d+)? MiB$/.test(size)) throw unsupported()
    return { tag }
  }
  const measurement = /^\d+(?:\.\d+)? \/ (\d+(?:\.\d+)?(?:[–-]\d+(?:\.\d+)?)? MiB)$/.exec(size)?.[1]
  if (!measurement && !(index === 2 && size === '—')) throw unsupported()
  return { tag, measurement: measurement && language.appNames[index] + ' ' + displayVersion + ' ' + measurement }
}

function renderRow(target, index, language) {
  const { version, tag, installer, checksum } = target ?? {}
  if (typeof version !== 'string' || !versionPattern(index).test(version) ||
      tag !== releaseTag(index, version) || !installer || !Number.isSafeInteger(installer.size) || installer.size <= 0 ||
      typeof installer.name !== 'string' || !installer.name.endsWith('.exe') ||
      !isAssetLink(installer.browser_download_url, tag) ||
      installer.browser_download_url !== downloadPrefix(tag) + installer.name) {
    throw new Error('Invalid published README installer for ' + language.appNames[index] + '.')
  }
  if (checksum && ![installer.browser_download_url + '.sha256', downloadPrefix(tag) + 'SHA256SUMS']
    .includes(checksum.browser_download_url)) {
    throw new Error('Invalid published README checksum for ' + language.appNames[index] + '.')
  }

  const label = '**' + language.appNames[index] + '**'
  const displayVersion = index === 2 && installer.name.endsWith('_x64-repair-setup.exe')
    ? version + language.repair
    : version
  const links = link(language.download, installer.browser_download_url) +
    (checksum ? ' · ' + link('SHA-256', checksum.browser_download_url) : '')
  return '| ' + [label, displayVersion, links, (installer.size / mib).toFixed(1) + ' MiB'].join(' | ') + ' |'
}

export function rewriteReleaseReadme(content, targets) {
  const newline = content.includes('\r\n') ? '\r\n' : '\n'
  const lines = content.split(newline)
  if (lines.some((line) => /[\r\n]/.test(line))) throw unsupported()
  const headings = lines.filter((line) => languages.some((language) => language.heading === line))
  if (headings.length !== 1) throw unsupported()
  const language = languages.find((candidate) => candidate.heading === headings[0])
  const section = lines.indexOf(language.heading)
  const end = lines.findIndex((line, index) => index > section && line.startsWith('## '))
  if (end < 0) throw unsupported()

  const currentHeader = language.columns + language.sizeHeading + ' |'
  const oldHeader = language.columns + language.oldSizeHeading + ' |'
  const table = lines.slice(section, end)
  const headers = table.filter((line) => line === oldHeader || line === currentHeader)
  if (headers.length !== 1) throw unsupported()
  const header = section + table.indexOf(headers[0])
  const legacy = headers[0] === oldHeader
  if (lines[header + 1] !== '| --- | --- | --- | --- |' || header + 5 >= end ||
      lines[header + 5].startsWith('|')) {
    throw unsupported()
  }

  const current = language.appNames.map((_, index) => readOldRow(lines[header + 2 + index], index, language, legacy))
  const currentNotes = notesLine(language, current.map(({ tag }) => tag))
  if (table.filter((line) => line === currentNotes).length !== 1 ||
      table.filter((line) => line === language.policy).length !== (legacy ? 0 : 1) ||
      table.filter((line) => line.startsWith(language.historicalPrefix)).length !== (legacy ? 0 : 1)) {
    throw unsupported()
  }

  const next = [targets.mainBeta, targets.leagueBeta, targets.mainStable]
  const replacement = next.map((target, index) => renderRow(target, index, language))
  lines[section + table.indexOf(currentNotes)] = notesLine(language, next.map(({ tag }) => tag))
  lines[header] = currentHeader
  lines.splice(header + 2, replacement.length, ...replacement)
  if (legacy) {
    const measurements = current.map(({ measurement }) => measurement).filter(Boolean)
    lines.splice(header + 5, 0, '', language.historical(measurements), '', language.policy)
  }
  return lines.join(newline)
}
