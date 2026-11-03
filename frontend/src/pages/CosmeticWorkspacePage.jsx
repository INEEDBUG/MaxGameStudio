import { Link } from 'react-router-dom';
import { ArrowLeft, FolderOpen, Palette } from 'lucide-react';
import { useAppShell } from '../context/AppShellContext.jsx';
import CosmeticLoadoutView from '../components/analysis/cosmetics/CosmeticLoadoutView.jsx';

export default function CosmeticWorkspacePage() {
  const shell = useAppShell();
  const demo = shell.uploadedDemos?.[shell.currentMatchIndex];
  const player = shell.players?.find((entry) => (entry.name || entry.player_name) === shell.currentActivePlayer);
  const xuid = String(player?.steam_id64 || player?.steam_id || player?.steamid || '');
  return <main aria-label="Demo 饰品工作区" tabIndex={0} className="mx-auto min-h-0 w-full max-w-[1440px] flex-1 overflow-y-auto space-y-5 px-5 py-6 text-cs2-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-cs2-accent">
    <header className="flex flex-wrap items-center justify-between gap-3"><div><h1 className="flex items-center gap-2 text-xl font-bold"><Palette size={22} aria-hidden="true" />Demo 饰品</h1><p className="mt-1 text-sm text-cs2-text-secondary">{demo?.filename || '使用 Demo 库中已选择的回放'}</p></div><Link className="inline-flex items-center gap-2 rounded-lg border border-cs2-border px-3 py-2 text-sm focus-visible:ring-2 focus-visible:ring-cs2-accent" to="/cs2/analysis"><ArrowLeft size={16} aria-hidden="true" />返回 Demo 分析</Link></header>
    {demo?.path ? <CosmeticLoadoutView demoPath={demo.path} selectedXuid={xuid} /> : <section className="cosmetic-empty"><FolderOpen size={30} aria-hidden="true" /><h2>先选择一个 Demo</h2><p>直接使用当前 Demo 与玩家，无需重复上传或填写 XUID。</p><Link className="rounded-lg bg-cs2-accent px-4 py-2 font-semibold text-cs2-text-on-accent focus-visible:ring-2 focus-visible:ring-cs2-accent" to="/cs2/library">从 Demo 库选择</Link></section>}
  </main>;
}
