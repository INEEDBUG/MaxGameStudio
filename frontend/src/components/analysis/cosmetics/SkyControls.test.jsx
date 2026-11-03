import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {beforeEach,expect,test,vi} from 'vitest';
import SkyControls from './SkyControls.jsx';
const mocks=vi.hoisted(()=>({options:vi.fn(),status:vi.fn(),start:vi.fn(),stop:vi.fn()}));
vi.mock('../../../api/cosmeticStudio.js',()=>({getSkyOptions:mocks.options,getSkyStatus:mocks.status,startSkyPreview:mocks.start,stopSkyPreview:mocks.stop}));
beforeEach(()=>{
 vi.clearAllMocks();mocks.options.mockResolvedValue({data:{hlae_exe:'D:/HLAE.exe',presets:[{id:'original',label:'原始天空',available:true},{id:'italy',label:'晴朗天空',available:true},{id:'night',label:'雨夜',available:false}]}});
 mocks.status.mockResolvedValue({data:{state:'idle'}});mocks.start.mockResolvedValue({data:{state:'starting',message:'正在启动'}});
 mocks.stop.mockResolvedValue({data:{state:'stopping',message:'正在关闭'}});
});
test('sky selection requires a verified copy and explicit offline acknowledgement',async()=>{
 const {rerender}=render(<SkyControls jobId={null} disabled={false}/>);
 await screen.findByRole('button',{name:/晴朗天空/});
 expect(screen.getByRole('button',{name:'预览所选天空'}).disabled).toBe(true);
 expect(screen.getByRole('button',{name:/雨夜/}).disabled).toBe(true);
 expect(mocks.start).not.toHaveBeenCalled();
 rerender(<SkyControls jobId={'a'.repeat(32)} disabled={false}/>);
 fireEvent.click(screen.getByRole('button',{name:/晴朗天空/}));
 fireEvent.click(screen.getByRole('checkbox'));
 fireEvent.click(screen.getByRole('button',{name:'预览所选天空'}));
 await waitFor(()=>expect(mocks.start).toHaveBeenCalledWith({job_id:'a'.repeat(32),sky_id:'italy',offline_acknowledged:true,hlae_exe:'D:/HLAE.exe'}));
 expect(await screen.findByRole('button',{name:'关闭天空预览'})).toBeTruthy();
 fireEvent.click(screen.getByRole('button',{name:'关闭天空预览'}));
 await waitFor(()=>expect(mocks.stop).toHaveBeenCalledOnce());
});
test('dirty skins prevent sky preview; original-copy helper does not launch a game',async()=>{
 const copy=vi.fn();render(<SkyControls jobId={null} disabled={false} onCreateCopy={copy}/>);
 await screen.findByRole('button',{name:/原始天空/});
 fireEvent.click(screen.getByRole('button',{name:'只换天空 · 生成 Demo 副本'}));
 expect(copy).toHaveBeenCalledOnce();expect(mocks.start).not.toHaveBeenCalled();
});
