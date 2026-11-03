import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';
import CosmeticLoadoutView from './CosmeticLoadoutView.jsx';
import gunCatalog from '../../../../../backend/app/cosmetic_studio/gun_finish_catalog.json';
import baseWeapons from '../../../../../backend/app/cosmetic_studio/base_weapon_metadata.json';
vi.mock('./SkyControls.jsx',()=>({default:()=>null}));

const mocks = vi.hoisted(() => ({ inventory: vi.fn(), settings: vi.fn(), apply: vi.fn(), status: vi.fn(), cancel: vi.fn(), play: vi.fn() }));
vi.mock('../../../api/cosmeticStudio.js', () => ({
  getAppearanceInventory: mocks.inventory, saveAppearanceSettings: mocks.settings,
  applyAppearancePlan: mocks.apply, getAppearanceJob: mocks.status, cancelAppearanceJob: mocks.cancel,
  playAppearanceDemo: mocks.play,
}));
const xuid = '76561199369875913';
const other = '76561199369875914';
const inventory = () => ({
  demo_sha256: 'a'.repeat(64), players: [{ xuid, name: 'Alice' }, { xuid: other, name: 'Bob' }],
  items: [
    { slot_id: 'ct:knife', xuid, team: 'ct', definition: 507, item_id: '123', name: '爪子刀 · 原始外观', wear: .3, seed: 561, editable: true, preset_ids: ['fade', 'butterfly'] },
    { slot_id: 't:ak', xuid, team: 't', definition: 7, item_id: '456', name: 'AK-47 · 原始外观', editable: false, reason: '无法确认原始归属', preset_ids: [] },
    { slot_id: 'ct:bob', xuid: other, team: 'ct', definition: 9, item_id: '789', name: 'AWP · 原始外观', editable: true, preset_ids: ['awp'] },
  ],
  presets: [
    { id: 'fade', label: '爪子刀 · 渐变之色', source_definition: 507, target_definition: 507, paint_index: 38, wear_min: 0, wear_max: .08 },
    { id: 'butterfly', label: '蝴蝶刀 · 渐变之色', source_definition: 507, target_definition: 515, paint_index: 38, wear_min: 0, wear_max: .08 },
    { id: 'awp', label: 'AWP · 巨龙传说', source_definition: 9, target_definition: 9, paint_index: 344, wear_min: 0, wear_max: .7 },
  ], settings: { core_exe: 'D:/core.exe', output_dir: 'D:/data/cosmetics' }, plans: {},
});
beforeEach(() => {
  vi.clearAllMocks();
  mocks.inventory.mockResolvedValue({ data: inventory() });
  mocks.apply.mockResolvedValue({ data: { id: 'job', state: 'file_verified', artifact: { playback: 'D:/copy.dem', visual_verified: null } } });
  mocks.settings.mockImplementation(async (settings) => ({ data: settings }));
  mocks.play.mockResolvedValue({ data: { session_id: 'offline', visual_verified: null } });
});
async function editKnife() {
  await screen.findByRole('option', { name: 'Alice' });
  fireEvent.click(screen.getByRole('button', { name: '更换 爪子刀 · 原始外观' }));
  return screen.getByRole('dialog', { name: '选择饰品外观' });
}
describe('reference-aligned appearance interaction', () => {
  test('every gun has its own stock image including the five tracker-hosted defaults',async()=>{
    const data=inventory();data.items=[];mocks.inventory.mockResolvedValue({data});
    const {container}=render(<CosmeticLoadoutView demoPath="D:/source.dem" selectedXuid={xuid} onQueueSave={vi.fn()}/>);
    await screen.findByRole('button',{name:'查看 USP-S'});
    const tiles=[...container.querySelectorAll('[data-weapon-definition]')];
    expect(tiles).toHaveLength(34);
    for(const tile of tiles)expect(tile.querySelector('img')?.getAttribute('src')).toBe(baseWeapons.items[tile.dataset.weaponDefinition].image_url);
  });
  test('a confirmed unpainted weapon uses its actual stock artwork, never a generic silhouette',async()=>{
    const data=inventory();data.items[0]={slot_id:'stock',xuid,team:'ct',definition:61,item_id:'123',name:'USP-S · 默认外观',paint_index:0,editable:false,reason:'missing_item_identity',preset_ids:[]};
    mocks.inventory.mockResolvedValue({data});
    const {container}=render(<CosmeticLoadoutView demoPath="D:/source.dem" selectedXuid={xuid} onQueueSave={vi.fn()}/>);
    const tile=await screen.findByRole('button',{name:'查看 USP-S · 默认外观'});
    expect(tile.querySelector('img')?.getAttribute('src')).toBe(baseWeapons.items['61'].image_url);
    expect(tile.querySelector('svg')).toBeNull();
    expect(tile.querySelector('[data-artwork-kind]')?.getAttribute('data-artwork-kind')).toBe('stock');
    fireEvent.error(tile.querySelector('img'));
    expect(tile.textContent).toContain('图片加载失败');
    expect(container.querySelector('.queue-buy-columns svg')).toBeNull();
  });
  test('queue artwork remains visible while loading and retries a newly equipped image after failure',async()=>{
    const data=inventory();data.items[0].image_url='https://community.cloudflare.steamstatic.com/original.png';
    data.presets[0].image_url='https://community.cloudflare.steamstatic.com/replacement.png';
    mocks.inventory.mockResolvedValue({data});
    const {container}=render(<CosmeticLoadoutView demoPath="D:/source.dem" selectedXuid={xuid} onQueueSave={vi.fn()}/>);
    await screen.findByRole('button',{name:'更换 爪子刀 · 原始外观'});
    const tile=screen.getByRole('button',{name:'更换 爪子刀 · 原始外观'});
    expect(tile.textContent).toContain('图片加载中');
    fireEvent.error(tile.querySelector('img'));
    fireEvent.click(tile);fireEvent.click(screen.getByRole('button',{name:'装备',exact:true}));
    await waitFor(()=>expect(tile.querySelector('img')?.getAttribute('src')).toContain('replacement.png'));
  });
  test('queue skin editing saves a profile without creating a file job or changing target player',async()=>{
    const saved=vi.fn();
    render(<CosmeticLoadoutView demoPath="D:/source.dem" selectedXuid={xuid} queueProfile={null} onQueueSave={saved}/>);
    await screen.findByRole('button',{name:'更换 爪子刀 · 原始外观'});
    expect(screen.getByRole('button',{name:'CT'})).toBeTruthy();
    expect(screen.getByRole('button',{name:'T'})).toBeTruthy();
    expect(screen.queryByRole('combobox')).toBeNull();
    fireEvent.click(screen.getByRole('button',{name:'更换 爪子刀 · 原始外观'}));
    fireEvent.click(screen.getByRole('button',{name:'装备'}));
    fireEvent.click(screen.getByRole('button',{name:'保存录制外观'}));
    expect(saved.mock.calls[0][0]).toEqual({expected_demo_sha256:'a'.repeat(64),scope:'match',replacements:[{slot_id:'ct:knife',preset_id:'fade',wear:.01,seed:0}]});
    expect(mocks.apply).not.toHaveBeenCalled(); expect(mocks.play).not.toHaveBeenCalled();
  });
  test('queue restores original by clearing its profile rather than applying shared Demo plans',async()=>{
    const saved=vi.fn(); const data=inventory(); data.plans={[xuid]:[{slot_id:'ct:knife',preset_id:'butterfly',wear:.01,seed:0}]};
    mocks.inventory.mockResolvedValueOnce({data});
    render(<CosmeticLoadoutView demoPath="D:/source.dem" selectedXuid={xuid} onQueueSave={saved}/>);
    await screen.findByRole('button',{name:'恢复原皮肤'});
    fireEvent.click(screen.getByRole('button',{name:'恢复原皮肤'}));
    expect(saved.mock.calls[0][0]).toBeNull(); expect(mocks.apply).not.toHaveBeenCalled();
  });
  test('default whole-match groups repeat purchases and equips every matching owned gun',async()=>{
    const data=inventory();
    data.items=[{...data.items[2],xuid,slot_id:'awp:first'},
      {...data.items[2],xuid,slot_id:'awp:rebuy',item_id:'790',team:'t'}];
    mocks.inventory.mockResolvedValueOnce({data});
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByRole('option',{name:'Alice'});
    expect(screen.getByText('整局应用')).toBeTruthy();
    expect(screen.getAllByRole('button',{name:'更换 AWP · 原始外观'})).toHaveLength(1);
    fireEvent.click(screen.getByRole('button',{name:'更换 AWP · 原始外观'}));
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button',{name:'装备'}));
    fireEvent.click(screen.getByRole('button',{name:'保存 Demo 副本'}));
    await waitFor(()=>expect(mocks.apply).toHaveBeenCalled());
    expect(mocks.apply.mock.calls[0][0].scope).toBe('match');
    expect(mocks.apply.mock.calls[0][0].replacements.map(row=>row.slot_id)).toEqual(['awp:first','awp:rebuy']);
  });
  test('full catalog renders only the selected gun and saves a searched skin with legal defaults', async () => {
    const expanded = inventory();
    expanded.presets = Object.entries(gunCatalog.items).map(([key, row]) => {
      const [definition, paint] = key.split(':').map(Number);
      return { id: `gun-${definition}-${paint}`, label: row.name, source_definition: definition,
        target_definition: definition, paint_index: paint, wear_min: row.wear_min,
        wear_max: row.wear_max, image_url: row.image_url };
    });
    expanded.items = [{ slot_id: 'ct:five-seven', xuid, team: 'ct', definition: 3,
      item_id: '123', name: 'FN57 · 原始外观', editable: true,
      preset_ids: expanded.presets.filter(p => p.source_definition === 3).map(p => p.id) }];
    mocks.inventory.mockResolvedValueOnce({ data: expanded });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByRole('option', { name: 'Alice' });
    fireEvent.click(screen.getByRole('button', { name: '更换 FN57 · 原始外观' }));
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getAllByRole('button', { name: /^选择 / })).toHaveLength(42);
    expect(dialog.querySelectorAll('img').length).toBeLessThan(50);
    expect([...dialog.querySelectorAll('img')].every(img => img.getAttribute('loading') === 'lazy')).toBe(true);
    fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: '蓝莓樱桃' } });
    fireEvent.click(within(dialog).getByRole('button', { name: '选择 FN57 | 蓝莓樱桃' }));
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await screen.findByText(/制片副本已校验/);
    expect(mocks.apply).toHaveBeenCalledWith({ scope:'match',demo_path: 'D:/source.dem', expected_demo_sha256: 'a'.repeat(64),
      xuid, replacements: [{ slot_id: 'ct:five-seven', preset_id: 'gun-3-1002', wear: .01, seed: 0 }] });
  });

  test('equipment slots open directly and changing players closes the picker', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByRole('option', { name: 'Alice' });
    const slot = screen.getByRole('button', { name: '更换 爪子刀 · 原始外观' });
    expect(slot.disabled).toBe(false);
    fireEvent.click(slot);
    expect(screen.getByRole('dialog', { name: '选择饰品外观' })).toBeTruthy();
    fireEvent.change(screen.getByRole('combobox', { name: '目标玩家' }), { target: { value: other } });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByRole('button', { name: '更换 AWP · 原始外观' }).disabled).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: '更换 AWP · 原始外观' }));
    expect(within(screen.getByRole('dialog')).getByRole('button', { name: '选择 AWP · 巨龙传说' })).toBeTruthy();
  });

  test('no supported items explains why editing is unavailable instead of an empty editor', async () => {
    const readonly = inventory();
    readonly.items = readonly.items.filter(item => !item.editable);
    mocks.inventory.mockResolvedValue({ data: readonly });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByRole('option', { name: 'Alice' });
    expect(screen.getByText(/当前玩家暂无可编辑装备/)).toBeTruthy();
    expect(screen.getByRole('button', { name: '保存 Demo 副本' }).disabled).toBe(true);
    expect(screen.getByText(/^暂不支持替换 ·/).closest('details').open).toBe(false);
  });
  test('equip works with defaults and keeps technical parameters collapsed', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    expect(within(dialog).getByText('高级选项').closest('details').open).toBe(false);
    expect(document.activeElement).toBe(within(dialog).getByRole('searchbox'));
    fireEvent.click(within(dialog).getByRole('button', { name: '选择 蝴蝶刀 · 渐变之色' }));
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.getByText('蝴蝶刀 · 渐变之色')).toBeTruthy();
    expect(screen.getByText('待保存')).toBeTruthy();
    expect(mocks.apply).not.toHaveBeenCalled();
    expect(mocks.play).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await screen.findByText(/制片副本已校验/);
    expect(mocks.apply).toHaveBeenCalledWith({ scope:'match',demo_path: 'D:/source.dem', expected_demo_sha256: 'a'.repeat(64), xuid,
      replacements: [{ slot_id: 'ct:knife', preset_id: 'butterfly', wear: .01, seed: 0 }] });
  });
  test('discard selections restores saved equipment without rewriting or launching', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '撤销本次选择' }));
    expect(screen.getByText('爪子刀 · 原始外观')).toBeTruthy();
    expect(screen.queryByText('待保存')).toBeNull();
    expect(screen.getByRole('button', { name: '保存 Demo 副本' }).disabled).toBe(true);
    expect(mocks.apply).not.toHaveBeenCalled();
    expect(mocks.play).not.toHaveBeenCalled();
  });
  test('qualified zero-ID equipment uses the same picker without sending a fabricated item ID', async () => {
    const observed = inventory();
    observed.items = [{ slot_id: 'observed:usp', xuid, team: 'ct', definition: 61,
      item_id: '0', name: 'USP-S · 原始外观', paint_index: 1142,
      editable: true, reason: null, preset_ids: ['usp-kill-confirmed'] }];
    observed.presets = [{ id: 'usp-kill-confirmed', label: 'USP-S · 枪响人亡',
      source_definition: 61, target_definition: 61, paint_index: 504, wear_min: 0, wear_max: 1 }];
    mocks.inventory.mockResolvedValue({ data: observed });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByText('USP-S · 原始外观');
    expect(screen.getByText('这份 Demo 的 USP-S 现在可生成替换副本；其他未适配装备仍不可编辑。')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '更换 USP-S · 原始外观' }));
    fireEvent.click(within(screen.getByRole('dialog', { name: '选择饰品外观' })).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await waitFor(() => expect(mocks.apply).toHaveBeenCalledOnce());
    expect(mocks.apply.mock.calls[0][0]).toEqual({ scope:'match',demo_path: 'D:/source.dem', expected_demo_sha256: 'a'.repeat(64), xuid,
      replacements: [{ slot_id: 'observed:usp', preset_id: 'usp-kill-confirmed', wear: expect.any(Number), seed: expect.any(Number) }] });
    expect(mocks.play).not.toHaveBeenCalled();
  });
  test('shows observed equipment and the actual compatibility reason without pretending it can be edited', async () => {
    const observed = inventory();
    observed.items = [{ slot_id: 'observed:usp', xuid, team: 'ct', definition: 61,
      item_id: '0', name: 'USP-S · 原始外观', paint_index: 1142,
      editable: false, reason: 'missing_item_identity', preset_ids: [] }];
    mocks.inventory.mockResolvedValue({ data: observed });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByText('USP-S · 原始外观');
    expect(screen.getByRole('button', { name: '更换 USP-S · 原始外观' }).disabled).toBe(true);
    expect(screen.getByText(/缺少有效的饰品编号/)).toBeTruthy();
    expect(screen.queryByText('missing_item_identity')).toBeNull();
    expect(screen.queryByText('此 Demo 没有可确认的装备记录')).toBeNull();
    expect(mocks.apply).not.toHaveBeenCalled();
  });
  test('uses the selected appearance name and readable save status instead of internal slot identifiers', async () => {
    mocks.apply.mockResolvedValue({ data: { id: 'job', state: 'file_verified',
      artifact: { playback: 'D:/copy.dem', visual_verified: null },
      items: [{ slot_id: 'ct:knife', preset_id: 'fade', status: 'applied' }],
    } });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    expect(await screen.findByText('爪子刀 · 渐变之色：已应用到制片副本')).toBeTruthy();
    expect(screen.queryByText('ct:knife：applied')).toBeNull();
  });
  test('inherits the Demo and renders roster and separate CT/T equipment without technical inputs', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByText('爪子刀 · 原始外观');
    expect(mocks.inventory).toHaveBeenCalledWith('D:/source.dem');
    expect(screen.getByRole('heading', { name: /CT 装备/ })).toBeTruthy();
    expect(screen.getByRole('heading', { name: /^T 装备/ })).toBeTruthy();
    expect(screen.queryByLabelText(/XUID|tick|本地 Demo 路径|OBS 路径/)).toBeNull();
    expect(mocks.apply).not.toHaveBeenCalled();
  });
  test('confirms a local draft only, then save explicitly generates the copy', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '选择 蝴蝶刀 · 渐变之色' }));
    fireEvent.change(within(dialog).getByLabelText('磨损数值'), { target: { value: '.04' } });
    fireEvent.change(within(dialog).getByLabelText('图案模板数值'), { target: { value: '123' } });
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    expect(mocks.apply).not.toHaveBeenCalled();
    expect(screen.getByText('待保存')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await waitFor(() => expect(mocks.apply).toHaveBeenCalledWith({
      scope:'match',demo_path: 'D:/source.dem', expected_demo_sha256: 'a'.repeat(64), xuid,
      replacements: [{ slot_id: 'ct:knife', preset_id: 'butterfly', wear: .04, seed: 123 }],
    }));
    expect(await screen.findByText(/制片副本已校验/)).toBeTruthy();
    expect(screen.getByText(/尚未确认原生回放画面/)).toBeTruthy();
  });
  test('cancel picker leaves no draft and enforces finish-specific wear limits', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '选择 蝴蝶刀 · 渐变之色' }));
    expect(within(dialog).getByLabelText('磨损数值').max).toBe('0.08');
    fireEvent.change(within(dialog).getByLabelText('磨损数值'), { target: { value: '.1' } });
    expect(within(dialog).getByRole('button', { name: '装备' }).disabled).toBe(true);
    fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));
    expect(screen.queryByText('待保存')).toBeNull();
    expect(mocks.apply).not.toHaveBeenCalled();
  });
  test('unknown ownership remains read-only with a reason instead of a fabricated slot', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await editKnife();
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    expect(screen.getByRole('button', { name: '更换 AK-47 · 原始外观' }).disabled).toBe(true);
    expect(screen.getByText('无法确认原始归属')).toBeTruthy();
  });
  test('switching players discards draft and does not retain another player’s replacement', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '选择 蝴蝶刀 · 渐变之色' }));
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.change(screen.getByLabelText('目标玩家'), { target: { value: other } });
    expect(await screen.findByText('AWP · 原始外观')).toBeTruthy();
    expect(screen.queryByText('蝴蝶刀 · 渐变之色')).toBeNull();
    expect(screen.queryByText('待保存')).toBeNull();
  });
  test('source switch ignores a late inventory result', async () => {
    let resolveOld;
    mocks.inventory.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
    const next = inventory(); next.players = [{ xuid: other, name: 'New source' }]; next.items = []; next.demo_sha256 = 'b'.repeat(64);
    mocks.inventory.mockResolvedValueOnce({ data: next });
    const view = render(<CosmeticLoadoutView demoPath="D:/old.dem" />);
    view.rerender(<CosmeticLoadoutView demoPath="D:/new.dem" />);
    await screen.findByRole('option', { name: 'New source' });
    await act(async () => resolveOld({ data: inventory() }));
    expect(screen.queryByText('爪子刀 · 原始外观')).toBeNull();
    expect(mocks.apply).not.toHaveBeenCalled();
  });
  test('restores original through an explicit empty player plan, not just hiding the card', async () => {
    const saved = inventory(); saved.plans[xuid] = [{ slot_id: 'ct:knife', preset_id: 'fade', wear: .04, seed: 2 }];
    mocks.inventory.mockResolvedValueOnce({ data: saved });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByText('爪子刀 · 渐变之色');
    fireEvent.click(screen.getByRole('button', { name: '恢复原始 爪子刀 · 原始外观' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await waitFor(() => expect(mocks.apply).toHaveBeenCalledWith(expect.objectContaining({ xuid, replacements: [] })));
  });
  test('settings are collapsed and saved once without game or recording prerequisites', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByRole('option', { name: 'Alice' });
    expect(screen.getByText('高级设置').closest('details').open).toBe(false);
    fireEvent.change(screen.getByLabelText('外观工具路径'), { target: { value: 'D:/new-core.exe' } });
    fireEvent.click(screen.getByRole('button', { name: '保存设置' }));
    await waitFor(() => expect(mocks.settings).toHaveBeenCalledWith({ core_exe: 'D:/new-core.exe', output_dir: 'D:/data/cosmetics' }));
    expect(mocks.apply).not.toHaveBeenCalled();
  });
  test('picker closes on Escape and restores keyboard focus to the slot', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await editKnife();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: '更换 爪子刀 · 原始外观' }));
  });
  test('native playback requires a separate explicit offline acknowledgement after file save', async () => {
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await screen.findByText(/制片副本已校验/);
    expect(mocks.play).not.toHaveBeenCalled();
    const play = screen.getByRole('button', { name: '播放制片副本' });
    expect(play.disabled).toBe(true);
    fireEvent.click(screen.getByLabelText('只启动专用离线 Demo 回放（-insecure）'));
    fireEvent.click(play);
    await waitFor(() => expect(mocks.play).toHaveBeenCalledWith('job'));
    expect(screen.getByText(/尚未确认原生回放画面/)).toBeTruthy();
  });
  test('changing the Demo while save is in flight cancels a late job instead of showing stale success', async () => {
    let resolveJob;
    mocks.apply.mockImplementationOnce(() => new Promise((resolve) => { resolveJob = resolve; }));
    const view = render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    view.rerender(<CosmeticLoadoutView demoPath="D:/other.dem" />);
    await act(async () => resolveJob({ data: { id: 'late-job', state: 'running' } }));
    expect(mocks.cancel).toHaveBeenCalledWith('late-job');
    expect(screen.queryByText(/制片副本已校验/)).toBeNull();
  });
  test('a failed apply retains the draft and never reports a verified copy', async () => {
    mocks.apply.mockResolvedValueOnce({ data: { id: 'bad', state: 'failed', error: '独立校验失败' } });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await screen.findByText('独立校验失败');
    expect(screen.getByText('待保存')).toBeTruthy();
    expect(screen.queryByText(/制片副本已校验/)).toBeNull();
    expect(mocks.play).not.toHaveBeenCalled();
  });
  test('restores the persisted verified job from the backend artifact after reopening', async () => {
    const restored = inventory();
    restored.artifact = { job_id: 'persisted-job', playback_path: 'D:/saved.dem', visual_verified: null };
    mocks.inventory.mockResolvedValueOnce({ data: restored });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByText(/制片副本已校验/);
    fireEvent.click(screen.getByLabelText('只启动专用离线 Demo 回放（-insecure）'));
    fireEvent.click(screen.getByRole('button', { name: '播放制片副本' }));
    await waitFor(() => expect(mocks.play).toHaveBeenCalledWith('persisted-job'));
  });
  test('equipment observed on both teams is presented as shared, not unknown ownership', async () => {
    const shared = inventory(); shared.items[0].team = 'both';
    mocks.inventory.mockResolvedValueOnce({ data: shared });
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    await screen.findByRole('heading', { name: /CT\/T 通用装备/ });
    expect(screen.queryByRole('heading', { name: /未确认阵营/ })).toBeNull();
  });
  test('does not allow selecting another player or editing during a pending native launch', async () => {
    let resolveLaunch;
    mocks.play.mockImplementationOnce(() => new Promise(resolve => { resolveLaunch = resolve; }));
    render(<CosmeticLoadoutView demoPath="D:/source.dem" />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    fireEvent.click(screen.getByRole('button', { name: '保存 Demo 副本' }));
    await screen.findByText(/制片副本已校验/);
    fireEvent.click(screen.getByLabelText('只启动专用离线 Demo 回放（-insecure）'));
    fireEvent.click(screen.getByRole('button', { name: '播放制片副本' }));
    expect(screen.getByLabelText('目标玩家').disabled).toBe(true);
    expect(screen.getByRole('button', { name: '更换 爪子刀 · 原始外观' }).disabled).toBe(true);
    await act(async () => resolveLaunch({ data: {} }));
    expect(screen.getByLabelText('目标玩家').disabled).toBe(false);
  });
  test('inherits a new selected analysis player while the same Demo stays mounted', async () => {
    const view = render(<CosmeticLoadoutView demoPath="D:/source.dem" selectedXuid={xuid} />);
    const dialog = await editKnife();
    fireEvent.click(within(dialog).getByRole('button', { name: '装备' }));
    view.rerender(<CosmeticLoadoutView demoPath="D:/source.dem" selectedXuid={other} />);
    await waitFor(() => expect(screen.getByLabelText('目标玩家').value).toBe(other));
    expect(screen.getByText('AWP · 原始外观')).toBeTruthy();
    expect(screen.queryByText('待保存')).toBeNull();
    expect(mocks.apply).not.toHaveBeenCalled();
  });
});
