'use strict';
(() => {
  const el=id=>document.getElementById(id);
  const settings=document.createElement('details');settings.id='voice-settings';
  settings.innerHTML=`<summary>Voice</summary><p class="settings-copy">Moonshine recognition and voice synthesis run locally on your CPU. Your AI connection still determines where conversation text is processed. Audio is kept in memory, not saved.</p>
  <label for="voice-language">Recognition language</label><select id="voice-language"><option value="en">English</option><option value="vi">Vietnamese · preview</option></select>
  <label><input id="voice-read-replies" type="checkbox" checked> Speak replies to my voice requests</label>
  <label><input id="voice-auto-send" type="checkbox"> Send voice transcripts automatically</label>
  <label><input id="voice-interrupt" type="checkbox"> Interrupt by speaking in conversation mode (headphones recommended)</label>
  <label for="voice-output">Speaking voice</label><select id="voice-output"><option value="en">Kokoro · George · British English</option><option value="computer">Installed computer voice</option></select><select id="voice-computer" aria-label="Installed local voice"></select>
  <p class="settings-copy">Choose the language you will speak. For Vietnamese replies, choose an installed Vietnamese computer voice. Conversation mode sends turns after a pause; use headphones for reliable interruption detection.</p>
  <button id="voice-check" type="button">Check voice</button><button id="voice-preview" type="button">Test speaking voice</button><p id="voice-health" role="status"></p>`;
  el('execution-settings').before(settings);
  const bar=document.createElement('div');bar.className='voice-controls';
  bar.innerHTML=`<button id="voice-record" type="button" aria-label="Talk using microphone" aria-pressed="false" title="Alt + Space">🎙 Talk</button><button id="voice-conversation" type="button" aria-pressed="false">Conversation</button><button id="voice-stop" type="button">Stop voice</button><span id="voice-status" role="status" aria-live="polite">Voice off</span>`;
  el('chat-form').before(bar);
  let generation=0,request=null,audio=null,stream=null,context=null,node=null,source=null;
  let chunks=[],samples=0,recording=false,opening=false,continuous=false,voiced=0,silence=0;
  let monitoring=false;
  let pendingVoice=false,watch=null,currentChat=null,observedChat=null,pausedForReply=false;
  let speechQueue=[],speechActive=false,speechResolve=null,recordChat=null;
  const savedIds=['voice-language','voice-read-replies','voice-auto-send','voice-output','voice-computer','voice-interrupt'];
  let saved={};try{saved=JSON.parse(localStorage.getItem('klyne-voice')||'{}');}catch(_){}
  for(const id of savedIds){if(id==='voice-computer')continue;if(saved[id]!==undefined){if(el(id).type==='checkbox')el(id).checked=!!saved[id];else el(id).value=saved[id];}}
  for(const id of ['voice-language','voice-output'])if(!el(id).value)el(id).value='en';
  const status=text=>{el('voice-status').textContent=text;};
  function save(){const values={};for(const id of savedIds)values[id]=el(id).type==='checkbox'?el(id).checked:el(id).value;try{localStorage.setItem('klyne-voice',JSON.stringify(values));}catch(_){} }
  for(const id of savedIds)el(id).onchange=save;
  function voices(){
    const choice=el('voice-computer').value||saved['voice-computer'];
    el('voice-computer').replaceChildren(...(window.speechSynthesis?.getVoices()||[]).filter(v=>v.localService).map(v=>{const option=document.createElement('option');option.value=v.voiceURI;option.textContent=`${v.name} · ${v.lang}`;return option;}));
    if(choice)el('voice-computer').value=choice;
    if(!el('voice-computer').value && el('voice-computer').options.length)el('voice-computer').selectedIndex=0;
  }
  window.speechSynthesis?.addEventListener('voiceschanged',voices);voices();
  async function call(operation,fields={}){
    if(request)throw Error('Voice is still finishing the previous operation.');
    const id=crypto.randomUUID(),controller=new AbortController();request={id,controller};
    const timeout=setTimeout(()=>cancelRequest(),125000);
    try{
      const response=await fetch('/api/voice',{method:'POST',headers:{'Content-Type':'application/json','X-Klyne-Request':'1'},body:JSON.stringify({operation,request_id:id,...fields}),signal:controller.signal});
      const value=await response.json();if(!response.ok||value.error)throw Error(value.error||'Voice request failed.');return value;
    }finally{clearTimeout(timeout);if(request?.id===id)request=null;}
  }
  function cancelRequest(){
    const previous=request;if(!previous)return Promise.resolve();request=null;previous.controller.abort();
    return fetch('/api/voice/cancel',{method:'POST',headers:{'Content-Type':'application/json','X-Klyne-Request':'1'},body:JSON.stringify({request_id:previous.id}),signal:AbortSignal.timeout(5000)}).catch(()=>{});
  }
  function stopSpeech(){
    generation++;speechQueue=[];speechActive=false;
    if(audio){audio.pause();audio.src='';audio=null;}
    window.speechSynthesis?.cancel();if(speechResolve){speechResolve();speechResolve=null;}
    return cancelRequest();
  }
  function releaseMic(){
    recording=false;opening=false;monitoring=false;
    if(node){node.port.onmessage=null;node.disconnect();node=null;}
    if(source){source.disconnect();source=null;}
    stream?.getTracks().forEach(t=>t.stop());stream=null;
    if(context){context.close().catch(()=>{});context=null;}
    el('voice-record').textContent='🎙 Talk';el('voice-record').setAttribute('aria-pressed','false');
  }
  function stopAll(){continuous=false;pausedForReply=false;watch=null;pendingVoice=false;releaseMic();chunks=[];samples=0;stopSpeech();el('voice-conversation').setAttribute('aria-pressed','false');status('Voice off');}
  function wavBase64(parts,count){
    const buffer=new ArrayBuffer(44+count*2),view=new DataView(buffer);const str=(offset,text)=>{for(let i=0;i<text.length;i++)view.setUint8(offset+i,text.charCodeAt(i));};
    str(0,'RIFF');view.setUint32(4,36+count*2,true);str(8,'WAVE');str(12,'fmt ');view.setUint32(16,16,true);view.setUint16(20,1,true);view.setUint16(22,1,true);view.setUint32(24,16000,true);view.setUint32(28,32000,true);view.setUint16(32,2,true);view.setUint16(34,16,true);str(36,'data');view.setUint32(40,count*2,true);
    let offset=44;for(const part of parts)for(const value of part){view.setInt16(offset,Math.round(Math.max(-1,Math.min(1,value))*32767),true);offset+=2;}
    const bytes=new Uint8Array(buffer);let binary='';for(let i=0;i<bytes.length;i+=8192)binary+=String.fromCharCode(...bytes.subarray(i,i+8192));return btoa(binary);
  }
  async function startRecording(monitor=false){
    if(recording||opening)return;
    if(!monitor)await stopSpeech();const epoch=generation;opening=true;monitoring=monitor;recordChat=currentChat;status('Opening microphone…');
    try{
      if(!navigator.mediaDevices?.getUserMedia)throw Error('Microphone access requires localhost and a supported browser.');
      const captured=await navigator.mediaDevices.getUserMedia({audio:{channelCount:1,echoCancellation:true,noiseSuppression:true,autoGainControl:true}});
      if(epoch!==generation){captured.getTracks().forEach(t=>t.stop());return;}
      stream=captured;context=new AudioContext({sampleRate:16000});await context.resume();
      if(context.sampleRate!==16000)throw Error('This browser cannot record at 16 kHz. Try current Edge or Chrome.');
      await context.audioWorklet.addModule('/voice-worklet.js');
      if(epoch!==generation){releaseMic();return;}
      source=context.createMediaStreamSource(stream);node=new AudioWorkletNode(context,'klyne-recorder');
      const mute=context.createGain();mute.gain.value=0;source.connect(node);node.connect(mute);mute.connect(context.destination);
      chunks=[];samples=0;voiced=0;silence=0;recording=true;opening=false;pausedForReply=false;
      node.port.onmessage=event=>{
        if(!recording)return;const values=event.data;
        const remaining=480000-samples,part=values.length>remaining?values.slice(0,remaining):values;chunks.push(part);samples+=part.length;
        let sum=0;for(const value of part)sum+=value*value;const loud=Math.sqrt(sum/Math.max(1,part.length))>0.018;
        if(monitoring){
          const sustained=Math.sqrt(sum/Math.max(1,part.length))>0.04;
          voiced=sustained?voiced+part.length:0;
          if(voiced>3200){monitoring=false;stopSpeech();silence=0;status('Interrupted · listening to you');}
          else if(samples>16000){const recent=new Float32Array(samples);let offset=0;for(const p of chunks){recent.set(p,offset);offset+=p.length;}chunks=[recent.slice(-8000)];samples=8000;}
          return;
        }
        if(loud){voiced+=part.length;silence=0;}else silence+=part.length;
        if(continuous && !voiced && samples>16000*5){chunks=[];samples=0;silence=0;}
        if(samples>=480000 || continuous&&voiced>3200&&silence>19200)finishRecording();
      };
      el('voice-record').textContent='Finish';el('voice-record').setAttribute('aria-pressed','true');status(continuous?'Listening · speak, then pause':'Listening · click Finish when done');
    }catch(error){releaseMic();continuous=false;el('voice-conversation').setAttribute('aria-pressed','false');status(error.message);}
  }
  async function finishRecording(){
    if(!recording)return;
    const parts=chunks,count=samples,origin=recordChat,epoch=generation;releaseMic();chunks=[];samples=0;
    if(count<1600){status('Recording was too short.');return;}
    status('Transcribing locally…');
    try{
      const result=await call('transcribe',{audio:wavBase64(parts,count),language:el('voice-language').value});
      if(epoch!==generation || origin!==currentChat)return;
      const text=result.text.trim();
      if(!text){status('No speech detected.');if(continuous)startRecording();return;}
      el('instruction').value=text;el('instruction').dispatchEvent(new Event('input',{bubbles:true}));
      if(continuous||el('voice-auto-send').checked){
        const command=text.toLowerCase().replace(/[.!?,]/g,'').trim();
        if(['stop the task','stop task','cancel the task'].includes(command)){
          if(!el('stop-chat').hidden)el('stop-chat').click();el('instruction').value='';status('Task stop requested.');if(continuous)startRecording();return;
        }
        if(['stop listening','sleep','go to sleep','stop voice'].includes(command)){el('instruction').value='';stopAll();return;}
        if(['status','task status','how is the task going','how’s the task going','how is it going'].includes(command)&&observedChat){el('instruction').value='';await speak(`The task is ${observedChat.status.toLowerCase()}.`);return;}
        if(!el('stop-chat').hidden){status('Task is running. Your transcript is ready to review; stop or finish the task before sending.');return;}
        pendingVoice=true;pausedForReply=true;status('Waiting for Klyne…');el('chat-form').requestSubmit();
      }else{status('Transcript ready · edit it or press Send');pendingVoice=true;el('instruction').focus();}
    }catch(error){if(epoch===generation){status(error.name==='AbortError'?'Voice stopped':error.message);continuous=false;el('voice-conversation').setAttribute('aria-pressed','false');}}
  }
  function splitSpeech(text){
    const plain=text.replace(/```[\s\S]*?```/g,'Code is shown in the conversation.').replace(/\[([^\]]+)\]\([^)]+\)/g,'$1').replace(/[#*_`]/g,'').trim();
    const shortened=plain.length>1600?plain.slice(0,1500)+' The full answer is in the conversation.':plain;
    const result=[];let remaining=shortened;
    while(remaining.length){let end=Math.min(320,remaining.length);if(end<remaining.length){const boundary=Math.max(remaining.lastIndexOf('. ',end),remaining.lastIndexOf('? ',end),remaining.lastIndexOf('! ',end));const space=remaining.lastIndexOf(' ',end);end=boundary>60?boundary+1:space>60?space:end;}
      result.push(remaining.slice(0,end).trim());remaining=remaining.slice(end).trim();}
    return result.filter(Boolean);
  }
  async function play(chunk,epoch){
    if(el('voice-output').value==='computer'){
      const voice=window.speechSynthesis?.getVoices().find(v=>v.localService && v.voiceURI===el('voice-computer').value);
      if(!voice)throw Error('Choose an installed computer voice in Settings.');
      return new Promise((resolve,reject)=>{speechResolve=resolve;const utterance=new SpeechSynthesisUtterance(chunk);utterance.voice=voice;utterance.onend=()=>{speechResolve=null;resolve();};utterance.onerror=()=>{speechResolve=null;epoch===generation?reject(Error('Computer speech was interrupted.')):resolve();};window.speechSynthesis.speak(utterance);});
    }
    const result=await call('speak',{text:chunk,language:el('voice-output').value});if(epoch!==generation)return;
    const bytes=Uint8Array.from(atob(result.audio),c=>c.charCodeAt(0));const url=URL.createObjectURL(new Blob([bytes],{type:'audio/wav'}));
    try{await new Promise((resolve,reject)=>{speechResolve=resolve;audio=new Audio(url);audio.onended=()=>{speechResolve=null;resolve();};audio.onerror=()=>{speechResolve=null;reject(Error('Could not play voice audio.'));};audio.play().catch(reject);});}
    finally{URL.revokeObjectURL(url);audio=null;}
  }
  async function speak(text){
    await stopSpeech();releaseMic();const epoch=generation;speechActive=true;speechQueue=splitSpeech(text);status('Speaking · Talk to interrupt');
    try{if(continuous&&el('voice-interrupt').checked){await startRecording(true);status('Speaking · listening for interruption');}
      while(speechQueue.length && epoch===generation)await play(speechQueue.shift(),epoch);
      if(epoch===generation){speechActive=false;if(monitoring)releaseMic();status('Voice ready');if(continuous)startRecording();}
    }catch(error){if(epoch===generation){speechActive=false;releaseMic();status(error.message);continuous=false;el('voice-conversation').setAttribute('aria-pressed','false');}}
  }
  el('voice-record').onclick=()=>{if(monitoring){monitoring=false;stopSpeech();voiced=0;silence=0;status('Listening · click Finish when done');return;}recording?finishRecording():startRecording();};
  el('voice-conversation').onclick=()=>{if(continuous){stopAll();return;}continuous=true;el('voice-conversation').setAttribute('aria-pressed','true');startRecording();};
  el('voice-stop').onclick=stopAll;
  el('voice-preview').onclick=()=>speak(el('voice-output').value==='vi'?'Xin chào. Tôi có thể giúp gì cho bạn?':'Hello. I am Klyne. What would you like to work on?');
  el('voice-check').onclick=async()=>{el('voice-health').textContent='Checking local speech…';try{const response=await fetch('/api/voice');const info=await response.json();if(!info.installed)throw Error('Run scripts/setup-voice.ps1 in the Klyne folder first.');const result=await call('health');el('voice-health').textContent=`Ready · ${result.asr} · ${result.tts}`;}catch(error){el('voice-health').textContent=error.message;}};
  document.addEventListener('keydown',event=>{if(event.altKey&&event.code==='Space'&&!event.repeat){event.preventDefault();el('voice-record').click();}if(event.key==='Escape'&&(recording||opening||speechActive||continuous))stopAll();});
  window.addEventListener('pagehide',stopAll);
  window.klyneVoice={
    consumeTurn(){const value=pendingVoice;pendingVoice=false;return value;},
    sent(id){watch={id,after:observedChat?.id===id?observedChat.messages.length:0};currentChat=id;},
    failed(){pausedForReply=false;watch=null;status('Could not send. Review the message and connection settings.');continuous=false;el('voice-conversation').setAttribute('aria-pressed','false');},
    selected(id){if(currentChat!==id){stopAll();currentChat=id;observedChat=null;}},
    update(chat){observedChat=chat;if(!chat)return;currentChat=chat.id;
      if(watch?.id===chat.id && !['Planning','Working','Reviewing','Stopping'].includes(chat.status)){
        const reply=chat.messages.slice(watch.after).filter(m=>m.role==='assistant'&&m.agent==='Klyne').at(-1);watch=null;pausedForReply=false;
        if(reply&&el('voice-read-replies').checked)speak(reply.text);else if(continuous)startRecording();
      }
    },
    splitSpeech,wavBase64
  };
})();
