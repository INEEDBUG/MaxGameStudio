export type ResgDisplayMode = 'all' | 'compact'

// Runs only inside the sandboxed public RESG frame: no application bridge, private
// Vue state, account data, extra requests, or polling. Unknown markup fails open
// to the original website rather than inventing recommendations.
export function resgDisplayScript(
  mode: ResgDisplayMode,
  championId: number,
  english: boolean,
  dark: boolean
) {
  return String.raw`(async () => {
    const mode = ${JSON.stringify(mode)};
    const championId = ${JSON.stringify(championId)};
    const english = ${english};
    const dark = ${dark};
    const rootId = 'mgs-resg-compact';
    const styleId = 'mgs-resg-compact-style';
    const reset = () => {
      document.getElementById(rootId)?.remove();
      document.getElementById(styleId)?.remove();
      document.body.removeAttribute('data-mgs-resg-compact');
    };
    reset();
    if (mode === 'all') return { kind: 'all' };
    const current = () => new RegExp('/champions/' + championId + '/?$').test(location.pathname);
    const text = (node) => (node?.textContent || '').trim();
    const count = (value) => {
      if (!/^(?:\d+|\d{1,3}(?:,\d{3})+)\s*(?:场)?$/.test(value)) throw Error('Unknown count');
      const result = Number(value.replace(/[,\s场]/g, ''));
      if (!Number.isSafeInteger(result) || result < 0) throw Error('Invalid count');
      return result;
    };
    const percent = (value) => {
      if (!/^\d+(?:\.\d+)?%$/.test(value)) throw Error('Unknown percentage');
      const result = Number(value.slice(0, -1));
      if (result < 0 || result > 100) throw Error('Invalid percentage');
      return result;
    };
    const icons = (row) => Array.from(row.querySelectorAll('img')).map(img => {
      const name = (img.getAttribute('alt') || '').trim();
      const src = new URL(img.currentSrc || img.src, document.baseURI);
      if (!name || src.protocol !== 'https:') throw Error('Unknown item');
      return { name, src: src.href };
    });
    const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
    let select;
    let originalQuality;
    try {
      const original = document.getElementById('app');
      const name = text(document.querySelector('h1'));
      select = document.querySelector('.augment-quality-select');
      if (!current() || !original || !name || !select ||
          !['1', '2', '3'].every(value => Array.from(select.options).some(option => option.value === value))) {
        throw Error('Unsupported page');
      }
      originalQuality = select.value;
      const augmentGroups = [];
      for (const [value, label] of [['1', english ? 'Silver' : '白银'], ['2', english ? 'Gold' : '黄金'], ['3', english ? 'Prismatic' : '棱彩']]) {
        if (!current()) throw Error('Champion changed');
        // Use the site's own public filter; restore it even when extraction fails.
        select.value = value;
        select.dispatchEvent(new Event('change', { bubbles: true }));
        await flush();
        const rows = Array.from(document.querySelectorAll('.augment-stat-row')).map(row => {
          const images = icons(row);
          if (images.length !== 1) throw Error('Unknown augment');
          const win = text(row.querySelector('strong'));
          percent(win);
          return { images, win, samples: count(text(row.querySelector('small'))) };
        }).sort((a, b) => b.samples - a.samples).slice(0, 3);
        augmentGroups.push({ label, rows });
        // A quality with no samples can unmount the filter. The retained select
        // still owns Vue's change handler and restores the original model.
      }
      const itemGroups = Array.from(document.querySelectorAll('.item-combo-column')).map(group => {
        const label = text(group.querySelector('h3'));
        if (!/^第[零一二三四五0-5]件$/.test(label)) throw Error('Unknown build stage');
        const rows = Array.from(group.querySelectorAll('.item-combo-row')).map(row => {
          const images = icons(row);
          if (!images.length) throw Error('Missing equipment');
          const win = text(row.querySelector('strong'));
          const pick = text(row.querySelector('b'));
          percent(win);
          return { images, win, pick, rate: percent(pick), samples: count(text(row.querySelector('em'))) };
        }).sort((a, b) => b.rate - a.rate || b.samples - a.samples).slice(0, 3);
        return { label: english ? 'Build stage ' + '零一二三四五'.indexOf(label[1]) : label, rows };
      });
      if (!augmentGroups.some(group => group.rows.length) || !itemGroups.some(group => group.rows.length)) {
        throw Error('Insufficient public data');
      }
      select.value = originalQuality;
      select.dispatchEvent(new Event('change', { bubbles: true }));
      await flush();
      if (!current()) throw Error('Champion changed');
      const element = (tag, label, className) => {
        const node = document.createElement(tag);
        if (label !== undefined) node.textContent = label;
        if (className) node.className = className;
        return node;
      };
      const panel = element('main');
      panel.id = rootId;
      panel.append(element('h1', name), element('p', english
        ? 'Popular picks · Top 3 per tier / build stage. Popularity is not a win-rate recommendation.'
        : '常用精选 · 每个品质 / 出装阶段最多 3 项。热门不等于最高胜率。', 'intro'));
      const addGroups = (heading, explanation, groups, type) => {
        panel.append(element('h2', heading), element('p', explanation, 'intro'));
        const grid = element('div', undefined, 'groups ' + type);
        for (const group of groups) {
          const card = element('section', undefined, 'card');
          card.append(element('h3', group.label));
          if (!group.rows.length) card.append(element('p', english ? 'No qualifying samples' : '暂无足够样本'));
          for (const row of group.rows) {
            const item = element('div', undefined, 'pick');
            const pictures = element('div', undefined, 'pictures');
            for (const icon of row.images) {
              const img = element('img');
              img.src = icon.src; img.alt = icon.name; img.title = icon.name;
              img.width = 30; img.height = 30;
              pictures.append(img);
            }
            item.append(pictures, element('div', row.images.map(image => image.name).join(' · '), 'names'));
            const stats = element('div', undefined, 'stats');
            if (row.pick) stats.append(element('b', (english ? 'Pick ' : '采用率 ') + row.pick));
            stats.append(element('span', (english ? 'Win ' : '胜率 ') + row.win),
              element('span', row.samples.toLocaleString('en-US') + (english ? ' games' : ' 场')));
            item.append(stats); card.append(item);
          }
          grid.append(card);
        }
        panel.append(grid);
      };
      addGroups(english ? 'Popular augments' : '热门海克斯', english
        ? 'Ranked by sample count within each tier; the source does not provide augment pick rates here.'
        : '按各品质的场次排序；原站此处未提供海克斯选取率，不将场次伪装为百分比。', augmentGroups, 'augments');
      addGroups(english ? 'Popular equipment builds' : '热门装备组合', english
        ? 'Ranked by adoption rate within each build stage.' : '按各出装阶段的采用率排序。', itemGroups, 'equipment');
      panel.append(element('footer', english
        ? 'Source: RESG · Switch to All data above for the original page, combinations and descriptions.'
        : '数据来源：RESG · 顶部切回“全部数据”可查看原始页面、完整组合与详细说明。'));
      const style = element('style');
      style.id = styleId;
      style.textContent = '#app{display:none!important}html,body{margin:0!important;background:' + (dark ? '#111111' : '#f5f7fb') + '!important}' +
        '#'+rootId+'{box-sizing:border-box;max-width:1100px;margin:auto;padding:16px;color:'+(dark?'#eeeeee':'#172033')+';font:14px/1.5 system-ui,sans-serif}' +
        '#'+rootId+' *{box-sizing:border-box}#'+rootId+' h1{font-size:22px;margin:0 0 6px}#'+rootId+' h2{font-size:17px;margin:22px 0 4px}' +
        '#'+rootId+' h3{font-size:14px;margin:0 0 4px;color:'+(dark?'#fbbf24':'#1d4ed8')+'}' +
        '#'+rootId+' .intro,#'+rootId+' footer{font-size:12px;color:'+(dark?'#b5b5bd':'#535d70')+';margin:4px 0 12px}' +
        '#'+rootId+' .groups{display:grid;gap:10px;grid-template-columns:repeat(3,minmax(0,1fr))}' +
        '#'+rootId+' .equipment{grid-template-columns:repeat(2,minmax(0,1fr))}' +
        '#'+rootId+' .card{min-width:0;border:1px solid '+(dark?'#36363c':'#d6dce6')+';background:'+(dark?'#1c1c20':'#ffffff')+';border-radius:8px;padding:10px}' +
        '#'+rootId+' .pick{padding:10px 0;border-top:1px solid '+(dark?'#333339':'#e4e8ef')+'}' +
        '#'+rootId+' .pictures{display:flex;gap:4px;flex-wrap:wrap}#'+rootId+' img{border-radius:4px;object-fit:contain}' +
        '#'+rootId+' .names{font-size:12px;overflow-wrap:anywhere;margin:5px 0}#'+rootId+' .stats{display:flex;flex-wrap:wrap;gap:3px 10px;font-size:12px}' +
        '#'+rootId+' footer{margin-top:20px}@media(max-width:540px){#'+rootId+' .groups{grid-template-columns:1fr}}';
      document.head.append(style);
      document.body.append(panel);
      document.body.setAttribute('data-mgs-resg-compact', String(championId));
      return { kind: 'compact', augmentCounts: augmentGroups.map(group => group.rows.length), itemCounts: itemGroups.map(group => group.rows.length) };
    } catch {
      reset();
      return { kind: 'fallback' };
    } finally {
      if (select && originalQuality !== undefined && current()) {
        select.value = originalQuality;
        select.dispatchEvent(new Event('change', { bubbles: true }));
        await flush();
      }
    }
  })()`
}
