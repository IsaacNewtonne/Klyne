const assert=require('assert/strict');
const fs=require('fs');
const path=require('path');
const {launch}=require('./test-browser.cjs');
const {audit}=require('./audit-ui.cjs');
(async()=>{
 const browser=await launch();
 try{
  const page=await browser.newPage(),errors=[],submitted=[];let jobs=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.route('http://127.0.0.1:4317/**',route=>{
   const req=route.request(),url=new URL(req.url());
   if(!url.pathname.startsWith('/api/')){const file=url.pathname==='/'?'chat.html':url.pathname.slice(1);return route.fulfill({path:path.resolve('apps/studio/web',file),contentType:file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});}
   const body=req.method()==='POST'?req.postDataJSON():null;
   if(url.pathname==='/api/goals'){
    if(body?.action==='submit'){submitted.push(body);jobs.push(...body.jobs.map(j=>({...j,goal:j.request.message,state:'queued',note:'Waiting'})));}
    if(body?.action==='cancel')jobs.find(j=>j.id===body.id).state='cancelled';
    return route.fulfill({json:{jobs}});
   }
   return route.fulfill({json:url.pathname==='/api/chats'?{chats:[]}:url.pathname==='/api/apps'?{connections:[]}:{}});
  });
  await page.goto('http://127.0.0.1:4317/');await page.locator('#goals-button').click();
  await page.locator('#goal-instructions').fill('Create file proof.txt with content ready');
  await page.locator('#goal-add').click();await page.waitForFunction(()=>document.querySelectorAll('.goal-card').length===1);
  assert.equal(submitted[0].jobs[0].require_verified,true);
  assert.equal(submitted[0].jobs[0].request.execution.max_steps,64);
  assert.equal(submitted[0].jobs[0].request.execution.timeout_seconds,900);
  await page.locator('#goal-after-current').check();await page.locator('#goal-reviewed').check();
  await page.locator('#goal-instructions').fill('<img src=x onerror=alert(1)>');await page.locator('#goal-add').click();
  await page.waitForFunction(()=>document.querySelectorAll('.goal-card').length===2);
  assert.deepEqual(submitted[1].jobs[0].depends_on,[submitted[0].jobs[0].id]);
  assert.equal(await page.locator('.goal-card img').count(),0,'goal text must not become markup');
  await page.locator('.goal-card button').first().click();await page.waitForFunction(()=>document.querySelector('.goal-card').textContent.includes('cancelled'));
  await page.setViewportSize({width:390,height:844});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,'board must fit mobile');
  await page.locator('[aria-label="Close goals"]').click();
  const output=path.resolve('workspace/studio-qa/goals-audit');fs.mkdirSync(output,{recursive:true});
  const result=await audit(page,{url:'http://127.0.0.1:4317/',output,ready:'#goals-button',rules:[{selector:'#goals-button',text:'Goals',minWidth:24,minHeight:24}]});
  assert.equal(result.passed,true,JSON.stringify(result));
  const rejected=await audit(page,{url:'http://127.0.0.1:4317/',output,ready:'#goals-button',viewports:[{width:390,height:844}],rules:[{selector:'#missing-required-control'},{selector:'#goals-button',minWidth:2000,styles:{'display':'none'}}]});
  assert.equal(rejected.passed,false);
  assert.deepEqual(new Set(rejected.findings[0].issues.map(i=>i.kind)),new Set(['missing_element','width','style_token']));
  assert.deepEqual(errors,[]);
  console.log('Goal board submission, dependencies, cancellation, safe rendering and responsive audit passed.');
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
