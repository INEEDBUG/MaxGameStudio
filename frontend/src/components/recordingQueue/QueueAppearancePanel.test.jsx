import {fireEvent, render, screen, waitFor, cleanup} from '@testing-library/react';
import {afterEach, beforeEach, expect, test, vi} from 'vitest';
import QueueAppearancePanel from './QueueAppearancePanel';
import {useRecordingQueue} from '../../stores/recordingQueueStore';
const api=vi.hoisted(()=>({inventory:vi.fn(),apply:vi.fn(),play:vi.fn()}));
vi.mock('../../api/cosmeticStudio.js',()=>({getAppearanceInventory:api.inventory,applyAppearancePlan:api.apply,playAppearanceDemo:api.play,
 saveAppearanceSettings:vi.fn(),cancelAppearanceJob:vi.fn(),getAppearanceJob:vi.fn()}));
const xuid='76561199369875913';
const item={id:'one',demoPath:'D:/source.dem',targetSteamId:xuid};
const data={demo_sha256:'a'.repeat(64),players:[{xuid,name:'Alice'}],plans:{},settings:{},
 items:[{slot_id:'owned:usp',definition:61,xuid,team:'ct',name:'USP-S',editable:true,preset_ids:['usp']}],
 presets:[{id:'usp',source_definition:61,target_definition:61,label:'USP-S · 消音',paint_index:504,wear_min:0,wear_max:1}]};
beforeEach(()=>{vi.clearAllMocks();useRecordingQueue.setState({queue:[item,{...item,id:'two'},{...item,id:'other',targetSteamId:'456'}]});api.inventory.mockResolvedValue({data});});
afterEach(cleanup);
test('idle mounts no inventory; queue menu saves and shares only on explicit choice',async()=>{
 render(<QueueAppearancePanel item={item}/>);
 expect(api.inventory).not.toHaveBeenCalled();
 fireEvent.click(screen.getByRole('button',{name:'选择武器皮肤'}));
 await screen.findByRole('button',{name:'更换 USP-S'});
 expect(api.inventory).toHaveBeenCalledWith(item.demoPath,xuid);
 fireEvent.click(screen.getByRole('checkbox'));
 fireEvent.click(screen.getByRole('button',{name:'更换 USP-S'}));
 fireEvent.click(screen.getByRole('button',{name:'装备',exact:true}));
 fireEvent.click(screen.getByRole('button',{name:'保存录制外观'}));
 await waitFor(()=>expect(screen.queryByRole('dialog')).toBeNull());
 const rows=useRecordingQueue.getState().queue;
 expect(rows[0].appearance.replacements[0].preset_id).toBe('usp');
 expect(rows[1].appearance).toEqual(rows[0].appearance);expect(rows[2].appearance).toBeUndefined();
 expect(api.apply).not.toHaveBeenCalled();expect(api.play).not.toHaveBeenCalled();
});
test('cancel discards uncommitted changes and restores opener focus',async()=>{
 render(<QueueAppearancePanel item={item}/>);
 const opener=screen.getByRole('button',{name:'选择武器皮肤'});opener.focus();fireEvent.click(opener);
 await screen.findByRole('button',{name:'更换 USP-S'});
 fireEvent.click(screen.getByRole('button',{name:'更换 USP-S'}));
 fireEvent.click(screen.getByRole('button',{name:'装备',exact:true}));
 fireEvent.keyDown(screen.getByRole('dialog'),{key:'Escape'});
 expect(useRecordingQueue.getState().queue[0].appearance).toBeUndefined();
 expect(document.activeElement).toBe(opener);
});
test('unknown target blocks edits rather than using the first roster player',async()=>{
 render(<QueueAppearancePanel item={{...item,targetSteamId:'999'}}/>);
 fireEvent.click(screen.getByRole('button',{name:'选择武器皮肤'}));
 expect(await screen.findByRole('alert')).toHaveProperty('textContent',expect.stringContaining('未在 Demo 中确认'));
 expect(screen.queryByRole('button',{name:'保存录制外观'})).toBeNull();
});
test('missing target and running recording disable opening',()=>{
 const {rerender}=render(<QueueAppearancePanel item={{...item,targetSteamId:null}}/>);
 expect(screen.getByRole('button',{name:'选择武器皮肤'}).disabled).toBe(true);
 rerender(<QueueAppearancePanel item={item} disabled/>);
 expect(screen.getByRole('button',{name:'选择武器皮肤'}).disabled).toBe(true);
});
