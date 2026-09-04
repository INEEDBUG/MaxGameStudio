import { Bug, CheckCircle2, CircleDot, ExternalLink, Github, Lightbulb, Sparkles } from "lucide-react";
import { useLocaleStore } from "../i18n/localeStore.js";
import { useT } from "../i18n/useT.js";
import { desktopBridge } from "../desktop/desktopBridge.js";
import {
  HOME_RELEASE_NOTES,
  HOME_RELEASE_VERSION,
  SUPERSEDED_LOCAL_CANDIDATE_VERSION,
} from "../data/homeReleaseNotes.js";

const REPO = "https://github.com/INEEDBUG/MaxGameStudio";
const ISSUE_URLS = {
  zh: {
    bug: `${REPO}/issues/new?template=bug_report.yml`,
    feature: `${REPO}/issues/new?template=feature_request.yml`,
  },
  en: {
    bug: `${REPO}/issues/new?template=bug_report_en.yml`,
    feature: `${REPO}/issues/new?template=feature_request_en.yml`,
  },
};

function openExternal(url) {
  if (desktopBridge?.openExternal) return void desktopBridge.openExternal(url);
  window.open(url, "_blank", "noopener,noreferrer");
}

function FeedbackCard({ icon: Icon, title, description, label, tone, onClick }) {
  return (
    <button type="button" onClick={onClick} className={`group flex min-h-[132px] cursor-pointer flex-col rounded-2xl border p-4 text-left transition-[border-color,background-color,transform] duration-200 hover:-translate-y-0.5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cs2-accent motion-reduce:transform-none motion-reduce:transition-none ${tone}`}>
      <span className="mb-3 flex items-center gap-2">
        <span className="grid h-9 w-9 place-items-center rounded-xl bg-cs2-bg-input"><Icon className="h-4 w-4" aria-hidden="true" /></span>
        <span className="text-sm font-semibold text-cs2-text-primary">{title}</span>
      </span>
      <span className="flex-1 text-xs leading-5 text-cs2-text-secondary">{description}</span>
      <span className="mt-3 inline-flex items-center gap-1 text-xs font-semibold text-cs2-accent">{label}<ExternalLink className="h-3.5 w-3.5" aria-hidden="true" /></span>
    </button>
  );
}

export default function HomePage() {
  const t = useT();
  const locale = useLocaleStore((state) => state.effectiveLocale);
  const language = locale === "zh-HK" || locale === "zh-TW" || locale === "zh" ? "zh" : "en";
  const issueUrls = ISSUE_URLS[language];
  const releaseNotes = HOME_RELEASE_NOTES[language];
  const sections = [
    ["fixed", CheckCircle2, "home.releaseFixed"],
    ["added", Sparkles, "home.releaseAdded"],
    ["optimized", CircleDot, "home.releaseOptimized"],
  ];

  return (
    <main className="mx-auto w-full max-w-6xl px-5 py-7 sm:px-8" aria-labelledby="home-title">
      <section className="relative overflow-hidden rounded-3xl border border-cs2-border/70 bg-cs2-bg-card p-6 shadow-[var(--cs2-shadow-md)] sm:p-8">
        <div className="pointer-events-none absolute -right-20 -top-24 h-64 w-64 rounded-full bg-cs2-accent/10 blur-3xl" />
        <div className="relative flex flex-wrap items-start justify-between gap-5">
          <div>
            <p className="mb-2 text-[11px] font-bold uppercase tracking-[0.18em] text-cs2-accent">MaxGameStudio</p>
            <h1 id="home-title" className="text-2xl font-bold tracking-tight text-cs2-text-primary sm:text-3xl">{t("home.title")}</h1>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-cs2-text-secondary">{t("home.subtitle")}</p>
          </div>
          <div className="rounded-2xl border border-cs2-border-subtle bg-cs2-bg-input/60 px-4 py-3 text-right">
            <div className="text-[10px] uppercase tracking-[0.16em] text-cs2-text-muted">{t("home.currentVersion")}</div>
            <div className="mt-1 font-mono text-lg font-semibold text-cs2-accent">v{HOME_RELEASE_VERSION}</div>
          </div>
        </div>
      </section>

      <section className="mt-5 rounded-2xl border border-cs2-border/70 bg-cs2-bg-card p-5 sm:p-6" aria-labelledby="home-release-title">
        <div className="flex items-center gap-2"><Github className="h-4 w-4 text-cs2-accent" aria-hidden="true" /><h2 id="home-release-title" className="text-base font-bold text-cs2-text-primary">{t("home.releaseTitle", { version: HOME_RELEASE_VERSION })}</h2></div>
        <div role="note" className="mt-4 rounded-xl border border-cs2-accent/35 bg-cs2-accent/10 px-4 py-3 text-xs leading-5 text-cs2-text-primary">{t("home.stableReleaseNotice", { stableVersion: HOME_RELEASE_VERSION, supersededVersion: SUPERSEDED_LOCAL_CANDIDATE_VERSION })}</div>
        <div className="mt-5 grid gap-5 md:grid-cols-3">
          {sections.map(([key, Icon, titleKey]) => <div key={key}><div className="flex items-center gap-2 text-xs font-semibold text-cs2-text-primary"><Icon className="h-4 w-4 text-cs2-accent" aria-hidden="true" />{t(titleKey)}</div><ul className="mt-3 space-y-2">{releaseNotes[key].map((note) => <li key={note} className="text-xs leading-5 text-cs2-text-secondary">{note}</li>)}</ul></div>)}
        </div>
      </section>

      <section className="mt-5" aria-labelledby="home-feedback-title">
        <div className="mb-3"><h2 id="home-feedback-title" className="text-base font-bold text-cs2-text-primary">{t("home.feedbackTitle")}</h2><p className="mt-1 text-xs text-cs2-text-muted">{t("home.feedbackSubtitle")}</p></div>
        <div className="grid gap-3 md:grid-cols-2">
          <FeedbackCard icon={Bug} title={t("home.reportBug")} description={t("home.reportBugDesc")} label={t("home.openIssue")} tone="border-red-500/30 bg-red-500/[0.06] hover:border-red-400/60" onClick={() => openExternal(issueUrls.bug)} />
          <FeedbackCard icon={Lightbulb} title={t("home.requestFeature")} description={t("home.requestFeatureDesc")} label={t("home.openIssue")} tone="border-cs2-accent/30 bg-cs2-accent/[0.06] hover:border-cs2-accent/60" onClick={() => openExternal(issueUrls.feature)} />
        </div>
        <div className="mt-3 flex flex-wrap gap-3">
          <button type="button" onClick={() => openExternal(`${REPO}/pulls`)} className="inline-flex min-h-10 cursor-pointer items-center gap-2 rounded-xl border border-cs2-border px-3.5 text-xs font-semibold text-cs2-text-secondary transition-colors hover:border-cs2-accent/50 hover:text-cs2-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cs2-accent"><Github className="h-4 w-4" aria-hidden="true" />{t("home.viewPr")}</button>
          <button type="button" onClick={() => openExternal(REPO)} className="inline-flex min-h-10 cursor-pointer items-center gap-2 rounded-xl border border-cs2-border px-3.5 text-xs font-semibold text-cs2-text-secondary transition-colors hover:border-cs2-accent/50 hover:text-cs2-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cs2-accent"><Sparkles className="h-4 w-4" aria-hidden="true" />{t("home.viewRepository")}</button>
        </div>
      </section>
    </main>
  );
}
