import { ArrowRight, Keyboard, Scissors } from "lucide-react";
import { Link } from "react-router-dom";
import { useT } from "../i18n/useT.js";
import { useLocaleStore } from "../i18n/localeStore.js";
import GameIcon from "../components/GameIcon.jsx";
import SteamAccountsPanel from "../components/SteamAccountsPanel.jsx";

const tools = [
  { to: "/cs2/library", game: "cs2", title: "nav.demoLibrary", zh: "导入、回看与分析比赛", en: "Import, review and analyze matches" },
  { to: "/cs2/match-history", game: "cs2", title: "nav.officialDemos", zh: "获取 Steam 官匹录像", en: "Get Steam matchmaking replays" },
  { to: "/cs2/lite-cut", icon: Scissors, title: "nav.liteCut", zh: "剪下想保留的片段", en: "Keep the moments you want" },
  { to: "/valorant/stretch", game: "valorant", title: "nav.sectionValorant", zh: "拉伸设置与准星工具", en: "Stretch settings and crosshairs" },
  { to: "/peripherals/sensitivity", icon: Keyboard, title: "nav.sectionPeripherals", zh: "换算灵敏度与检查输入", en: "Convert sensitivity and check inputs" },
];

export default function HomePage() {
  const t = useT();
  const locale = useLocaleStore((state) => state.effectiveLocale);
  const language = locale?.startsWith("zh") ? "zh" : "en";
  return (
    <div className="mx-auto w-full max-w-5xl px-6 py-7 lg:px-9" aria-labelledby="home-title">
      <header className="mb-7 flex items-baseline justify-between gap-4">
        <h1 id="home-title" className="text-xl font-semibold tracking-tight">{t("nav.home")}</h1>
        <Link to="/about" className="rounded text-xs text-cs2-text-muted hover:text-cs2-text-primary focus-visible:outline-2 focus-visible:outline-cs2-accent">{t("nav.about")}</Link>
      </header>
      <SteamAccountsPanel compact />
      <section className="mt-8" aria-labelledby="home-tools-title">
        <h2 id="home-tools-title" className="mb-3 text-sm font-semibold">{language === "zh" ? "常用工具" : "Tools"}</h2>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {tools.map(({ to, game, icon: Icon, title, ...copy }) => (
            <Link key={to} to={to} className="group flex min-h-24 items-center gap-3 rounded-lg border border-cs2-border bg-cs2-bg-card p-4 transition-colors hover:border-cs2-accent/50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cs2-accent">
              <span className="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-cs2-bg-input">{game ? <GameIcon game={game} /> : <Icon className="h-5 w-5 text-cs2-text-secondary" aria-hidden="true" />}</span>
              <span className="min-w-0 flex-1"><span className="block text-sm font-medium">{t(title)}</span><span className="mt-1 block text-xs leading-5 text-cs2-text-muted">{copy[language]}</span></span>
              <ArrowRight className="h-3.5 w-3.5 shrink-0 text-cs2-text-muted group-hover:text-cs2-accent" aria-hidden="true" />
            </Link>
          ))}
        </div>
      </section>
    </div>
  );
}
