// The symbols below come from Lucide 0.460.0. See artwork/LUCIDE-LICENSE.txt.
const swords = `
  <polyline points="14.5 17.5 3 6 3 3 6 3 17.5 14.5" />
  <line x1="13" x2="19" y1="19" y2="13" />
  <line x1="16" x2="20" y1="16" y2="20" />
  <line x1="19" x2="21" y1="21" y2="19" />
  <polyline points="14.5 6.5 18 3 21 3 21 6 17.5 9.5" />
  <line x1="5" x2="9" y1="14" y2="18" />
  <line x1="7" x2="4" y1="17" y2="20" />
  <line x1="3" x2="5" y1="19" y2="21" />
`

const gamepad = `
  <line x1="6" x2="10" y1="11" y2="11" />
  <line x1="8" x2="8" y1="9" y2="13" />
  <line x1="15" x2="15.01" y1="12" y2="12" />
  <line x1="18" x2="18.01" y1="10" y2="10" />
  <path d="M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z" />
`

const monitor = `
  <path d="M10 7.75a.75.75 0 0 1 1.142-.638l3.664 2.249a.75.75 0 0 1 0 1.278l-3.664 2.25a.75.75 0 0 1-1.142-.64z" />
  <path d="M12 17v4" />
  <path d="M8 21h8" />
  <rect x="2" y="3" width="20" height="14" rx="2" />
`

const medal = `
  <path d="M7.21 15 2.66 7.14a2 2 0 0 1 .13-2.2L4.4 2.8A2 2 0 0 1 6 2h12a2 2 0 0 1 1.6.8l1.6 2.14a2 2 0 0 1 .14 2.2L16.79 15" />
  <path d="M11 12 5.12 2.2" />
  <path d="m13 12 5.88-9.8" />
  <path d="M8 7h8" />
  <circle cx="12" cy="17" r="5" />
  <path d="M12 18v-2h-.5" />
`

const dice = `
  <rect width="18" height="18" x="3" y="3" rx="2" ry="2" />
  <path d="M16 8h.01" />
  <path d="M8 8h.01" />
  <path d="M8 16h.01" />
  <path d="M16 16h.01" />
  <path d="M12 12h.01" />
`

const chart = `
  <path d="M12 16v5" />
  <path d="M16 14v7" />
  <path d="M20 10v11" />
  <path d="m22 3-8.646 8.646a.5.5 0 0 1-.708 0L9.354 8.354a.5.5 0 0 0-.707 0L2 15" />
  <path d="M4 18v3" />
  <path d="M8 14v7" />
`

function icon(paths, color) {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none"
    stroke="${color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    ${paths}
  </svg>`
}

const coordinateGrid = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <rect width="100" height="100" fill="#1b2632"/>
  <path d="M25 0v100M50 0v100M75 0v100M0 25h100M0 50h100M0 75h100"
    stroke="#425166" stroke-width=".65"/>
  <rect x=".5" y=".5" width="99" height="99" fill="none" stroke="#687687"/>
</svg>`

const rankColors = {
  unranked: '#8591a1', iron: '#a69b99', bronze: '#bb8463', silver: '#a7b9cc',
  gold: '#dfb253', platinum: '#6fc5b0', emerald: '#58b48b', diamond: '#88abec',
  master: '#bd92ec', grandmaster: '#e97578', challenger: '#f5cc89'
}

const launcherArtwork = {
  league: icon(swords, '#d89230'),
  wegame: icon(gamepad, '#62bca0'),
  riotclient: icon(monitor, '#76a9e4')
}

const shared = 'src/renderer-shared'
const assetReplacements = new Map([
  [`${shared}/assets/logo/maxgamestudio-logo.png`, launcherArtwork.league],
  [`${shared}/assets/champions/bravery-circle.png`, icon(dice, '#d89230')],
  [`${shared}/assets/champions/bravery-square.png`, icon(dice, '#d89230')]
])

for (const [name, svg] of Object.entries(launcherArtwork)) {
  assetReplacements.set(`${shared}/assets/ico/${name}.ico`, svg)
  assetReplacements.set(`src/renderer/src-main-window/assets/ico/${name}.ico`, svg)
}
for (const [name, color] of Object.entries(rankColors)) {
  const badge = icon(medal, color)
  assetReplacements.set(`${shared}/assets/ranked-icons/${name}.png`, badge)
  assetReplacements.set(`${shared}/assets/ranked-icons-large/${name}.png`, badge)
}
for (const id of [11, 12, 21]) {
  assetReplacements.set(`${shared}/components/match-card/map-images/${id}.png`, coordinateGrid)
}

export const logoComponents = new Map([
  [`${shared}/assets/icon/AkariLogo.vue`, icon(swords, 'currentColor')],
  [`${shared}/assets/icon/OpggIcon.vue`, icon(chart, 'currentColor')]
])

export function artworkModule(path) {
  const svg = assetReplacements.get(path)
  if (svg) return `export default ${JSON.stringify(`data:image/svg+xml,${encodeURIComponent(svg)}`)}`
}

const ioniconNames = new Map(Object.entries({
  Fire: 'FlameOutline', Robot: 'HardwareChipOutline', Crown: 'TrophyOutline',
  Github: 'CodeSlashOutline', Star: 'StarOutline', ExchangeAlt: 'SwapHorizontalOutline',
  Discord: 'ChatbubblesOutline', Qq: 'ChatbubbleEllipsesOutline', Telegram: 'PaperPlaneOutline',
  Envelope: 'MailOutline', Globe: 'GlobeOutline', Link: 'LinkOutline'
}))

export function useMitIconImports(source) {
  return source.replace(/^import \{([^}]+)\} from '@vicons\/fa'(?=\r?$)/gm, (_, imports) => {
    const bindings = imports.split(',').map((binding) => {
      const [name, localName = name] = binding.trim().split(/\s+as\s+/)
      const icon = ioniconNames.get(name)
      if (!icon) throw new Error(`Missing native replacement for Font Awesome icon ${name}`)
      return `${icon} as ${localName}`
    })
    return `import { ${bindings.join(', ')} } from '@vicons/ionicons5'`
  })
}
