const assert=require('node:assert/strict');
const fs=require('node:fs');const vm=require('node:vm');const path=require('node:path');
const source=()=>fs.readFileSync(path.join(__dirname,'../app/cosmetic_studio/sky_runtime.js'),'utf8');
function run(overrides={}) {
 const state={playing:true,paused:false,tick:2048,demo:'F:/game/playback.dem'};
 const commands=[],records=[];let frame;let now=100000;
 const config={root:'D:/session',demo:state.demo,tag:'MGS_SKY_TEST',commands:['mirv_sky material "materials/skybox/sky_de_dust2.vmat_c"'],...overrides};
 const context={Date:{now:()=>now},JSON,String,Number,Error,Math,
  AdvancedfxPath:class {constructor(p){this.p=p;} isFile(){return this.p.endsWith('lease-0.flag');}},
  mirv:{isPlayingDemo:()=>state.playing,getDemoFilePath:()=>state.demo,resolveGamePath:p=>p,
   getDemoTick:()=>state.tick,isDemoPaused:()=>state.paused,
   message:s=>records.push(JSON.parse(s.slice(s.indexOf(' ')+1))),exec:s=>commands.push(s),
   events:{clientFrameStageNotify:{on:(id,fn)=>{frame=fn;}}}}};
 vm.runInNewContext(source().replace('__SKY_CONFIG__',JSON.stringify(config)),context);
 return {state,commands,records,frame:()=>{now+=300;frame({isBefore:false,curStage:12});},expire:()=>{now+=9000;}};
}
const h=run();h.frame();assert(h.commands.some(s=>s.startsWith('mirv_sky material')));
assert(!h.commands.some(s=>s.includes('mirv_loadlibrary')));
h.state.tick=512;h.frame();assert.equal(h.commands.filter(s=>s.startsWith('mirv_sky material')).length,2);
h.state.demo='F:/game/other.dem';h.frame();assert(h.commands.includes('quit'));
assert.equal(h.records.at(-1).reason,'UNKNOWN_DEMO');
const bad=run({commands:['connect server']});bad.frame();assert(!bad.commands.includes('connect server'));assert(bad.commands.includes('quit'));
const lease=run();lease.frame();lease.expire();lease.frame();assert(lease.commands.includes('quit'));
assert(lease.records.every(r=>r.visual_verified!==true));
console.log('Sky runtime: finite commands, Demo identity, seek reapply, expired lease: PASS');
