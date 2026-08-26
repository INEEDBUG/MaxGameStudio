import { useEffect, useRef, useState } from "react";
import { NavLink } from "react-router-dom";
import {
  BarChart3,
  BookOpen,
  Check,
  ChevronUp,
  Clapperboard,
  Clock3,
  Crosshair,
  Download,
  Gamepad2,
  Keyboard,
  Laptop,
  Library,
  Moon,
  Package,
  Settings,
  Sun,
  Video,
} from "lucide-react";
import { useThemeStore } from "../stores/themeStore";
import { useReplayStore } from "../stores/replayStore";
import { useT } from "../i18n/useT.js";

const linkBase =
  "group flex items-center gap-2.5 rounded-[10px] px-2.5 py-2 text-[12px] font-medium transition-[background-color,color,transform] duration-150 active:scale-[0.98]";
const linkIdle = "text-cs2-text-secondary hover:bg-cs2-bg-hover hover:text-cs2-text-primary";
const linkActive = "bg-cs2-accent-soft text-cs2-accent shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--cs2-accent)_14%,transparent)]";

const themeOptions = [
  { mode: "system", icon: Laptop, labelKey: "nav.themeSystem", hintKey: "nav.themeSystemHint" },
  { mode: "time", icon: Clock3, labelKey: "nav.themeTime", hintKey: "nav.themeTimeHint" },
  { mode: "light", icon: Sun, labelKey: "nav.themeLight", hintKey: "nav.themeLightHint" },
  { mode: "dark", icon: Moon, labelKey: "nav.themeDark", hintKey: "nav.themeDarkHint" },
];

function suspendReplayPlayback() {
  useReplayStore.getState().requestSuspendPlayback();
}

function AppearanceMenu() {
  const mode = useThemeStore((state) => state.mode);
  const resolvedTheme = useThemeStore((state) => state.resolvedTheme);
  const setMode = useThemeStore((state) => state.setMode);
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(() => new Date());
  const rootRef = useRef(null);
  const t = useT();
  const current = themeOptions.find((option) => option.mode === mode) ?? themeOptions[0];
  const CurrentIcon = current.icon;

  useEffect(() => {
    if (!open) return undefined;
    const updateClock = window.setInterval(() => setNow(new Date()), 30_000);
    const closeOnOutside = (event) => {
      if (!rootRef.current?.contains(event.target)) setOpen(false);
    };
    const closeOnEscape = (event) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      window.clearInterval(updateClock);
      document.removeEventListener("pointerdown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      {open && (
        <div
          role="menu"
          aria-label={t("nav.appearance")}
          className="absolute bottom-[calc(100%+8px)] left-0 z-[120] w-[248px] origin-bottom-left rounded-2xl border border-cs2-border-subtle bg-cs2-bg-elevated p-1.5 shadow-[var(--cs2-shadow-lg)] transition-[opacity,transform] duration-150"
        >
          <div className="px-2.5 pb-1.5 pt-1 text-[10px] font-semibold uppercase tracking-[0.14em] text-cs2-text-muted">
            {t("nav.appearance")}
          </div>
          {themeOptions.map(({ mode: optionMode, icon: Icon, labelKey, hintKey }) => {
            const selected = optionMode === mode;
            return (
              <button
                key={optionMode}
                type="button"
                role="menuitemradio"
                aria-checked={selected}
                onClick={() => {
                  setMode(optionMode);
                  setOpen(false);
                }}
                className={`flex w-full items-center gap-2.5 rounded-xl px-2.5 py-2 text-left transition-[background-color,transform] duration-150 active:scale-[0.98] ${selected ? "bg-cs2-accent-soft" : "hover:bg-cs2-bg-hover"}`}
              >
                <span className={`grid h-7 w-7 shrink-0 place-items-center rounded-lg ${selected ? "bg-cs2-accent text-white" : "bg-cs2-bg-input text-cs2-text-secondary"}`}>
                  <Icon className="h-3.5 w-3.5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-[12px] font-semibold text-cs2-text-primary">{t(labelKey)}</span>
                  <span className="block truncate text-[10px] text-cs2-text-muted">{t(hintKey)}</span>
                </span>
                {selected && <Check className="h-3.5 w-3.5 shrink-0 text-cs2-accent" />}
              </button>
            );
          })}
          <div className="mx-2 mt-1 flex items-center justify-between border-t border-cs2-border-subtle px-0.5 pt-2 text-[10px] text-cs2-text-muted">
            <span>{t("nav.localTime")}</span>
            <span className="font-mono tabular-nums text-cs2-text-secondary">
              {now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} · {resolvedTheme === "dark" ? t("nav.darkActive") : t("nav.lightActive")}
            </span>
          </div>
        </div>
      )}

      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2.5 rounded-[10px] px-2.5 py-2 text-[12px] font-medium text-cs2-text-secondary transition-[background-color,color,transform] duration-150 hover:bg-cs2-bg-hover hover:text-cs2-text-primary active:scale-[0.98]"
      >
        <CurrentIcon className="h-4 w-4 shrink-0" />
        <span className="min-w-0 flex-1 truncate text-left">{t(current.labelKey)}</span>
        <ChevronUp className={`h-3.5 w-3.5 transition-transform duration-150 ${open ? "rotate-180" : ""}`} />
      </button>
    </div>
  );
}

function NavItem({ to, icon: Icon, children, end = false, disabled = false, badge = null, iconClass = "" }) {
  return (
    <NavLink
      to={to}
      end={end}
      className={({ isActive }) => `${linkBase} ${isActive ? linkActive : linkIdle} ${disabled ? "pointer-events-none opacity-40" : ""}`}
    >
      <Icon className={`h-4 w-4 shrink-0 ${iconClass}`} />
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {badge !== null && (
        <span className="min-w-5 rounded-full bg-cs2-bg-input px-1.5 text-center font-mono text-[10px] tabular-nums text-cs2-text-secondary">
          {badge}
        </span>
      )}
    </NavLink>
  );
}

export default function SidebarNav({ queueLength = 0, disabled = false }) {
  const t = useT();

  return (
    <aside className="relative z-[60] flex w-56 shrink-0 flex-col border-r border-cs2-border-subtle bg-cs2-bg-sidebar/88 px-2.5 pb-2.5 backdrop-blur-2xl">
      <div className="flex items-center gap-2.5 px-1.5 py-4">
        <img src="/cs2-ultimate-insight-logo.png" alt={t("nav.brand")} width={42} height={42} decoding="async" className="h-10 w-10 shrink-0 object-contain drop-shadow-sm" />
        <div className="min-w-0">
          <div className="truncate text-[13px] font-semibold tracking-[-0.01em] text-cs2-text-primary">{t("nav.brand")}</div>
          <div className="mt-0.5 text-[9px] font-medium tracking-[0.08em] text-cs2-text-muted">STUDIO · {__APP_VERSION__}</div>
        </div>
      </div>

      <nav className="scrollbar-hover flex flex-1 flex-col gap-0.5 overflow-y-auto" aria-label={t("nav.mainNav")} onPointerDownCapture={suspendReplayPlayback}>
        <div className="px-2.5 pb-1 pt-1 text-[9px] font-semibold uppercase tracking-[0.14em] text-cs2-text-muted">{t("nav.sectionWorkflow")}</div>
        <NavItem to="/" end icon={BookOpen}>{t("nav.guide")}</NavItem>
        <NavItem to="/library" icon={Library}>{t("nav.demoLibrary")}</NavItem>
        <NavItem to="/match-history" icon={Download}>{t("nav.officialDemos")}</NavItem>
        <NavItem to="/analysis" icon={BarChart3}>{t("nav.analysis")}</NavItem>
        <NavItem to="/queue" icon={Package} disabled={disabled} badge={queueLength}>{t("nav.recordQueue")}</NavItem>
        <NavItem to="/recorded-videos" icon={Video} disabled={disabled}>{t("nav.recordedVideos")}</NavItem>
        <NavItem to="/montage" icon={Clapperboard} disabled={disabled}>{t("nav.montage")}</NavItem>
        <NavItem to="/lite-cut" icon={Clapperboard} disabled={disabled}>LiteCut</NavItem>

        <div className="mt-3 px-2.5 pb-1 text-[9px] font-semibold uppercase tracking-[0.14em] text-cs2-text-muted">{t("nav.sectionTools")}</div>
        <NavItem to="/sensitivity-lab" icon={Crosshair}>{t("nav.sensitivityLab")}</NavItem>
        <NavItem to="/input-lab" icon={Keyboard}>{t("nav.inputLab")}</NavItem>
        <NavItem to="/valorant-lab" icon={Gamepad2}>{t("nav.valorantLab")}</NavItem>
        <NavItem to="/league-lab" icon={Gamepad2}>{t("nav.leagueLab")}</NavItem>
      </nav>

      <div className="mt-2 space-y-0.5 border-t border-cs2-border-subtle pt-2">
        <NavItem to="/settings" icon={Settings}>{t("nav.settings")}</NavItem>
        <AppearanceMenu />
      </div>
    </aside>
  );
}
