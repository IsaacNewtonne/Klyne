const fs=require('fs'),assert=require('assert');
const {launch}=require('./test-browser.cjs');
(async()=>{
const b=await launch();
const p=await b.newPage({viewport:{width:1920,height:1080}}),errors=[];
p.on('pageerror',e=>errors.push(e.message));
await p.route('http://127.0.0.1:4317/**',r=>{
const pathname=new URL(r.request().url()).pathname;
if(pathname.startsWith('/api/'))return r.fulfill({json:pathname==='/api/chats'?{chats:[]}:{}});
const file='apps/studio/web/'+(pathname==='/'?'chat.html':pathname.slice(1));
return r.fulfill({path:file,contentType:file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});
});
await p.goto('http://127.0.0.1:4317');
await p.waitForTimeout(300);
const frames=()=>p.evaluate(()=>['.ascii-fire','#ambient-embers'].map(s=>document.querySelector(s).toDataURL()));
const first=await frames();await p.waitForTimeout(300);const second=await frames();
assert(first.every((v,i)=>v!==second[i]),'Both idle canvases animate');
await p.evaluate(()=>window.dispatchEvent(new PointerEvent('pointermove',{pointerType:'touch',clientX:950,clientY:200})));
await p.waitForTimeout(200);
await p.screenshot({path:'workspace/restored-embers.png'});
await p.emulateMedia({reducedMotion:'reduce'});await p.waitForTimeout(1200);
const still=await frames();await p.waitForTimeout(250);assert((await frames()).every((v,i)=>v===still[i]),'Reduced motion canvases stay still');
assert.deepEqual(errors,[]);
await p.emulateMedia({reducedMotion:'no-preference'});
await p.evaluate(()=>document.querySelector('#welcome').hidden=true);await p.waitForTimeout(150);
assert.equal(await p.locator('#ambient-embers').isVisible(),false);
await p.evaluate(()=>document.querySelector('#welcome').hidden=false);await p.waitForTimeout(150);
const resumed=await frames();await p.waitForTimeout(300);assert((await frames()).every((v,i)=>v!==resumed[i]));
await b.close();console.log('PASS: idle flame and embers animate; touch accepted; reduced motion pauses; no page errors');
})().catch(e=>{console.error(e);process.exit(1)});



