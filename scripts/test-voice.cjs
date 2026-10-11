const assert=require('assert/strict');
const fs=require('fs');
const path=require('path');
const {launch}=require('./test-browser.cjs');
(async()=>{
 const browser=await launch();
 try{
  const page=await browser.newPage(),errors=[],voiceCalls=[],submissions=[];let chat=null,seq=0;
  page.on('pageerror',e=>errors.push(e.message));
  await page.addInitScript(()=>{
    window.__stopped=0;window.__played=0;
    Object.defineProperty(navigator,'mediaDevices',{value:{getUserMedia:async()=>({getTracks:()=>[{stop(){window.__stopped++;}}]})}});
    window.AudioContext=class {constructor(){this.sampleRate=16000;this.destination={};this.audioWorklet={addModule:async()=>{}};}resume(){return Promise.resolve();}close(){return Promise.resolve();}createMediaStreamSource(){return {connect(){},disconnect(){}};}createGain(){return {gain:{value:1},connect(){}};}};
    window.AudioWorkletNode=class {constructor(){this.port={onmessage:null};window.__capture=this;}connect(){}disconnect(){}};
    window.Audio=class {play(){window.__played++;if(!window.__holdAudio)setTimeout(()=>this.onended?.(),20);return Promise.resolve();}pause(){window.__paused=true;}};
  });
  await page.route('http://127.0.0.1:4317/**',route=>{
   const req=route.request(),url=new URL(req.url());
   if(!url.pathname.startsWith('/api/')){const file=url.pathname==='/'?'chat.html':url.pathname.slice(1);return route.fulfill({path:path.resolve('apps/studio/web',file),contentType:file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html'});}
   const body=req.method()==='POST'?req.postDataJSON():null;
   if(url.pathname==='/api/voice/cancel')return route.fulfill({json:{cancelled:true}});
   if(url.pathname==='/api/voice'){
    if(!body)return route.fulfill({json:{installed:true}});
    voiceCalls.push(body);
    return route.fulfill({json:body.operation==='transcribe'?{text:'Explain a compiler',language:'en'}:body.operation==='health'?{ready:true,asr:'fixture ASR',tts:'fixture TTS'}:{audio:Buffer.from('fixture').toString('base64'),mime:'audio/wav'}});
   }
   if(url.pathname==='/api/chats'&&body){submissions.push(body);chat={id:'123',title:body.message,status:'Completed',messages:[...(chat?.messages||[]),{role:'user',agent:'You',text:body.message},{role:'assistant',agent:'Klyne',text:'A compiler translates source code.'}],tasks:[],evidence:[],pending:null,activity:null,activity_events:[],execution:{},provider:body.provider,access:body.access,workspace:'',used:1,limit:0};seq++;return route.fulfill({json:{id:'123'}});}
   return route.fulfill({json:url.pathname==='/api/chats'?{chats:chat?[chat]:[]}:url.pathname==='/api/chats/123'?chat:url.pathname.includes('/pulse')?{seq,status:chat?.status}:url.pathname==='/api/apps'?{connections:[]}:{}});
  });
  await page.goto('http://127.0.0.1:4317/');await page.waitForFunction(()=>!!window.klyneVoice);
  await page.locator('#voice-record').click();await page.waitForFunction(()=>window.__capture?.port.onmessage);
  await page.evaluate(()=>window.__capture.port.onmessage({data:new Float32Array(16000).fill(0.1)}));
  await page.locator('#voice-record').click();await page.waitForFunction(()=>document.querySelector('#instruction').value==='Explain a compiler');
  assert.equal(submissions.length,0,'dictation must not auto-submit by default');
  const wav=Buffer.from(voiceCalls[0].audio,'base64');assert.equal(wav.toString('ascii',0,4),'RIFF');assert.equal(wav.readUInt32LE(24),16000);assert.equal(wav.length,32044);
  assert.equal(await page.evaluate(()=>window.__stopped),1,'microphone released after dictation');
  await page.locator('#send').click();await page.waitForFunction(()=>window.__played===1);
  assert.equal(submissions[0].voice_turn,true);assert.equal(voiceCalls.filter(c=>c.operation==='speak').length,1);
  await page.waitForTimeout(1700);assert.equal(voiceCalls.filter(c=>c.operation==='speak').length,1,'polling must not replay speech');
  await page.locator('#new-chat').click();assert.equal(await page.locator('#voice-status').textContent(),'Voice off');
  await page.locator('#voice-record').click();await page.waitForFunction(()=>window.__capture?.port.onmessage);
  await page.locator('#voice-stop').click();assert.equal(await page.evaluate(()=>window.__stopped),2);
  const chunks=await page.evaluate(()=>klyneVoice.splitSpeech('x'.repeat(1600)));assert(chunks.every(c=>c.length<=450));assert(chunks.length<10);
  await page.evaluate(()=>{document.querySelector('#voice-interrupt').checked=true;window.__holdAudio=true;});
  await page.locator('#voice-conversation').click();
  await page.waitForFunction(()=>window.__capture?.port.onmessage);
  await page.evaluate(()=>{window.__capture.port.onmessage({data:new Float32Array(8000).fill(0.1)});window.__capture.port.onmessage({data:new Float32Array(24000)});});
  await page.waitForFunction(()=>window.__played===2&&document.querySelector('#voice-status').textContent.includes('interruption'));
  await page.evaluate(()=>window.__capture.port.onmessage({data:new Float32Array(4000).fill(0.1)}));
  await page.waitForFunction(()=>window.__paused&&document.querySelector('#voice-status').textContent.includes('Interrupted'));
  await page.locator('#voice-stop').click();
  assert.equal(await page.locator('#voice-conversation').getAttribute('aria-pressed'),'false');
  assert.equal(voiceCalls.filter(c=>c.operation==='speak').at(-1).language,'en');
  await page.setViewportSize({width:390,height:844});assert(await page.locator('#voice-record').isVisible());
  assert.deepEqual(errors,[]);
  console.log('Voice UI: PCM capture, review before sending, voice turn, single reply playback, spoken interruption, mic release, cancellation and mobile passed.');
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
