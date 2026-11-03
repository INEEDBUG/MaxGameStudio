import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, test, vi } from 'vitest';
import { AppShellProvider } from '../context/AppShellContext.jsx';
import CosmeticWorkspacePage from './CosmeticWorkspacePage.jsx';
vi.mock('../components/analysis/cosmetics/CosmeticLoadoutView.jsx', () => ({
  default: ({ demoPath, selectedXuid }) => <section aria-label="饰品上下文">{demoPath} · {selectedXuid}</section>,
}));
function page(value) { return render(<MemoryRouter><AppShellProvider value={value}><CosmeticWorkspacePage /></AppShellProvider></MemoryRouter>); }
describe('standalone cosmetic entry', () => {
  test('reuses the current Demo and exact roster ID without a wizard', async () => {
    page({ uploadedDemos: [{ path: 'D:/demo.dem', filename: 'Current Demo' }], currentMatchIndex: 0, players: [{ name: 'Alice', steam_id64: '76561199369875913' }], currentActivePlayer: 'Alice' });
    expect((await screen.findByRole('region', { name: '饰品上下文' })).textContent).toBe('D:/demo.dem · 76561199369875913');
    expect(screen.queryByLabelText('本地 Demo 路径')).toBeNull();
  });
  test('links existing Demo library when no Demo is selected instead of asking for XUID or a path', () => {
    page({ uploadedDemos: [], currentMatchIndex: 0, players: [] });
    expect(screen.getByRole('link', { name: '从 Demo 库选择' }).getAttribute('href')).toBe('/cs2/library');
    expect(screen.queryByRole('region', { name: '饰品上下文' })).toBeNull();
  });
  test('only exposes the contextual loadout workflow, not the old technical experiment form', () => {
    page({ uploadedDemos: [], currentMatchIndex: 0, players: [] });
    expect(screen.queryByText('独立制片实验工具')).toBeNull();
    expect(screen.queryByRole('link', { name: /打开离线预览/ })).toBeNull();
  });
});
