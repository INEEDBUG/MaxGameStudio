import { fireEvent, render, screen, waitFor, act } from "@testing-library/react";
import { beforeEach, afterEach, describe, expect, test, vi } from "vitest";
import { MemoryRouter, useLocation } from "react-router-dom";
import CosmeticStudioPage from "./CosmeticStudioPage";

const mocks = vi.hoisted(() => ({
  getCapabilities: vi.fn(), active: vi.fn(), inspect: vi.fn(), create: vi.fn(), getJob: vi.fn(), preview: vi.fn(), confirm: vi.fn(), cancel: vi.fn(),
  profiles:vi.fn(), saveProfile:vi.fn(), deleteProfile:vi.fn(), importDirector:vi.fn(), validateInterval:vi.fn(),
}));

vi.mock("../api/cosmeticStudio.js", () => ({
  getCosmeticCapabilities: mocks.getCapabilities,
  getActiveCosmeticJob: mocks.active,
  inspectCosmeticDemo: mocks.inspect,
  createCosmeticJob: mocks.create,
  getCosmeticJob: mocks.getJob,
  getCosmeticPreviewBlob: mocks.preview,
  confirmCosmeticJob: mocks.confirm,
  cancelCosmeticJob: mocks.cancel,
  getCosmeticProfiles:mocks.profiles,
  saveCosmeticProfile:mocks.saveProfile,
  deleteCosmeticProfile:mocks.deleteProfile,
  importCosmeticDirectorPlan:mocks.importDirector,
  validateCosmeticInterval:mocks.validateInterval,
}));

describe("CosmeticStudioPage", () => {
  test('offers a qualified replacement knife for its source model and submits the target preset', async () => {
    mocks.getCapabilities.mockResolvedValueOnce({data:{enabled:true,providers:[{id:'skin_core',label:'Demo 制片副本'}],
      presets:[{id:'karambit-fade',weapon_id:507,paint_kit:38,label:'爪子刀 · 渐变之色',provider_ids:['skin_core']},
        {id:'karambit-to-butterfly-fade',weapon_id:515,source_weapon_id:507,paint_kit:38,
          label:'蝴蝶刀 · 渐变之色（原刀：爪子刀）',provider_ids:['skin_core']}],
      defaults:{cs2_exe:'cs2',hlae_exe:'hlae',core_exe:'core',obs_exe:'obs',ffmpeg_exe:'ffmpeg',ffprobe_exe:'probe',output_dir:'out'}}});
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),candidates:[
      {xuid:'76561199369875913',start_tick:67431,end_tick:67687,weapon_id:507,label:'原始爪子刀片段'}]}});
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    const target = screen.getByRole('option',{name:'蝴蝶刀 · 渐变之色（原刀：爪子刀）'});
    expect(target.value).toBe('karambit-to-butterfly-fade');
    fireEvent.change(screen.getByLabelText('皮肤预设'),{target:{value:target.value}});
    fireEvent.click(screen.getByRole('button',{name:'启动预览'}));
    await waitFor(()=>expect(mocks.create).toHaveBeenCalledWith(expect.objectContaining({
      preset_id:'karambit-to-butterfly-fade',xuid:'76561199369875913',glove_preset_id:null})));
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  test('uses the qualified wear limit for a replacement knife', async () => {
    mocks.getCapabilities.mockResolvedValueOnce({data:{enabled:true,providers:[{id:'skin_core',label:'Demo 制片副本'}],
      presets:[{id:'karambit-to-butterfly-fade',weapon_id:515,source_weapon_id:507,paint_kit:38,
        label:'蝴蝶刀 · 渐变之色',provider_ids:['skin_core']}],defaults:{}}});
    render(<CosmeticStudioPage />);
    const wear = await screen.findByLabelText('磨损');
    expect(wear.max).toBe('0.08');
    fireEvent.change(wear,{target:{value:'0.09'}});
    expect(screen.getByText('请输入当前外观允许范围内的有效磨损。')).toBeTruthy();
  });

  test('keeps original gloves by default and submits an explicit optional glove preset',async()=>{
    mocks.getCapabilities.mockResolvedValueOnce({data:{enabled:true,providers:[{id:'skin_core',label:'Demo 制片副本'}],
      presets:[{id:'karambit-fade',weapon_id:507,paint_kit:38,label:'爪子刀',provider_ids:['skin_core']}],
      glove_presets:[{id:'specialist-10061',label:'专家手套 · 10061',weapon_ids:[507]}],
      defaults:{cs2_exe:'cs2',hlae_exe:'hlae',core_exe:'core',obs_exe:'obs',ffmpeg_exe:'ffmpeg',ffprobe_exe:'probe',output_dir:'out'}}});
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),
      candidates:[{xuid:'76561199369875913',start_tick:1000,end_tick:1128,weapon_id:507,label:'爪子刀片段'}]}});
    render(<CosmeticStudioPage />);
    const gloves=await screen.findByLabelText('手套外观');
    expect(gloves.value).toBe('');
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    fireEvent.change(gloves,{target:{value:'specialist-10061'}});
    fireEvent.click(screen.getByRole('button',{name:'启动预览'}));
    await waitFor(()=>expect(mocks.create).toHaveBeenCalledWith(expect.objectContaining({
      xuid:'76561199369875913',preset_id:'karambit-fade',glove_preset_id:'specialist-10061'})));
    expect(mocks.confirm).not.toHaveBeenCalled();
  });
  test('drops the glove attachment when switching to an unqualified weapon',async()=>{
    mocks.getCapabilities.mockResolvedValueOnce({data:{enabled:true,providers:[{id:'skin_core',label:'Demo 制片副本'}],
      presets:[{id:'karambit-fade',weapon_id:507,paint_kit:38,label:'爪子刀',provider_ids:['skin_core']},
        {id:'ak47-fire-serpent',weapon_id:7,paint_kit:180,label:'AK',provider_ids:['skin_core']}],
      glove_presets:[{id:'specialist-10061',label:'专家手套 · 10061',weapon_ids:[507]}],
      defaults:{cs2_exe:'cs2',hlae_exe:'hlae',core_exe:'core',obs_exe:'obs',ffmpeg_exe:'ffmpeg',ffprobe_exe:'probe',output_dir:'out'}}});
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),candidates:[
      {xuid:'123',start_tick:1000,end_tick:1128,weapon_id:507,label:'刀片段'},
      {xuid:'123',start_tick:2000,end_tick:2128,weapon_id:7,label:'AK 片段'}]}});
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    fireEvent.change(screen.getByLabelText('手套外观'),{target:{value:'specialist-10061'}});
    fireEvent.change(screen.getByLabelText('目标玩家与片段'),{target:{value:'123:2000:2128'}});
    expect(screen.queryByLabelText('手套外观')).toBeNull();
    fireEvent.click(screen.getByRole('button',{name:'启动预览'}));
    await waitFor(()=>expect(mocks.create).toHaveBeenCalledWith(expect.objectContaining({
      preset_id:'ak47-fire-serpent',glove_preset_id:null})));
  });
  test('blocks job start while manual validation is pending',async()=>{
    mocks.getCapabilities.mockResolvedValueOnce({data:{enabled:true,providers:[{id:'skin_core',label:'Demo 制片副本'}],
      presets:[{id:'karambit-fade',weapon_id:507,paint_kit:38,label:'爪子刀',provider_ids:['skin_core']}],
      defaults:{cs2_exe:'cs2',hlae_exe:'hlae',core_exe:'core',obs_exe:'obs',ffmpeg_exe:'ffmpeg',ffprobe_exe:'probe',output_dir:'out'}}});
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),players:[{xuid:'123',name:'Player'}],
      candidates:[{xuid:'123',start_tick:1000,end_tick:1128,weapon_id:507,label:'已有刀片段'}]}});
    mocks.validateInterval.mockImplementationOnce(()=>new Promise(()=>{}));
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    fireEvent.change(await screen.findByLabelText('刀展示开始 tick'),{target:{value:'67431'}});
    fireEvent.change(screen.getByLabelText('刀展示结束 tick'),{target:{value:'67687'}});
    fireEvent.click(screen.getByRole('button',{name:'校验刀展示片段'}));
    await waitFor(()=>expect(mocks.validateInterval).toHaveBeenCalled());
    expect(screen.getByRole('button',{name:'启动预览'}).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button',{name:'启动预览'}));
    expect(mocks.create).not.toHaveBeenCalled();
  });

  test('locks selection while job creation request is pending',async()=>{
    mocks.create.mockImplementationOnce(()=>new Promise(()=>{}));
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    fireEvent.click(screen.getByRole('button',{name:'启动预览'}));
    await waitFor(()=>expect(mocks.create).toHaveBeenCalled());
    expect(screen.getByLabelText('目标玩家与片段').disabled).toBe(true);
    expect(screen.getByRole('button',{name:/检查 Demo/}).disabled).toBe(true);
  });
  test('validates a manual knife window without automatically launching or approving',async()=>{
    mocks.getCapabilities.mockResolvedValueOnce({data:{enabled:true,providers:[{id:'skin_core',label:'Demo 制片副本'}],
      presets:[{id:'karambit-fade',weapon_id:507,paint_kit:38,label:'爪子刀 · 渐变之色',provider_ids:['skin_core']}],defaults:{}}});
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),
      players:[{xuid:'76561199369875913',name:'Player'}],candidates:[]}});
    mocks.validateInterval.mockResolvedValueOnce({data:{xuid:'76561199369875913',start_tick:67431,end_tick:67687,
      weapon_id:507,label:'爪子刀 · 手动片段'}});
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    fireEvent.change(await screen.findByLabelText('刀展示开始 tick'),{target:{value:'67431'}});
    fireEvent.change(screen.getByLabelText('刀展示结束 tick'),{target:{value:'67687'}});
    fireEvent.click(screen.getByRole('button',{name:'校验刀展示片段'}));
    await waitFor(()=>expect(mocks.validateInterval).toHaveBeenCalledWith({demo_path:'D:/demo.dem',
      expected_demo_sha256:'a'.repeat(64),xuid:'76561199369875913',start_tick:67431,end_tick:67687,weapon_id:507}));
    expect(await screen.findByLabelText('目标玩家与片段')).toBeTruthy();
    expect(mocks.create).not.toHaveBeenCalled();expect(mocks.confirm).not.toHaveBeenCalled();
  });
  test('imports director intervals only after inspection without starting recording',async()=>{
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),candidates:[{xuid:'123',start_tick:1000,end_tick:1128,label:'目标'}]}});
    mocks.importDirector.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),candidates:[{xuid:'123',start_tick:1000,end_tick:1128,label:'导演片段'}]}});
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    const plan={demo_path:'D:/demo.dem',tick_rate:64,active_segments:[]};
    const file=new File([JSON.stringify(plan)],'plan.json',{type:'application/json'});
    file.text=async()=>JSON.stringify(plan);
    fireEvent.change(screen.getByLabelText('导入导演计划 JSON'),{target:{files:[file]}});
    await waitFor(()=>expect(mocks.importDirector).toHaveBeenCalledWith({demo_path:'D:/demo.dem',expected_demo_sha256:'a'.repeat(64),plan}));
    expect(await screen.findByRole('option',{name:'导演片段'})).toBeTruthy();
    expect(mocks.create).not.toHaveBeenCalled();expect(mocks.confirm).not.toHaveBeenCalled();
  });
  test('filters presets by selected weapon before launching', async () => {
    mocks.getCapabilities.mockResolvedValueOnce({data: {enabled: true,
      defaults: {cs2_exe:'cs2', hlae_exe:'hlae', core_exe:'core', obs_exe:'obs', ffmpeg_exe:'ffmpeg', ffprobe_exe:'ffprobe', output_dir:'out'},
      providers: [{id:'skin_core', label:'Demo 副本'}], presets: [
        {id:'awp-dragon-lore', label:'AWP', weapon_id:9, provider_ids:['skin_core']},
        {id:'ak47-fire-serpent', label:'AK 火蛇', weapon_id:7, provider_ids:['skin_core']}]}});
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64),candidates:[{xuid:'123',start_tick:1000,end_tick:1128,label:'AK 片段',weapon_id:7}]}});
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'), {target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button', {name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    expect(screen.getByLabelText('皮肤预设').value).toBe('ak47-fire-serpent');
    expect(screen.queryByRole('option', {name:'AWP'})).toBeNull();
    fireEvent.click(screen.getByRole('button', {name:/启动预览/}));
    await waitFor(() => expect(mocks.create).toHaveBeenCalledWith(expect.objectContaining({preset_id:'ak47-fire-serpent',xuid:'123',expected_demo_sha256:'a'.repeat(64)})));
  });
  function LocationProbe() {
    return <output data-testid="location-probe">{useLocation().pathname}</output>;
  }

  beforeEach(() => {
    vi.useRealTimers();
    mocks.getCapabilities.mockResolvedValue({ data: { enabled: true, defaults: { cs2_exe: "cs2", hlae_exe: "hlae", nskinz_dll: "dll", obs_exe: "obs", ffmpeg_exe: "ffmpeg", ffprobe_exe: "ffprobe", output_dir: "out" }, presets: [{ id: "awp-dragon-lore", label: "AWP · 巨龙传说", weapon_id: 9, paint_kit: 344 }] } });
    mocks.active.mockResolvedValue({ data: null });
    mocks.profiles.mockResolvedValue({data:[]});
    mocks.saveProfile.mockImplementation(async body=>({data:{...body,id:'a'.repeat(32)}}));
    mocks.deleteProfile.mockResolvedValue({data:null});
    mocks.inspect.mockResolvedValue({ data: { demo_sha256: "demo-sha", candidates: [{ xuid: "123", start_tick: 10, end_tick: 20, label: "目标" }] } });
    mocks.create.mockResolvedValue({ data: { id: "job-1", state: "preparing", message: "准备中" } });
    mocks.getJob.mockResolvedValue({ data: { id: "job-1", state: "preparing", message: "准备中" } });
    mocks.preview.mockResolvedValue({ data: new Blob(["preview"], { type: "image/png" }) });
    mocks.confirm.mockResolvedValue({ data: { id: "job-1", state: "recording", message: "录制中" } });
    mocks.cancel.mockResolvedValue({ data: { id: "job-1", state: "cancelled", message: "已取消" } });
    vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:preview"), revokeObjectURL: vi.fn() });
  });

  afterEach(() => { vi.restoreAllMocks(); });

  test('saves exact player and inspected segment without launching a job', async()=>{
    mocks.inspect.mockResolvedValueOnce({data:{demo_sha256:'a'.repeat(64), candidates:[
      {xuid:'123',start_tick:1000,end_tick:1128,label:'目标',weapon_id:9}]}});
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    fireEvent.change(screen.getByLabelText('预设名称'),{target:{value:'我的巨龙'}});
    fireEvent.click(screen.getByRole('button',{name:'保存玩家预设'}));
    await waitFor(()=>expect(mocks.saveProfile).toHaveBeenCalledWith(expect.objectContaining({
      name:'我的巨龙',xuid:'123',demo_sha256:'a'.repeat(64),start_tick:1000,end_tick:1128,preset_id:'awp-dragon-lore'})));
    expect(mocks.create).not.toHaveBeenCalled();
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  test('refuses to apply a saved profile bound to another Demo',async()=>{
    mocks.profiles.mockResolvedValueOnce({data:[{id:'b'.repeat(32),name:'另一个Demo',xuid:'123',
      demo_sha256:'b'.repeat(64),start_tick:1000,end_tick:1128,preset_id:'awp-dragon-lore',pattern_seed:5,wear:.05}]});
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText('本地 Demo 路径'),{target:{value:'D:/demo.dem'}});
    fireEvent.click(screen.getByRole('button',{name:/检查 Demo/}));
    await screen.findByLabelText('目标玩家与片段');
    fireEvent.change(screen.getByLabelText('已保存玩家预设'),{target:{value:'b'.repeat(32)}});
    fireEvent.click(screen.getByRole('button',{name:'应用预设'}));
    expect(await screen.findByText(/预设的 Demo、玩家或片段与当前选择不匹配/)).toBeTruthy();
    expect(mocks.create).not.toHaveBeenCalled();
  });

  test("disabled capability does not allow starting", async () => {
    mocks.getCapabilities.mockResolvedValue({ data: { enabled: false, reason: "未开启", defaults: {}, presets: [] } });
    render(<CosmeticStudioPage />);
    expect(await screen.findByText("本地实验暂不可用")).toBeTruthy();
    expect(screen.getByRole("button", { name: /检查 Demo/ }).disabled).toBe(true);
  });

  test("requires a loaded preview before confirming", async () => {
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    await waitFor(() => expect(mocks.create).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: /确认当前画面并录制/ }).disabled).toBe(true);
    expect(screen.getByLabelText("确认当前预览画面").disabled).toBe(true);
  });

  test("shows readable field validation errors without exposing raw payloads", async () => {
    mocks.create.mockRejectedValueOnce({ response: { data: { detail: [
      { loc: ["body", "tools", "core_exe"], msg: "Network and relative paths are not allowed", input: "private-value" },
    ] } } });
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await screen.findByLabelText("目标玩家与片段");
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    expect(await screen.findByText("SkinCore 程序（文件提供方）：Network and relative paths are not allowed")).toBeTruthy();
    expect(screen.queryByText(/\[object Object\]|private-value/)).toBeNull();
    expect(screen.getByLabelText("确认当前预览画面").disabled).toBe(true);
  });

  test("loads preview and confirms only after explicit action", async () => {
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    mocks.getJob.mockResolvedValueOnce({ data: { id: "job-1", state: "preview", preview_id: "preview-1", preview_url: "/preview.png", message: "请确认" } });
    await waitFor(() => expect(screen.getByAltText("当前 Demo 外观预览")).toBeTruthy(), { timeout: 3000 });
    fireEvent.load(screen.getByAltText("当前 Demo 外观预览"));
    await waitFor(() => expect(screen.getByLabelText("确认当前预览画面").disabled).toBe(false), { timeout: 3000 });
    fireEvent.click(screen.getByLabelText("确认当前预览画面"));
    expect(screen.getByRole("button", { name: /确认当前画面并录制/ }).disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: /确认当前画面并录制/ }));
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalledWith("job-1", "preview-1"));
  });

  test("cancel remains available for a running job", async () => {
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    await waitFor(() => expect(screen.getByRole("button", { name: /取消专用会话/ }).disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: /取消专用会话/ }));
    await waitFor(() => expect(mocks.cancel).toHaveBeenCalledWith("job-1"));
  });

  test("keeps same-XUID candidate intervals distinct", async () => {
    mocks.inspect.mockResolvedValueOnce({ data: { candidates: [
      { xuid: "123", start_tick: 10, end_tick: 20, label: "第一段" },
      { xuid: "123", start_tick: 80, end_tick: 100, label: "第二段" },
    ] } });
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.change(screen.getByLabelText("目标玩家与片段"), { target: { value: "123:80:100" } });
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    await waitFor(() => expect(mocks.create).toHaveBeenCalledWith(expect.objectContaining({ xuid: "123", start_tick: 80, end_tick: 100 })));
  });

  test("uses a complete backend candidate label without duplicating ticks", async () => {
    mocks.inspect.mockResolvedValueOnce({ data: { demo_sha256: "demo-sha", candidates: [{ xuid: "123", start_tick: 10, end_tick: 20, label: "完整后端标签" }, { xuid: "456", start_tick: 30, end_tick: 40 }] } });
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    const select = await screen.findByLabelText("目标玩家与片段");
    await waitFor(() => expect(select.options[0].textContent).toBe("完整后端标签"));
    expect(select.options[1].textContent).toContain("456 · tick 30–40");
  });

  test("preview load failure cannot be confirmed", async () => {
    mocks.preview.mockRejectedValueOnce(new Error("bad image"));
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    mocks.getJob.mockResolvedValueOnce({ data: { id: "job-1", state: "preview", preview_id: "preview-1", preview_url: "/preview.png", message: "请确认" } });
    await waitFor(() => expect(screen.getByText("bad image")).toBeTruthy(), { timeout: 3000 });
    expect(screen.getByLabelText("确认当前预览画面").disabled).toBe(true);
    expect(screen.getByRole("button", { name: /确认当前画面并录制/ }).disabled).toBe(true);
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  test("polling errors keep the job non-terminal", async () => {
    mocks.getJob.mockRejectedValueOnce(new Error("status unavailable"));
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    await waitFor(() => expect(screen.getByText("status unavailable")).toBeTruthy(), { timeout: 3000 });
    expect(screen.getByText("准备中")).toBeTruthy();
    expect(screen.queryByText("完成")).toBeNull();
  });

  test("polling error clears after a successful status read", async () => {
    mocks.getJob.mockRejectedValueOnce(new Error("temporary status failure"));
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    await waitFor(() => expect(screen.getByText("temporary status failure")).toBeTruthy(), { timeout: 3000 });
    await waitFor(() => expect(screen.queryByText("temporary status failure")).toBeNull(), { timeout: 3000 });
  });

  test("active-state failure blocks start until retry succeeds", async () => {
    let rejectActive;
    mocks.active.mockImplementationOnce(() => new Promise((_, reject) => { rejectActive = reject; }));
    render(<CosmeticStudioPage />);
    const start = screen.getByRole("button", { name: /读取状态中|启动预览/ });
    expect(start.disabled).toBe(true);
    rejectActive(new Error("active state unavailable"));
    expect(await screen.findByText("active state unavailable")).toBeTruthy();
    expect(screen.getByRole("button", { name: "启动预览" }).disabled).toBe(true);
    mocks.active.mockResolvedValueOnce({ data: null });
    fireEvent.click(screen.getByRole("button", { name: "重试读取状态" }));
    await waitFor(() => expect(screen.queryByText("active state unavailable")).toBeNull());
  });

  test("a changed preview resets confirmation", async () => {
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    mocks.getJob.mockResolvedValueOnce({ data: { id: "job-1", state: "preview", preview_id: "preview-1", preview_url: "/preview-a.png", message: "请确认" } });
    await waitFor(() => expect(screen.getByAltText("当前 Demo 外观预览")).toBeTruthy(), { timeout: 3000 });
    fireEvent.load(screen.getByAltText("当前 Demo 外观预览"));
    await waitFor(() => expect(screen.getByLabelText("确认当前预览画面").disabled).toBe(false), { timeout: 3000 });
    fireEvent.click(screen.getByLabelText("确认当前预览画面"));
    expect(screen.getByRole("button", { name: /确认当前画面并录制/ }).disabled).toBe(false);
    mocks.getJob.mockResolvedValueOnce({ data: { id: "job-1", state: "preview", preview_id: "preview-2", preview_url: "/preview-b.png", message: "新预览" } });
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 950)); });
    await waitFor(() => expect(screen.getByLabelText("确认当前预览画面").checked).toBe(false));
    expect(screen.getByRole("button", { name: /确认当前画面并录制/ }).disabled).toBe(true);
  });

  test("editing the Demo path clears the previous candidate", async () => {
    render(<CosmeticStudioPage />);
    const path = await screen.findByLabelText("本地 Demo 路径");
    fireEvent.change(path, { target: { value: "D:/first.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.change(path, { target: { value: "D:/second.dem" } });
    expect(screen.queryByLabelText("目标玩家与片段")).toBeNull();
    expect(screen.getByRole("button", { name: /启动预览/ }).disabled).toBe(true);
  });

  test("active job locks inputs while cancellation remains available", async () => {
    render(<CosmeticStudioPage />);
    const path = await screen.findByLabelText("本地 Demo 路径");
    fireEvent.change(path, { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    await waitFor(() => expect(screen.getByRole("button", { name: /取消专用会话/ }).disabled).toBe(false));
    expect(path.disabled).toBe(true);
    expect(screen.getByLabelText("目标玩家与片段").disabled).toBe(true);
    expect(screen.getByLabelText("cs2_exe").disabled).toBe(true);
  });

  test("selects provider-specific preset and sends seed and wear", async () => {
    mocks.getCapabilities.mockResolvedValueOnce({ data: {
      enabled: true,
      providers: [{ id: "nskinz", label: "nSkinz" }, { id: "skin_core", label: "SkinCore" }],
      presets: [
        { id: "awp-dragon-lore", label: "AWP · 巨龙传说", weapon_id: 9, paint_kit: 344, provider_ids: ["nskinz", "skin_core"] },
        { id: "awp-asiimov", label: "AWP · 二西莫夫", weapon_id: 9, paint_kit: 279, provider_ids: ["skin_core"] },
      ],
      defaults: { cs2_exe: "cs2", hlae_exe: "hlae", core_exe: "core", obs_exe: "obs", ffmpeg_exe: "ffmpeg", ffprobe_exe: "ffprobe", output_dir: "out" },
    } });
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("外观提供方"), { target: { value: "skin_core" } });
    expect(screen.queryByLabelText("nskinz_dll")).toBeNull();
    fireEvent.change(screen.getByLabelText("皮肤预设"), { target: { value: "awp-asiimov" } });
    fireEvent.change(screen.getByLabelText("图案种子"), { target: { value: "321" } });
    fireEvent.change(screen.getByLabelText("磨损"), { target: { value: "0.123" } });
    const path = screen.getByLabelText("本地 Demo 路径");
    fireEvent.change(path, { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    await waitFor(() => expect(mocks.create).toHaveBeenCalledWith(expect.objectContaining({ provider_id: "skin_core", preset_id: "awp-asiimov", pattern_seed: 321, wear: 0.123, tools: expect.not.objectContaining({ nskinz_dll: expect.anything() }) })));
  });

  test("blocks invalid core seed or wear before start", async () => {
    mocks.getCapabilities.mockResolvedValueOnce({ data: { enabled: true, providers: [{ id: "skin_core", label: "SkinCore" }], presets: [{ id: "awp-asiimov", label: "AWP · 二西莫夫", weapon_id: 9, paint_kit: 279, provider_ids: ["skin_core"] }], defaults: { cs2_exe: "cs2", hlae_exe: "hlae", core_exe: "core", obs_exe: "obs", ffmpeg_exe: "ffmpeg", ffprobe_exe: "ffprobe", output_dir: "out" } } });
    render(<CosmeticStudioPage />);
    fireEvent.change(await screen.findByLabelText("外观提供方"), { target: { value: "skin_core" } });
    fireEvent.change(screen.getByLabelText("图案种子"), { target: { value: "1001" } });
    fireEvent.change(screen.getByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    expect(screen.getByRole("button", { name: /启动预览/ }).disabled).toBe(true);
    expect(mocks.create).not.toHaveBeenCalled();
  });

  test("opens LiteCut through SPA routing after a completed job", async () => {
    mocks.create.mockResolvedValueOnce({ data: { id: "job-complete", state: "complete", message: "完成", litecut_project_id: 7, output_path: "D:/out.mkv" } });
    render(<MemoryRouter initialEntries={["/cs2/cosmetic-studio"]}><><CosmeticStudioPage /><LocationProbe /></></MemoryRouter>);
    fireEvent.change(await screen.findByLabelText("本地 Demo 路径"), { target: { value: "D:/demo.dem" } });
    fireEvent.click(screen.getByRole("button", { name: /检查 Demo/ }));
    await waitFor(() => expect(screen.getByLabelText("目标玩家与片段")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: /启动预览/ }));
    const link = await screen.findByRole("link", { name: "打开 LiteCut" });
    fireEvent.click(link);
    expect(screen.getByTestId("location-probe").textContent).toBe("/cs2/lite-cut");
  });

  test("cleans polling and preview blob on unmount", async () => {
    vi.useFakeTimers();
    const { unmount } = render(<CosmeticStudioPage />);
    await act(async () => {});
    unmount();
    await act(async () => { vi.advanceTimersByTime(2000); });
    expect(mocks.getJob).not.toHaveBeenCalled();
  });
});
