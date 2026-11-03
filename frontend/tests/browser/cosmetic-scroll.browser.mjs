// Run against a local Vite server. Uses installed Playwright/Chrome only.
// PLAYWRIGHT_MODULE points to playwright/index.mjs if not a project dependency.
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
const {chromium}=await import(process.env.PLAYWRIGHT_MODULE?pathToFileURL(process.env.PLAYWRIGHT_MODULE).href:'playwright');
const browser=await chromium.launch({headless:true,executablePath:process.env.SCROLL_TEST_CHROME});
const xuid='76561199369875913';
const presets=Array.from({length:80},(_,i)=>({id:'test-awp-'+i,label:'AWP · layout fixture '+i,
    source_definition:9,target_definition:9,paint_index:100+i,wear_min:0,wear_max:1}));
const items=[1,2,3,4,7,8,9,10,11,13,14,16,17,19,23,24,25,26,27,28,29,30,32,33,34,35,36,38,39,40,60,61,63,64].map((definition,i)=>({
    slot_id:'test-slot-'+i,xuid,team:i%2?'t':'ct',definition,item_id:String(10000+i),name:'Gun '+definition,
    paint_index:0,wear:.01,seed:0,editable:true,preset_ids:definition===9?presets.map(p=>p.id):['test-'+definition]}));
for(const item of items.filter(row=>row.definition!==9))presets.push({id:'test-'+item.definition,label:item.name,
    source_definition:item.definition,target_definition:item.definition,paint_index:100,wear_min:0,wear_max:1});
const inventory={demo_sha256:'a'.repeat(64),players:[{xuid,name:'Alice'}],items,presets,
    settings:{core_exe:'D:/fixture-core.exe',output_dir:'D:/output'},plans:{}};
let writes=0;
try{
 for(const [width,height] of [[1440,900],[1100,700]])for(const theme of ['dark','light']){
  const context=await browser.newContext({viewport:{width,height}}),page=await context.newPage();
  page.on('pageerror',error=>console.error('PAGE_ERROR',error.message));
  await page.route('**/api/cosmetic-studio/**',async route=>{
   const request=route.request(),pathname=new URL(request.url()).pathname;
   if(pathname.endsWith('/inventory'))return route.fulfill({json:inventory});
   if(pathname.endsWith('/sky/options'))return route.fulfill({json:{hlae_exe:'D:/HLAE.exe',presets:['original','italy','dust2','night','overcast','sunset'].map(id=>({id,label:id,available:true}))}});
   if(pathname.endsWith('/sky/status'))return route.fulfill({json:{state:'idle'}});
   writes++;return route.fulfill({status:400,json:{detail:'No write/game action permitted in scroll regression'}});
  });
  await page.goto((process.env.SCROLL_TEST_URL||'http://127.0.0.1:18746')+'/tests/browser/cosmetic-scroll.html',{waitUntil:'domcontentloaded',timeout:60000});
  await page.locator('.cosmetic-slot').first().waitFor({timeout:15000}).catch(async error=>{
    console.error('PAGE_STATE',await page.locator('body').innerText());throw error;
  });
  await page.evaluate(theme=>{document.documentElement.className=theme;},theme);
  const workspace=page.locator('main').filter({has:page.getByRole('heading',{name:'Demo 饰品',exact:true})}).last();
  const before=await workspace.evaluate(el=>({height:el.clientHeight,content:el.scrollHeight,top:el.scrollTop,overflow:getComputedStyle(el).overflowY}));
  await page.mouse.move(width-70,250);await page.mouse.wheel(0,600);
  await page.waitForTimeout(250);
  assert.ok(await workspace.evaluate(el=>el.scrollTop)>0,`Mouse wheel cannot scroll ${width}x${height}/${theme}: ${JSON.stringify(before)}`);
  assert.ok(before.height<=height-44,'Page exceeds bounded shell viewport');
  await workspace.focus();const top=await workspace.evaluate(el=>el.scrollTop);
  await page.keyboard.press('PageDown');await page.waitForTimeout(200);
  assert.ok(await workspace.evaluate(el=>el.scrollTop)>top,'Keyboard PageDown cannot scroll');
  await page.mouse.wheel(0,20000);await page.waitForTimeout(300);
  const bottom=await workspace.evaluate(el=>({top:el.scrollTop,max:el.scrollHeight-el.clientHeight}));
  assert.ok(Math.abs(bottom.top-bottom.max)<3,'Cannot reach page bottom');
  await page.getByRole('heading',{name:'天空',exact:true}).scrollIntoViewIfNeeded();
  await page.getByText('高级设置',{exact:true}).scrollIntoViewIfNeeded();
  const advanced=page.getByText('高级设置',{exact:true});await advanced.click();
  await page.getByLabel('应用范围').scrollIntoViewIfNeeded();
  assert.ok(await page.getByLabel('应用范围').evaluate(el=>{
    const r=el.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight;
  }),'Advanced control is clipped outside viewport');
  await workspace.evaluate(el=>{el.scrollTop=0;});
  await page.getByRole('button',{name:'更换 Gun 9',exact:true}).click();
  const body=page.locator('.cosmetic-picker-body');const initial=await body.evaluate(el=>el.scrollTop);
  await body.hover();await page.mouse.wheel(0,1600);await page.waitForTimeout(200);
  assert.ok(await body.evaluate(el=>el.scrollTop)>initial,'Skin picker cannot scroll');
  const footer=page.getByRole('dialog').getByRole('button',{name:'装备',exact:true});
  assert.ok(await footer.evaluate(el=>{
    const r=el.getBoundingClientRect();return r.width>0&&r.top>=0&&r.bottom<=innerHeight;
  }),'Picker Equip footer inaccessible');
  await page.getByRole('button',{name:'关闭外观选择'}).click();
  assert.equal(await workspace.evaluate(el=>el.scrollTop),0,'Modal wheel leaked into page');
  console.log(`PASS ${width}x${height}/${theme}: wheel,PageDown,bottom,sky,advanced,picker`);
  await context.close();
 }
 assert.equal(writes,0,'Scroll test issued a write/game action');
}finally{await browser.close();}
