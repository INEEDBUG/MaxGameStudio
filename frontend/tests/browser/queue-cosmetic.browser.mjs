import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const {chromium}=await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const browser=await chromium.launch({headless:true,executablePath:process.env.SCROLL_TEST_CHROME});
const xuid='76561199369875913';
const defs=[1,2,3,4,7,8,9,10,11,13,14,16,17,19,23,24,25,26,27,28,29,30,32,33,34,35,36,38,39,40,60,61,63,64];
const presets=defs.flatMap(def=>Array.from({length:65},(_,i)=>({id:`skin-${def}-${i}`,label:`Gun ${def} · Finish ${i}`,source_definition:def,target_definition:def,paint_index:i+1,wear_min:0,wear_max:1})));
const items=defs.map(def=>({slot_id:`owned:${def}`,xuid,team:'both',definition:def,item_id:String(def+1000),name:`Gun ${def}`,editable:true,preset_ids:presets.filter(p=>p.source_definition===def).map(p=>p.id)}));
items.find(item=>item.definition===61).paint_index=0;
items.push({slot_id:'blocked:knife',xuid,definition:42,team:'both',name:'未确认的刀',editable:false,reason:'missing_item_identity',preset_ids:[]});
items.push({...items.find(item=>item.definition===9),slot_id:'foreign:awp',editable:false,reason:'unverified_original_ownership',preset_ids:[]});
const inventory={demo_sha256:'a'.repeat(64),players:[{xuid,name:'Alice'}],items,presets,plans:{},settings:{}};
let reads=0,writes=0;
const root=process.env.QUEUE_TEST_URL||'http://127.0.0.1:18747';
async function bounded(button){await button.scrollIntoViewIfNeeded();assert.ok(await button.evaluate(el=>{const r=el.getBoundingClientRect();return r.width>0&&r.height>0&&r.top>=0&&r.bottom<=innerHeight&&r.right<=innerWidth;}),'Control clipped outside viewport');}
try{
 for(const [width,height] of [[1440,900],[1100,700]])for(const theme of ['dark','light']){
  const context=await browser.newContext({viewport:{width,height}}),page=await context.newPage();let errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.route('**/api/cosmetic-studio/**',route=>{if(new URL(route.request().url()).pathname.endsWith('/inventory')){reads++;return route.fulfill({json:inventory});}writes++;return route.fulfill({status:400,json:{detail:'No native action allowed'}});});
  await page.goto(root+'/tests/browser/queue-cosmetic.html');await page.evaluate(theme=>{document.documentElement.className=theme;},theme);
  const open=page.getByRole('button',{name:'选择武器皮肤'});await open.waitFor();const before=reads;
  assert.equal(await page.locator('.queue-buy-menu').count(),0,'Picker eagerly mounted');
  await open.click();await page.getByRole('button',{name:'更换 Gun 40',exact:true}).waitFor();assert.equal(reads,before+1);
  assert.equal(await page.getByRole('combobox').count(),0,'Queue target must be locked');
  const stock=page.locator('[data-weapon-definition="61"] [data-artwork-kind="stock"] img');
  await stock.waitFor();await stock.evaluate(img=>{
   if(img.complete){if(!img.naturalWidth)throw Error('Stock artwork failed to load');return;}
   return new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Stock artwork load timeout')),10000);img.addEventListener('load',()=>{clearTimeout(timer);resolve();},{once:true});img.addEventListener('error',()=>{clearTimeout(timer);reject(Error('Stock artwork failed to load'));},{once:true});});
  });
  assert.equal(await page.locator('.queue-buy-columns svg').count(),0,'Generic silhouettes remain in weapon menu');
  await page.getByRole('button',{name:'查看 未确认的刀'}).click();
  assert.equal(await page.getByRole('button',{name:'装备',exact:true}).count(),0,'Unqualified knife was editable');
  await page.getByRole('button',{name:'T',exact:true}).click();await page.getByRole('button',{name:'CT',exact:true}).click();
  const weapons=page.locator('[data-weapon-definition]');assert.equal(await weapons.count(),34,'Incomplete firearm menu');
  assert.equal(await page.locator('[data-weapon-definition="9"]').count(),1,'Duplicate AWP');
  const clipped=await weapons.evaluateAll(nodes=>nodes.flatMap(el=>{const r=el.getBoundingClientRect();const hit=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);return r.top>=0&&r.bottom<=innerHeight&&r.width>=32&&r.height>=44&&(hit===el||el.contains(hit))?[]:[{name:el.getAttribute('aria-label'),top:r.top,bottom:r.bottom,height:r.height,hit:hit?.className}];}));
  if(clipped.length&&process.env.QUEUE_SCREENSHOT_DIR)await page.screenshot({path:process.env.QUEUE_SCREENSHOT_DIR+'/queue-menu-clipped.png'});
  assert.deepEqual(clipped,[],'A weapon is clipped or requires scrolling');
  const scroll=page.locator('.queue-loadout-layout');assert.ok(await scroll.evaluate(el=>el.scrollHeight<=el.clientHeight+1),'Weapon area overflows vertically');
  await page.getByRole('button',{name:'更换 Gun 40',exact:true}).click();
  const picker=page.locator('.queue-buy-preview .cosmetic-picker-body');await picker.hover();await page.mouse.wheel(0,1000);await page.waitForTimeout(200);
  assert.ok(await picker.evaluate(el=>el.scrollTop)>0,'Inline skin list cannot scroll');
  const equip=page.getByRole('button',{name:'装备',exact:true});await bounded(equip);
  assert.ok(await equip.evaluate(el=>{const r=el.getBoundingClientRect();const hit=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);return hit===el||el.contains(hit);}), 'Save bar obstructs Equip');
  const search=page.getByPlaceholder('输入饰品名称');await search.fill('Finish 60');
  await page.getByRole('button',{name:'选择 Gun 40 · Finish 60',exact:true}).click();await equip.click();
  await page.getByRole('checkbox').check();const save=page.getByRole('button',{name:'保存录制外观'});await bounded(save);
  if(process.env.QUEUE_SCREENSHOT_DIR) await page.screenshot({path:process.env.QUEUE_SCREENSHOT_DIR+`/queue-menu-${width}-${theme}.png`});
  await save.click();await page.getByRole('dialog').waitFor({state:'detached'});
  let rows=await page.evaluate(()=>window.queueTest.get());assert.equal(rows[0].appearance.replacements[0].preset_id,'skin-40-60');assert.deepEqual(rows[1].appearance,rows[0].appearance);assert.equal(rows[2].appearance,undefined);
  await open.click();await page.getByRole('button',{name:'更换 Gun 40',exact:true}).click();
  await page.getByRole('button',{name:'装备',exact:true}).click();await page.getByRole('button',{name:'取消皮肤编辑'}).click();
  rows=await page.evaluate(()=>window.queueTest.get());assert.equal(rows[0].appearance.replacements[0].preset_id,'skin-40-60','Cancel changed saved profile');
  assert.ok(await open.evaluate(el=>el===document.activeElement),'Opener focus not restored');
  await page.getByRole('button',{name:'恢复原皮肤',exact:true}).click();rows=await page.evaluate(()=>window.queueTest.get());assert.equal(rows[0].appearance,null);assert.ok(rows[1].appearance);
  await page.getByRole('button',{name:'片段 other · Bob'}).click();await open.click();await page.getByRole('alert').waitFor();assert.equal(await page.getByRole('button',{name:'保存录制外观'}).count(),0);
  await page.keyboard.press('Escape');assert.equal(await page.getByRole('dialog').count(),0);assert.deepEqual(errors,[]);
  console.log(`PASS ${width}x${height}/${theme}: all34 firearms visible,unique models,CT/T,blocked knife read-only,skin list scroll,equip,save,reopen,cancel,share,restore,unknown target`);await context.close();
 }
 assert.equal(writes,0,'Editing invoked native/file APIs');
}finally{await browser.close();}
