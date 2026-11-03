// Isolated layout regression: real routed page, real styles and bounded shell.
// External API is fulfilled by the browser test, never a production backend.
import React from 'react';
import {createRoot} from 'react-dom/client';
import {MemoryRouter} from 'react-router-dom';
import {AppShellProvider} from '../../src/context/AppShellContext.jsx';
import CosmeticWorkspacePage from '../../src/pages/CosmeticWorkspacePage.jsx';
import '../../src/index.css';
import '../../src/styles/liquidGlass.css';

const shell={uploadedDemos:[{path:'D:/fixture.dem',filename:'Scroll regression Demo'}],
    currentMatchIndex:0,players:[{name:'Alice',steam_id64:'76561199369875913'}],currentActivePlayer:'Alice'};
createRoot(document.getElementById('root')).render(<MemoryRouter><AppShellProvider value={shell}>
  <div className="app-shell relative flex h-screen flex-col overflow-hidden bg-cs2-bg-page text-cs2-text-primary">
    <header className="h-11 shrink-0 border-b border-cs2-border px-5 py-2">MaxGameStudio · local scroll regression</header>
    <div className="relative flex flex-1 overflow-hidden">
      <aside className="w-56 shrink-0 border-r border-cs2-border p-5">CS2 · Demo 饰品</aside>
      <main className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden"><CosmeticWorkspacePage/></div>
      </main>
    </div>
  </div>
</AppShellProvider></MemoryRouter>);
