const fs=require('fs'),path=require('path'),assert=require('assert');
const {launch}=require('./test-browser.cjs');
(async()=>{
 const browser=await launch();
 let appConnections=[];
 const page=await browser.newPage({viewport:{width:1680,height:1050}}),errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.route('http://127.0.0.1:4317/**',route=>{
  const url=new URL(route.request().url()),file=url.pathname==='/'?'chat.html':url.pathname.slice(1);
  const local=path.resolve('apps/studio/web',file);
  if(!url.pathname.startsWith('/api/') && fs.existsSync(local))return route.fulfill({path:local,contentType:file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});
  return route.fulfill({json:url.pathname==='/api/apps'?{connections:appConnections}:url.pathname==='/api/chats'?{chats:[]}:url.pathname==='/api/runtime/load'?{cpu_percent:12.5}:{}});
 });
 await page.goto('http://127.0.0.1:4317/');
 await page.waitForFunction(()=>window.productionView&&window.klyneActivity);
 await page.evaluate(()=>{
  window.fixture={id:'1',status:'Working',title:'Activity validation',messages:[],tasks:[{agent:'Worker',status:'Working',instruction:'Read inputs'}],evidence:[],pending:null,activity:{kind:'model',role:'planner'},activity_events:[],access:{web:true,terminal:true,desktop:true,apps:true}};
  polling=true;window.show=()=>{selected='1';snapshot=fixture;render();};show();
 });
 assert.equal(await page.locator('#prod-details').count(),0);
 assert.equal(await page.locator('#production').getAttribute('data-mode'),'waiting');
 assert.equal(await page.locator('#production').getAttribute('data-view'),'production');
 await page.evaluate(()=>{
  fixture.activity=null;fixture.activity_events=[{seq:1,type:'tool:start',data:{id:'b',tool:'browser',operation:'Reading webpage'}},{seq:2,type:'tool:start',data:{id:'f',tool:'files',operation:'Reading file'}}];show();
 });
 assert.equal(await page.locator('#prod-health-tools').textContent(),'2');
 assert.equal(await page.locator('#prod-cap-browser').getAttribute('data-state'),'active');
 assert.equal(await page.locator('#prod-cap-terminal').getAttribute('data-state'),'ready');
 await page.waitForFunction(()=>document.querySelectorAll('.energy-packet.outbound').length===2);
 assert.equal(await page.locator('.energy-packet.outbound').count(),2);
 await page.waitForTimeout(400);
 assert(await page.locator('#prod-reactor').evaluate(e=>e.width>100));
 await page.screenshot({path:'workspace/activity-parallel.png',fullPage:true});
 await page.evaluate(()=>{fixture.activity_events.push({seq:3,type:'tool:complete',data:{id:'b',tool:'browser'}});show();});
 assert.equal(await page.locator('#prod-health-tools').textContent(),'1');
 assert.equal(await page.locator('#prod-cap-browser').getAttribute('data-state'),'complete');
 await page.waitForTimeout(950);
 assert.equal(await page.locator('#prod-cap-browser').getAttribute('data-state'),'ready');
 assert.equal(await page.locator('#prod-cap-files').getAttribute('data-state'),'active');
 await page.evaluate(()=>{fixture.activity_events.push({seq:4,type:'tool:waiting',data:{id:'f',operation:'Awaiting file service'}});show();});
 assert.equal(await page.locator('#production').getAttribute('data-mode'),'waiting');
 assert.equal(await page.locator('#prod-cap-files').getAttribute('data-state'),'waiting');
 await page.evaluate(()=>{fixture.activity_events.push({seq:5,type:'tool:error',data:{id:'f',tool:'files'}});fixture.status='Blocked';show();});
 await page.waitForTimeout(950);
 assert.equal(await page.locator('#prod-cap-files').getAttribute('data-state'),'failed');
 assert.equal(await page.locator('#production').getAttribute('data-mode'),'error');
 await page.evaluate(()=>{fixture.provider={kind:'ollama',model:'hf.co/empero-ai/Qwythos-9B-Claude-Mythos-5-1M-GGUF:Q5_K_M'};fixture.messages=[{agent:'Klyne',text:'Desktop result was not confirmed as achieved.'}];fixture.execution={failure:{guidance:'Inspect the recorded error and evidence.'}};show();});
 assert(await page.locator('header .prod-health').isVisible());
 assert.equal(await page.locator('#prod-result').isVisible(),false);
 await page.locator('#prod-notice').click();
 assert((await page.locator('#prod-inspector-body').textContent()).includes('Inspect the recorded error'));
 await page.keyboard.press('Escape');
 for(const viewport of [{width:1920,height:1080},{width:1280,height:720},{width:390,height:844}]){
  await page.setViewportSize(viewport);await page.waitForTimeout(550);
  const layout=await page.evaluate(()=>{
   const core=document.querySelector('#prod-core').getBoundingClientRect(),graph=document.querySelector('#prod-graph').getBoundingClientRect(),main=document.querySelector('#main');
   return {center:Math.abs(core.x+core.width/2-graph.x-graph.width/2),fits:main.scrollHeight<=main.clientHeight+2,model:document.querySelector('#prod-core #prod-model').textContent};
  });
  assert(layout.center<2,JSON.stringify(layout));if(viewport.width>1450)assert(layout.fits,JSON.stringify(layout));
  assert(layout.model.includes('Qwythos'));
 }
 await page.setViewportSize({width:1920,height:1080});await page.waitForTimeout(200);
 await page.screenshot({path:'workspace/activity-compact-blocked.png',fullPage:true});
 await page.locator('#prod-core').click();assert(await page.locator('#prod-inspector').isVisible());await page.keyboard.press('Escape');
 await page.evaluate(()=>{fixture.status='Needs input';show();});
 assert.equal(await page.locator('#production').getAttribute('data-mode'),'input');
 await page.setViewportSize({width:390,height:844});await page.waitForTimeout(550);
 assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
 await page.screenshot({path:'workspace/activity-mobile.png',fullPage:true});
 await page.emulateMedia({reducedMotion:'reduce'});
 assert.equal(await page.locator('#prod-cap-files').evaluate(e=>getComputedStyle(e).animationName),'none');
 await page.emulateMedia({reducedMotion:'no-preference'});
 await page.setViewportSize({width:1680,height:1050});
 const pixels=()=>page.locator('#prod-reactor').evaluate(c=>c.toDataURL());
 await page.evaluate(()=>{fixture.status='Working';fixture.tasks=[];fixture.pending=null;fixture.activity={kind:'model',role:'reviewer'};show();});
 await page.waitForTimeout(150);
 assert.equal(await page.locator('#production').getAttribute('data-mode'),'waiting');
 assert((await page.locator('#prod-core-state').textContent()).includes('Reviewer'));
 assert(await page.locator('#prod-progress').evaluate(p=>!p.hasAttribute('value')));
 const waitFrame=await pixels();await page.waitForTimeout(200);assert.notEqual(await pixels(),waitFrame);
 assert.equal(await page.locator('#prod-cap-terminal > span').evaluate(e=>getComputedStyle(e).animationName),'none');
 await page.evaluate(()=>{fixture.activity=null;show();});
 assert.equal(await page.locator('#production').getAttribute('data-mode'),'working');
 const workFrame=await pixels();await page.waitForTimeout(200);assert.notEqual(await pixels(),workFrame);
 await page.emulateMedia({reducedMotion:'reduce'});await page.waitForTimeout(150);
 const reducedFrame=await pixels();await page.waitForTimeout(200);assert.equal(await pixels(),reducedFrame);
 await page.emulateMedia({reducedMotion:'no-preference'});
 await page.evaluate(()=>{fixture.status='Completed';show();});
 await page.waitForTimeout(1000);
 const settled=await pixels();await page.waitForTimeout(200);assert.notEqual(await pixels(),settled,'Completed sun retains its minimum living flame');
 assert.equal(await page.locator('#prod-progress').evaluate(p=>p.value/p.max),1);
 await page.evaluate(()=>{fixture.status='Needs input';show();});
 assert((await page.locator('#instruction').getAttribute('placeholder')).includes('needs your input'));
 assert.equal(await page.locator('#ambient-embers').isVisible(),false);
 assert.equal(await page.locator('.energy-packet').count(),0);
 await page.emulateMedia({reducedMotion:'no-preference'});
 appConnections=[{name:'notes',base_url:'https://notes.example'},{name:'calendar',base_url:'https://calendar.example'}];
 await page.evaluate(list=>window.dispatchEvent(new CustomEvent('klyne-connections',{detail:list})),appConnections);
 await page.waitForTimeout(600);
 assert.equal(await page.locator('.prod-capability[data-connected-app=true]').count(),2);
 const icons=await page.locator('.prod-capability[data-connected-app=true] svg').evaluateAll(nodes=>nodes.map(n=>n.outerHTML));
 assert.notEqual(icons[0],icons[1]);
 await page.evaluate(()=>{
   fixture.status='Working';fixture.pending=null;fixture.activity=null;
   fixture.activity_events.push({seq:fixture.activity_events.at(-1).seq+1,type:'tool:start',data:{id:'notes-call',action:{tool:'app_invoke',name:'notes',operation:'GET /notes'}}});show();
 });
 assert.equal(await page.locator('[id="prod-cap-app:notes"]').getAttribute('data-state'),'active');
 assert.equal(await page.locator('[id="prod-cap-app:calendar"]').getAttribute('data-state'),'ready');
 assert.equal(await page.locator('#prod-cap-apps').getAttribute('data-state'),'ready');
 await page.locator('[id="prod-cap-app:notes"]').focus();
 await page.setViewportSize({width:1300,height:820});await page.waitForTimeout(60);
 assert(await page.locator('[id="prod-cap-app:notes"]').evaluate(n=>n.getAnimations().length>0),'Resize animates retained card');
 await page.waitForTimeout(600);
 assert.equal(await page.evaluate(()=>document.activeElement.id),'prod-cap-app:notes');
 appConnections=[appConnections[0]];
 await page.evaluate(list=>window.dispatchEvent(new CustomEvent('klyne-connections',{detail:list})),appConnections);
 await page.waitForTimeout(600);
 assert.equal(await page.locator('[id="prod-cap-app:calendar"]').count(),0);
 assert.equal(await page.locator('[data-morph-exit]').count(),0);
 await page.locator('[id="prod-cap-app:notes"]').click();
 assert((await page.locator('#prod-inspector-body').textContent()).includes('https://notes.example'));
 await page.keyboard.press('Escape');
 await page.emulateMedia({reducedMotion:'reduce'});
 appConnections.push({name:'tasks',base_url:'https://tasks.example'});
 await page.evaluate(list=>window.dispatchEvent(new CustomEvent('klyne-connections',{detail:list})),appConnections);
 await page.waitForTimeout(100);
 assert.equal(await page.locator('[id="prod-cap-app:tasks"]').evaluate(n=>n.getAnimations().length),0);
 await page.screenshot({path:'workspace/activity-app-cards.png',fullPage:true});
 // The ASCII sun responds to pointer input and scales with real active calls.
 await page.emulateMedia({reducedMotion:'no-preference'});
 await page.evaluate(()=>{fixture.status='Stopped';fixture.activity=null;show();});
 await page.mouse.move(0,0);await page.waitForTimeout(1100);
 const idleSun=await page.locator('#prod-reactor').getAttribute('data-sun-energy');
 const beforePointer=await pixels();
 await page.waitForTimeout(250);
 assert.notEqual(await pixels(),beforePointer,'Stopped sun retains its minimum living flame');
 await page.screenshot({path:'workspace/ascii-sun-minimum.png',fullPage:true});
 const sun=await page.locator('#prod-core').boundingBox();
 await page.mouse.move(sun.x+sun.width/2+sun.height*.32,sun.y+sun.height/2);
 await page.waitForTimeout(350);
 assert.notEqual(await pixels(),beforePointer);
 await page.mouse.move(0,0);
 await page.evaluate(()=>{fixture.status='Working';fixture.activity=null;show();});
 await page.waitForTimeout(900);
 const processingEnergy=Number(await page.locator('#prod-reactor').getAttribute('data-sun-energy'));
 assert(processingEnergy>Number(idleSun));
 await page.evaluate(()=>{
   for(let i=0;i<3;i++)fixture.activity_events.push({seq:fixture.activity_events.at(-1).seq+1,type:'tool:start',data:{id:'sun-'+i,tool:['browser','files','terminal'][i]}});
   show();
 });
 await page.waitForTimeout(700);
 assert(Number(await page.locator('#prod-reactor').getAttribute('data-sun-energy'))>processingEnergy);
 assert(Number(await page.locator('#prod-reactor').getAttribute('data-sun-glyphs'))>200);
 await page.evaluate(()=>window.dispatchEvent(new CustomEvent('klyne-telemetry',{detail:{gpu_percent:100}})));
 await page.waitForTimeout(700);
 const gpuEnergy=Number(await page.locator('#prod-reactor').getAttribute('data-sun-energy'));
 await page.evaluate(()=>window.dispatchEvent(new CustomEvent('klyne-telemetry',{detail:{gpu_percent:0}})));
 await page.waitForTimeout(700);
 assert(gpuEnergy>Number(await page.locator('#prod-reactor').getAttribute('data-sun-energy')),'Measured GPU load increases flames');
 await page.screenshot({path:'workspace/ascii-sun-active.png',fullPage:true});

 // Completion must not resize the composer and recenter the entire workspace.
 for(const viewport of [{width:1920,height:1080},{width:1300,height:820}]){
   await page.setViewportSize(viewport);
   await page.evaluate(()=>{fixture.status='Working';fixture.activity=null;show();});
   await page.waitForTimeout(650);
   const positions=async()=>page.evaluate(()=>['prod-core','prod-cap-files','chat-form'].map(id=>{
     const r=document.getElementById(id).getBoundingClientRect();return [r.x,r.y,r.width,r.height];
   }));
   const before=await positions();
   for(const status of ['Reviewing','Completed','Blocked','Working']){
     await page.evaluate(status=>{fixture.status=status;show();},status);
     await page.waitForTimeout(650);
     const after=await positions();
     after.forEach((rect,i)=>rect.forEach((v,j)=>assert(Math.abs(v-before[i][j])<1,
       status+' shifted '+['core','files','composer'][i]+' at '+viewport.width+': '+JSON.stringify({before,after}))));
   }
 }

 await page.setViewportSize({width:1920,height:1080});
 await page.evaluate(()=>{fixture.messages=[{role:'user',text:'Hello input'},{agent:'Klyne',role:'assistant',text:'Hello output'}];show();});
 await page.waitForTimeout(650);
 assert.equal(await page.locator('.prod-input-panel #chat-form').count(),1);
 assert((await page.locator('#prod-output-content').textContent()).includes('Hello output'));
 const panels=await page.evaluate(()=>{
   const rect=id=>document.getElementById(id).getBoundingClientRect();
   return {input:rect('chat-form').right,graph:rect('prod-graph').left,output:rect('prod-output-content').left,right:rect('prod-graph').right,card:rect('prod-cap-files').width};
 });
 assert(panels.input<panels.graph && panels.output>panels.right);
 assert(panels.card<=201);
 await page.screenshot({path:'workspace/activity-side-chat.png',fullPage:true});
 await page.locator('#prod-view-toggle').click();await page.waitForTimeout(500);
 assert.equal(await page.locator('.prod-input-panel #chat-form').count(),0);
 assert(await page.locator('#chat-form').isVisible());
 assert.deepEqual(errors,[]);

 await browser.close();console.log('Activity UI: parallel operations, waiting, completion, failure, inspection, mobile, reduced motion passed.');
})().catch(e=>{console.error(e);process.exit(1)});
