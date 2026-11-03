import { ArrowUpRight, BarChart3, BookOpen, Clapperboard, Download, FolderOpen, Gamepad2, Scissors, Settings2, Video } from 'lucide-react'
import { Link } from 'react-router-dom'
import { useT } from '../i18n/useT.js'
import '../styles/csWorkspace.css'

const sections = [
  {
    id: 'analysis',
    icon: BarChart3,
    links: [
      ['library', '/cs2/library', FolderOpen],
      ['officialDemos', '/cs2/match-history', Download],
      ['preview', '/cs2/analysis', BarChart3],
    ],
  },
  {
    id: 'production',
    icon: Clapperboard,
    links: [
      ['queue', '/cs2/queue', Video, true],
      ['assets', '/cs2/recorded-videos', FolderOpen, true],
      ['montage', '/cs2/montage', Clapperboard, true],
      ['liteCut', '/cs2/lite-cut', Scissors, true],
    ],
  },
  {
    id: 'tools',
    icon: Settings2,
    links: [
      ['recordingParams', '/cs2/params', Settings2, true],
      ['playerConfig', '/cs2/player-game-config', Gamepad2, true],
      ['guide', '/cs2/guide', BookOpen],
    ],
  },
]

const alwaysAvailable = new Set(['/cs2/library', '/cs2/match-history', '/cs2/analysis', '/cs2/guide'])

function WorkspaceLink({ section, id, to, Icon, gated, disabled, queueLength, t }) {
  const titleKey = `csWorkspace.${section}.${id}.title`
  const descriptionKey = `csWorkspace.${section}.${id}.description`
  const unavailable = disabled && gated && !alwaysAvailable.has(to)
  const accessibleName = id === 'queue' ? t('csWorkspace.queueCount', { count: queueLength }) : t(titleKey)
  const rowClass = `cs-workspace-link${unavailable ? ' is-disabled' : ''}`
  const content = (
    <>
      <span className="cs-workspace-link-icon" aria-hidden="true"><Icon size={18} strokeWidth={1.8} /></span>
      <span className="cs-workspace-link-copy">
        <span className="cs-workspace-link-title">{t(titleKey)}</span>
        <span className="cs-workspace-link-description">{t(descriptionKey)}</span>
      </span>
      {id === 'queue' && <span className="cs-workspace-queue-count" aria-label={t('csWorkspace.queueCount', { count: queueLength })}>{queueLength}</span>}
      {unavailable
        ? <span className="cs-workspace-link-state">{t('csWorkspace.disabled')}</span>
        : <ArrowUpRight className="cs-workspace-link-arrow" size={17} aria-hidden="true" />}
    </>
  )

  return unavailable
    ? <div className={rowClass} aria-label={accessibleName} aria-disabled="true">{content}</div>
    : <Link className={rowClass} aria-label={accessibleName} to={to}>{content}</Link>
}

export default function CsWorkspacePage({ queueLength = 0, disabled = false }) {
  const t = useT()

  return (
    <section className="cs-workspace-home" aria-labelledby="cs-workspace-page-heading">
      <div className="cs-workspace-inner">
        <header className="cs-workspace-hero">
          <div className="cs-workspace-hero-copy">
            <p className="cs-workspace-eyebrow">{t('csWorkspace.eyebrow')}</p>
            <h1 id="cs-workspace-page-heading">{t('csWorkspace.title')}</h1>
            <p className="cs-workspace-intro">{t('csWorkspace.description')}</p>
          </div>
          <Link className="cs-workspace-primary" to="/cs2/library">
            <span>{t('csWorkspace.libraryCta')}</span>
            <ArrowUpRight size={18} aria-hidden="true" />
          </Link>
        </header>

        <div className="cs-workspace-grid">
          {sections.map(({ id, icon: SectionIcon, links }) => (
            <section className={`cs-workspace-section cs-workspace-${id}`} key={id} aria-labelledby={`cs-workspace-${id}-heading`}>
              <div className="cs-workspace-section-heading">
                <span className="cs-workspace-section-icon" aria-hidden="true"><SectionIcon size={18} strokeWidth={1.8} /></span>
                <div>
                  <h2 id={`cs-workspace-${id}-heading`}>{t(`csWorkspace.${id}.title`)}</h2>
                  <p>{t(`csWorkspace.${id}.description`)}</p>
                </div>
              </div>
              <nav className="cs-workspace-nav" aria-label={t(`csWorkspace.${id}.navLabel`)}>
                {links.map(([linkId, to, Icon, gated]) => (
                  <WorkspaceLink key={linkId} section={id} id={linkId} to={to} Icon={Icon} gated={gated} disabled={disabled} queueLength={queueLength} t={t} />
                ))}
              </nav>
            </section>
          ))}
        </div>
        <p className="cs-workspace-footer-note">{t('csWorkspace.footerNote')}</p>
      </div>
    </section>
  )
}
