'use strict';
(() => {
  const $ = id => document.getElementById(id);
  const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  const busy = new Set(['Planning','Working','Reviewing','Stopping','Upgrading']);
  // Semantic UI state (docs/UI-COLORS.md): every state = colour + label + icon.
  const UI={idle:['Ready','○'],thinking:['Thinking','◌'],acting:['Working','▶'],approval:['Needs your approval','⚠'],input:['Needs your answer','?'],done:['Done','✓'],blocked:['Couldn’t finish','⏸'],error:['Error','✕'],off:['Access off','🔒']};
  const UNREACHABLE=/connection failed|could not connect|connection refused|not reachable|error sending request/i;
  function lastKlyne(chat){return chat?.messages?.filter(m=>m.agent==='Klyne').at(-1)?.text||'';}
  function uiState(chat,activity,submitting){
    if(submitting)return 'thinking';
    if(!chat)return 'idle';
    const st=chat.status;
    if(['Planning','Working','Reviewing','Stopping','Upgrading'].includes(st)){
      if(chat.pending?.proposal)return 'approval';
      return activity?.operations?.length && activity.mode!=='waiting'?'acting':st==='Working'&&activity?.mode!=='waiting'?'acting':'thinking';
    }
    if(chat.pending?.proposal)return 'approval';
    if(st==='Needs input')return 'input';
    if(chat.pending)return 'blocked';
    if(st==='Completed')return 'done';
    if(UNREACHABLE.test(lastKlyne(chat)))return 'error';
    if(['Blocked','Interrupted','Stopped'].includes(st))return 'blocked';
    return 'idle';
  }
  function uiFromSummary(c){const a=c.attention;return a==='approval'?'approval':a==='input'?'input':a==='error'?'error':a==='blocked'||a==='uncertain'?'blocked':c.status==='Completed'?'done':['Planning','Reviewing'].includes(c.status)?'thinking':['Working','Stopping','Upgrading'].includes(c.status)?'acting':'idle';}
  window.klyneUi={UI,uiState,uiFromSummary,lastKlyne,UNREACHABLE};
  const capabilities = [
    ['files','Files','⌗',null], ['terminal','Terminal','›_', 'terminal'],
    ['browser','Web & browser','◎','web'], ['desktop','Desktop','▣','desktop'],
    ['apps','App APIs','↗','apps'], ['skills','Skills & tools','✳','terminal'],
    ['memory','Memory','◈',null], ['runtime','Runtime','⟳','terminal']
  ];
  const baseCapabilities=capabilities.length;
  let connectedApps=[],appSignature='',appTimer=0,appPending=false;
  function appIcon(name){
    let hash=2166136261;
    for(const c of name)hash=Math.imul(hash^c.charCodeAt(0),16777619)>>>0;
    let cells='';
    for(let y=0;y<5;y++)for(let x=0;x<3;x++){
      if((hash>>>(y*3+x))&1){
        cells+='<rect x="'+(3+x*4)+'" y="'+(3+y*4)+'" width="3" height="3" rx=".6"/>';
        if(x<2)cells+='<rect x="'+(3+(4-x)*4)+'" y="'+(3+y*4)+'" width="3" height="3" rx=".6"/>';
      }
    }
    return '<svg viewBox="0 0 25 25" aria-hidden="true" fill="currentColor">'+cells+'</svg>';
  }
  function syncApps(list){
    if(!Array.isArray(list))return;
    const next=list.filter(a=>typeof a.name==='string').sort((a,b)=>a.name.localeCompare(b.name));
    const signature=JSON.stringify(next.map(a=>[a.name,a.base_url]));
    if(signature===appSignature)return;
    appSignature=signature;connectedApps=next;
    const keys=new Set(next.map(a=>'app:'+a.name));
    for(const item of capabilities.slice(baseCapabilities))if(!keys.has(item[0])){
      const card=$('prod-cap-'+item[0]);if(!card)continue;
      if(!motion.matches && !root.hidden && !document.hidden){
        const rect=card.getBoundingClientRect(),ghost=card.cloneNode(true);
        ghost.removeAttribute('id');ghost.removeAttribute('data-inspect');ghost.dataset.morphExit='true';
        ghost.inert=true;ghost.setAttribute('aria-hidden','true');
        Object.assign(ghost.style,{position:'fixed',left:rect.left+'px',top:rect.top+'px',width:rect.width+'px',height:rect.height+'px',margin:'0',pointerEvents:'none',zIndex:'6'});
        root.append(ghost);
        ghost.animate([{opacity:1,transform:'none'},{opacity:0,transform:'scale(.96)'}],{duration:180,easing:'ease-out'}).finished.catch(()=>{}).finally(()=>ghost.remove());
      }
      card.remove();
    }
    capabilities.splice(baseCapabilities,capabilities.length-baseCapabilities,...next.map(a=>['app:'+a.name,a.name,appIcon(a.name),'apps']));
    graphDirty=true;if(current)render();
  }
  async function refreshApps(){
    clearTimeout(appTimer);
    if(document.hidden || appPending || !$('api-connections'))return;
    appPending=true;
    try{const response=await fetch('/api/apps',{cache:'no-store'});if(response.ok)syncApps((await response.json()).connections);}catch(_){}
    finally{appPending=false;if(!document.hidden)appTimer=setTimeout(refreshApps,5000);}
  }
  const root = document.createElement('section');
  root.id = 'production'; root.hidden = true;
  root.setAttribute('aria-label','Production workspace');
  root.innerHTML = `<div class="prod-heading"><div><p class="prod-eyebrow">KLYNE / WORKSPACE</p><h2 id="prod-title">Putting your intent in motion</h2></div><button id="prod-view-toggle" type="button" aria-pressed="false">Conversation ↗</button></div>
  <div class="prod-meta"><span id="prod-live" role="status" aria-live="polite"></span><span id="prod-metrics"></span><progress id="prod-progress" max="1" value="0" aria-label="Task steps completed"></progress></div>
  <div id="prod-surface"><div class="prod-intent"><span>YOUR INSTRUCTION</span><p id="prod-goal"></p></div>
  <div class="prod-pipeline" aria-label="Execution stages"><span data-stage="Planning">01 <b>Understand</b></span><i></i><span data-stage="Working">02 <b>Execute</b></span><i></i><span data-stage="Reviewing">03 <b>Review</b></span><i></i><span data-stage="Completed">04 <b>Deliver</b></span></div>
  <div id="prod-graph" data-layout="flow"><svg id="prod-edges" aria-hidden="true"></svg>
    <section id="prod-model-panel" class="prod-panel"><p class="prod-eyebrow">Activity</p><div id="prod-core" aria-hidden="true"><svg class="core-fire" viewBox="0 0 100 100" aria-hidden="true"><g fill="#ff792e"><path d="M50 7C70 7 86 21 91 39C81 30 72 31 65 36C69 25 62 16 50 7ZM91 39C98 62 84 85 63 92C72 82 71 73 65 66C77 68 87 57 91 39ZM63 92C40 100 16 85 9 63C19 73 29 72 36 65C33 78 45 89 63 92ZM9 63C1 40 16 16 39 9C29 19 30 29 36 36C23 33 12 45 9 63Z"/></g></svg></div><h3 id="prod-core-state">Thinking</h3><p id="prod-provider"></p><p id="prod-model"></p><div id="prod-request"></div><p class="prod-explainer">One model connection.<br>Distinct planning, work and review roles.</p></section>
    <section class="prod-panel prod-assignment-panel"><div class="prod-panel-heading"><p class="prod-eyebrow">Steps</p><span id="prod-task-count"></span></div><div id="prod-workers"></div><div id="prod-review-node" class="prod-review"><span>◇</span><div><strong>Review the result</strong><small>Read-only review · after execution</small></div></div></section>
    <section class="prod-panel prod-capability-panel"><p class="prod-eyebrow">Connected tools</p><div id="prod-capabilities"></div><p class="prod-explainer">A lit route means observed use.<br>Available capabilities remain on standby.</p></section>
  </div><div class="prod-legend"><span><i class="legend-live"></i>In progress</span><span><i class="legend-done"></i>Observed</span><span><i></i>Standby</span><span>Assignments execute sequentially</span></div>
  <div id="prod-result" hidden><span id="prod-result-icon">✓</span><div><h3 id="prod-result-title"></h3><p id="prod-result-text"></p></div></div>
  <section class="prod-stream"><button id="prod-open-log" type="button">Observations ↗</button><button id="prod-open-result" type="button" hidden>Read full result ↗</button><div class="prod-panel-heading"><p class="prod-eyebrow">OBSERVATIONS</p><span id="prod-observation-count"></span></div><div id="prod-events"></div></section>
  <dialog id="prod-inspector" aria-labelledby="prod-inspector-title"><form method="dialog" class="dialog-heading"><h2 id="prod-inspector-title">Inspect node</h2><button class="icon-button" aria-label="Close inspection">×</button></form><pre id="prod-inspector-body"></pre></dialog>
  </div>`;
  $('main').insertBefore(root,$('main').firstChild);
  root.dataset.theme='ember-console';
  $('prod-core').insertAdjacentHTML('afterbegin','<canvas id="prod-reactor" aria-hidden="true"></canvas><div class="reactor-wordmark">KLYNE<small>YOUR IDEAS, IN MOTION</small></div>');
  root.querySelector('.prod-assignment-panel').insertAdjacentHTML('afterbegin','<section class="prod-health"><p class="prod-eyebrow">SYSTEM PULSE</p><div class="prod-gauge"><strong id="prod-cpu">—</strong><span>CPU</span></div><div class="prod-gauge" title="Whole NVIDIA GPU usage, including other apps"><strong id="prod-gpu">?</strong><span>GPU</span></div><dl><dt>Connection</dt><dd id="prod-health-connection">Studio online</dd><dt>Tools in use</dt><dd id="prod-health-tools">0</dd><dt>Observations</dt><dd id="prod-health-evidence">0</dd></dl><p class="prod-health-note">Live process measurement</p></section>');
  root.querySelector('.prod-heading').insertAdjacentHTML('beforeend','<button id="prod-stop" type="button" hidden>■ Stop</button>');
  $('prod-stop').onclick=()=>($('stop-chat') || $('demo-stop'))?.click();
  window.addEventListener('klyne-telemetry',event=>{
    window.klyneActivity.emit('system:cpu_updated',event.detail);
    window.klyneActivity.emit('system:gpu_updated',event.detail);
    const gpu=event.detail?.gpu_percent;
    $('prod-core').title=Number.isFinite(gpu)?'GPU '+gpu.toFixed(0)+'% - whole NVIDIA GPU usage':'GPU measurement unavailable';
    const gpuValue=Number.isFinite(gpu)?Math.max(0,Math.min(100,gpu)):null;
    $('prod-gpu').textContent=gpuValue===null?'?':gpuValue.toFixed(1)+'%';
    const gpuGauge=$('prod-gpu').parentElement;
    gpuGauge.style.setProperty('--cpu',((gpuValue||0)*3.6)+'deg');
    gpuGauge.style.setProperty('--cpu-intensity',String((gpuValue||0)/100));
    gpuGauge.title=gpuValue===null?'GPU measurement unavailable':'Whole NVIDIA GPU usage, including other apps';
    const value=event.detail?.cpu_percent;
    $('prod-cpu').textContent=typeof value==='number' && Number.isFinite(value)?`${value.toFixed(1)}%`:'—';
    document.querySelector('.prod-gauge').style.setProperty('--cpu-intensity',String(typeof value==='number'?Math.max(0,Math.min(100,value))/100:0));
    document.querySelector('.prod-gauge').style.setProperty('--cpu',`${typeof value==='number'?Math.max(0,Math.min(100,value))*3.6:0}deg`);
  });

  const composer=document.querySelector('.composer-wrap');
  const composerHome=document.createComment('composer home');
  composer.before(composerHome);
  const desk=document.createElement('div');desk.className='prod-desk';
  const inputPanel=document.createElement('section');inputPanel.className='prod-input-panel';
  inputPanel.innerHTML='<h3>Chat input</h3><div id="prod-input-history"></div>';
  const outputPanel=document.createElement('section');outputPanel.className='prod-output-panel';
  outputPanel.innerHTML='<h3>Output</h3><div id="prod-output-content" role="log" aria-label="Klyne responses"></div>';
  $('prod-surface').before(desk);desk.append(inputPanel,$('prod-surface'),outputPanel);
  function dockComposer(enabled){
    desk.hidden=!enabled;
    if(enabled && composer.parentElement!==inputPanel)inputPanel.append(composer);
    if(!enabled && composer.parentNode!==composerHome.parentNode)composerHome.after(composer);
  }
  function updateSideChat(chat){
    const users=[...(chat?.messages||[]).filter(m=>m.role==='user'),...(chat?.dialogue||[]).map(t=>({text:t.message}))];
    markup($('prod-input-history'),users.map(m=>'<p>'+esc(m.text)+'</p>').join(''));
    const answers=(chat?.messages||[]).filter(m=>m.role!=='user'&&m.agent==='Klyne');
    const output=$('prod-output-content'),nearEnd=output.scrollHeight-output.scrollTop-output.clientHeight<50;
    const live=(chat?.dialogue||[]).map(t=>'<article><small>'+esc(t.state.replaceAll('_',' '))+'</small><p>'+esc(t.reply||'Thinking…')+'</p>'+(['queued','responding'].includes(t.state)?'<button type="button" data-live-cancel="'+esc(t.id)+'">Cancel '+(t.state==='queued'?'queued task':'reply')+'</button>':'')+'</article>').join('');
    markup(output,answers.length||live?answers.map(m=>'<article>'+esc(m.text)+'</article>').join('')+live:'<p class="prod-output-empty">Klyne’s answer will appear here when the work is done. Watch the trail under the orb to see each step as it happens.</p>');
    if(nearEnd)output.scrollTop=output.scrollHeight;
  }
  $('prod-output-content').addEventListener('click',event=>{const button=event.target.closest('[data-live-cancel]');if(button)document.querySelector('#live-chat-log [data-cancel-turn="'+button.dataset.liveCancel+'"]')?.click();});
  const health=root.querySelector('.prod-health');
  const header=document.querySelector('.app>header') || document.querySelector('header');
  if(header) header.insertBefore(health,$('settings-button'));
  const wordmark=root.querySelector('.reactor-wordmark');
  wordmark.replaceChildren($('prod-provider'),$('prod-model'));
  $('prod-core').removeAttribute('aria-hidden');
  const noticeButton=document.createElement('button');
  noticeButton.id='prod-notice';noticeButton.type='button';noticeButton.hidden=true;
  noticeButton.setAttribute('aria-haspopup','dialog');
  root.querySelector('.prod-heading').append(noticeButton);
  noticeButton.onclick=()=>showInspector('result:all');
  const stepsButton=document.createElement('button');
  stepsButton.id='prod-open-steps';stepsButton.type='button';stepsButton.textContent='Steps';
  root.querySelector('.prod-stream').append(stepsButton);
  stepsButton.onclick=()=>showInspector('steps:all');
  const reactor=$('prod-reactor'), brush=reactor.getContext('2d');
  let reactorFrame=0,reactorIdle=0,lastReactorFrame=0,inViewport=true,modeSince=0,lastMode='idle';
  let dimensions={w:1,h:1,dpr:1}, routes=[], graphDirty=true;

  const prodCore=$('prod-core');
  const sunPointer={x:0,y:0,strength:0,target:0};
  let sunEnergy=0,lastSunTime=0;
  prodCore.addEventListener('pointermove',event=>{
    if(motion.matches)return;
    const box=prodCore.getBoundingClientRect();
    sunPointer.x=(event.clientX-box.left-box.width/2)/Math.min(box.width,box.height);
    sunPointer.y=(event.clientY-box.top-box.height/2)/Math.min(box.width,box.height);
    sunPointer.target=Math.max(0,1-Math.abs(Math.hypot(sunPointer.x,sunPointer.y)-.33)*2);
    refreshReactor();
  },{passive:true});
  function releaseSun(){sunPointer.target=0;refreshReactor();}
  prodCore.addEventListener('pointerleave',releaseSun);
  prodCore.addEventListener('pointercancel',releaseSun);
  prodCore.addEventListener('pointerup',event=>{if(event.pointerType==='touch')releaseSun();});
  window.addEventListener('blur',releaseSun);
  // Signal flow (2026-10-24): comets travel along each orb↔tool route, driven
  // only by real activity events (tool:start/result/complete/error) and the
  // pending approval. Canvas + requestAnimationFrame; the loop stops when idle.
  const flow=(()=>{
    const graphEl=$('prod-graph'),edgesEl=$('prod-edges');
    const canvas=document.createElement('canvas');canvas.id='prod-flow';canvas.setAttribute('aria-hidden','true');
    edgesEl.after(canvas);graphEl.classList.add('flow-on');
    const ctx=canvas.getContext('2d');
    let geo={routes:[],cx:0,cy:0,r:0,w:0,h:0},frame=0,frames=0,ops=[],approvalKey=null,failed=new Set(),uiColor='',showLines=true;
    const comets=[],glows=new Map(),badges=new Map(),seenFlash=new Set(),opSeen=new Map();
    const css=name=>getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    const capColor=key=>css('--cap-'+(String(key).startsWith('app:')?'apps':key))||css('--brand')||'#ff8a3d';
    const stColor=name=>css('--st-'+name)||'#ffffff';
    function rgba(hex,a){const h=hex.replace('#','');const n=parseInt(h.length===3?h.split('').map(c=>c+c).join(''):h.slice(0,6),16);return `rgba(${n>>16&255},${n>>8&255},${n&255},${a})`;}
    function point(R,t){const u=1-t;return[u*u*u*R.x1+3*u*u*t*R.c1x+3*u*t*t*R.c2x+t*t*t*R.x2,u*u*u*R.y1+3*u*u*t*R.c1y+3*u*t*t*R.c2y+t*t*t*R.y2];}
    function resize(g){
      geo=g;const dpr=Math.min(devicePixelRatio||1,2);
      const W=Math.max(1,Math.round(g.w*dpr)),H=Math.max(1,Math.round(g.h*dpr));
      if(canvas.width!==W||canvas.height!==H){canvas.width=W;canvas.height=H;}
      ctx.setTransform(dpr,0,0,dpr,0,0);
      const es=getComputedStyle(edgesEl);showLines=es.display!=='none';
      kick();
    }
    const fmt=ms=>ms==null||!isFinite(ms)||ms<0?'':ms<1000?Math.round(ms)+' ms':(ms/1000).toFixed(ms<10000?1:0)+' s';
    function spawn(key,dir,color,speed,big){comets.push({key,dir,color,born:performance.now(),dur:speed,big:!!big});}
    function sync(state,chat){
      const now=performance.now(),connected=state.connected;
      ops=connected?state.operations:[];
      failed=new Set((state.failures||[]).map(f=>f.tool));
      const p=chat?.pending?.proposal;
      approvalKey=null;
      if(p){const cat=window.klyneActivity.category;approvalKey=p.kind==='shell'||p.kind==='secret'?'terminal':p.kind==='delete'?(geo.routes.some(r=>r.key==='app:'+p.connection)?'app:'+p.connection:'apps'):(cat(p.action)||'files');}
      for(const o of ops){
        if(!opSeen.has(o.id)){opSeen.set(o.id,{at:now});if(!state.replaying)spawn(o.tool,1,capColor(o.tool),900,true);}
        badges.set(o.tool,{text:o.operation||'Working',start:o.started_ms,status:o.status,live:true,until:Infinity});
      }
      for(const [id] of opSeen)if(!ops.some(o=>o.id===id))opSeen.delete(id);
      for(const f of state.flashes||[]){
        if(seenFlash.has(f.id))continue;seenFlash.add(f.id);
        const ok=f.status!=='failed',color=stColor(ok?'done':'error');
        spawn(f.tool,-1,color,750,true);
        glows.set(f.tool,{color,at:now,ok});
        const took=f.at_ms&&f.started_ms?f.at_ms-f.started_ms:null;
        badges.set(f.tool,{text:(ok?'✓ ':'✕ ')+(f.operation||'Done'),ms:took,status:ok?'done':'failed',live:false,until:now+3500});
      }
      if(seenFlash.size>200)seenFlash.clear();
      for(const [k,b] of badges)if(b.live&&!ops.some(o=>o.tool===k))badges.delete(k);
      uiColor=css('--ui-state')||stColor('idle');
      kick();
    }
    function animating(now){
      if(comets.length)return true;
      for(const g of glows.values())if(now-g.at<6000)return true;
      for(const b of badges.values())if((b.live&&!motion.matches)||(!b.live&&b.until>now))return true;
      if(!motion.matches&&(ops.length||approvalKey))return true;
      return false;
    }
    let slowFrame=0;
    function kick(){if(!frame)frame=requestAnimationFrame(draw);}
    function badge(R,text,color,alpha){
      const [x,y]=point(R,.5);ctx.font='600 11px "Segoe UI",system-ui,sans-serif';
      const w=ctx.measureText(text).width+16,h=20;
      ctx.globalAlpha=alpha;ctx.fillStyle='rgba(14,12,11,.92)';ctx.strokeStyle=color;ctx.lineWidth=1;
      ctx.beginPath();ctx.roundRect(x-w/2,y-h/2,w,h,10);ctx.fill();ctx.stroke();
      ctx.fillStyle=color;ctx.textAlign='center';ctx.textBaseline='middle';ctx.fillText(text,x,y+.5);ctx.globalAlpha=1;
    }
    function draw(now){
      frame=0;clearTimeout(slowFrame);slowFrame=0;frames++;ctx.clearRect(0,0,geo.w,geo.h);
      if(document.hidden||root.hidden||conversation){return;}
      {const es=getComputedStyle(edgesEl);showLines=es.display!=='none'&&parseFloat(es.opacity)>0;}
      const still=motion.matches,breathe=still?1:.65+.35*Math.sin(now/380);
      // State ring around the orb (fix #8): colour = current UI state.
      if(geo.r>0){
        const ring=uiColor||stColor('idle'),ap=approvalKey&&!still;
        ctx.save();ctx.strokeStyle=rgba(ring,ap?.5+.35*Math.sin(now/380):.7);ctx.lineWidth=2;ctx.shadowColor=ring;ctx.shadowBlur=12;
        ctx.beginPath();ctx.arc(geo.cx,geo.cy,geo.r*.98,0,Math.PI*2);ctx.stroke();ctx.restore();
      }
      if(showLines)for(const R of geo.routes){
        const k=R.key,cap=capColor(k),active=ops.filter(o=>o.tool===k),g=glows.get(k);
        let color=cap,alpha=.2,width=1,blur=0;
        if(failed.has(k)){color=stColor('error');alpha=.35;}
        if(g){const age=(now-g.at)/6000;if(age<1){color=age<.4?g.color:cap;alpha=Math.max(alpha,.85*(1-age)+.2*age);width=1+2*(1-age);blur=10*(1-age);}else glows.delete(k);}
        if(active.length){const waiting=active.every(o=>o.status==='waiting');color=cap;alpha=waiting?.6:.95;width=waiting?2:3;blur=12;}
        if(k===approvalKey){color=stColor('approval');alpha=.55+.4*(breathe-.65)/.35*(still?0:1)+(still?.35:0);width=3;blur=16;}
        ctx.save();ctx.strokeStyle=rgba(color,Math.min(1,alpha));ctx.lineWidth=width;ctx.lineCap='round';
        if(blur){ctx.shadowColor=color;ctx.shadowBlur=blur;}
        if(failed.has(k)&&!active.length&&!g&&k!==approvalKey)ctx.setLineDash([4,5]);
        ctx.beginPath();ctx.moveTo(R.x1,R.y1);ctx.bezierCurveTo(R.c1x,R.c1y,R.c2x,R.c2y,R.x2,R.y2);ctx.stroke();ctx.restore();
        // Reduced motion: a static chevron shows direction instead of comets.
        if(still&&(active.length||g)){const dir=active.length?(active[0].direction==='inbound'?-1:1):-1;const [ax,ay]=point(R,.62),[bx,by]=point(R,dir>0?.66:.58);const an=Math.atan2(by-ay,bx-ax);
          ctx.save();ctx.fillStyle=color;ctx.translate(ax,ay);ctx.rotate(an);ctx.beginPath();ctx.moveTo(6,0);ctx.lineTo(-5,-5);ctx.lineTo(-5,5);ctx.closePath();ctx.fill();ctx.restore();}
        if(k===approvalKey){const [hx,hy]=point(R,1);ctx.save();ctx.fillStyle=stColor('approval');ctx.shadowColor=ctx.fillStyle;ctx.shadowBlur=still?8:8+8*breathe;ctx.beginPath();ctx.arc(hx,hy,4.5,0,Math.PI*2);ctx.fill();ctx.restore();}
      }
      // Comets with glowing trails. Outbound = orb→tool, inbound = tool→orb.
      if(showLines&&!still){
        ctx.save();ctx.globalCompositeOperation='lighter';
        for(let i=comets.length-1;i>=0;i--){
          const c=comets[i],R=geo.routes.find(r=>r.key===c.key),t=(now-c.born)/c.dur;
          if(!R||t>=1){comets.splice(i,1);continue;}
          const e=t<.5?2*t*t:1-Math.pow(-2*t+2,2)/2;
          const N=24,head=c.big?5:4;
          for(let j=N;j>=1;j--){
            const tt=Math.max(0,Math.min(1,e-j*.014));const pos=c.dir>0?tt:1-tt;const [x,y]=point(R,pos);
            ctx.fillStyle=rgba(c.color,.6*(1-j/(N+1)));ctx.beginPath();ctx.arc(x,y,Math.max(.8,head*(1-j/(N+4))),0,Math.PI*2);ctx.fill();
          }
          {const pos=c.dir>0?e:1-e;const [x,y]=point(R,pos);
            const halo=ctx.createRadialGradient(x,y,0,x,y,head*4);halo.addColorStop(0,rgba(c.color,.55));halo.addColorStop(1,rgba(c.color,0));
            ctx.fillStyle=halo;ctx.beginPath();ctx.arc(x,y,head*4,0,Math.PI*2);ctx.fill();
            ctx.fillStyle=rgba(c.color,1);ctx.shadowColor=c.color;ctx.shadowBlur=16;ctx.beginPath();ctx.arc(x,y,head,0,Math.PI*2);ctx.fill();
            ctx.shadowBlur=0;ctx.fillStyle='rgba(255,255,255,.9)';ctx.beginPath();ctx.arc(x,y,head*.45,0,Math.PI*2);ctx.fill();}
        }
        ctx.restore();
        // Active calls keep emitting: steady stream while running, slower while waiting.
        for(const o of ops){const last=comets.findLast?.(c=>c.key===o.tool);const gap=o.status==='waiting'?1500:650;
          if(!last||now-last.born>gap)spawn(o.tool,o.direction==='inbound'?-1:1,capColor(o.tool),o.status==='waiting'?1700:950,false);}
      }else comets.length=0;
      if(showLines)for(const R of geo.routes){
        const b=badges.get(R.key);
        if(R.key===approvalKey){badge(R,'⚠ Needs your approval',stColor('approval'),1);continue;}
        if(!b)continue;
        if(!b.live&&b.until<=now){badges.delete(R.key);continue;}
        const took=b.live?(b.start?Date.now()-b.start:null):b.ms;
        const color=b.status==='done'?stColor('done'):b.status==='failed'?stColor('error'):capColor(R.key);
        badge(R,b.text+(took!=null&&fmt(took)?' · '+fmt(took):''),color,b.live?1:Math.min(1,(b.until-now)/600));
      }
      canvas.dataset.comets=String(comets.length);canvas.dataset.approval=approvalKey||'';
      // Comets need full frame rate; approval breathing / fading glows / live badges only need ~25 fps.
      if(animating(now)){if(comets.length||ops.length)frame=requestAnimationFrame(draw);else slowFrame=setTimeout(kick,40);}
      else if(still&&(glows.size||[...badges.values()].some(b=>b.live)))setTimeout(kick,500);
    }
    motion.addEventListener('change',kick);
    document.addEventListener('visibilitychange',()=>{if(!document.hidden)kick();});
    return {resize,sync,kick,state:()=>({frames,comets:comets.length,approval:approvalKey,ops:ops.map(o=>o.tool),glows:[...glows.keys()],badges:[...badges.entries()].map(([k,b])=>k+':'+b.text)})};
  })();
  window.klyneFlow=flow;
  function measureReactor(){
    if(root.hidden || conversation)return;
    const box=reactor.getBoundingClientRect(),dpr=Math.min(devicePixelRatio||1,1.5);
    dimensions={w:Math.max(1,Math.round(box.width*dpr)),h:Math.max(1,Math.round(box.height*dpr)),dpr};
    const graph=$('prod-graph').getBoundingClientRect(), core=$('prod-core').getBoundingClientRect();
    const svg=$('prod-edges');svg.setAttribute('viewBox',`0 0 ${graph.width} ${graph.height}`);
    const cx=core.left+core.width/2-graph.left,cy=core.top+core.height/2-graph.top,r=Math.min(core.width,core.height)*.35;
    routes=[];
    for(const [key]of capabilities){
      const card=$('prod-cap-'+key);if(!card)continue;const b=card.getBoundingClientRect();if(!b.width)continue;
      const tx=b.left+b.width/2-graph.left,ty=b.top+b.height/2-graph.top,angle=Math.atan2(ty-cy,tx-cx);
      const x1=cx+Math.cos(angle)*r,y1=cy+Math.sin(angle)*r;
      const right=tx>cx,x2=(right?b.left:b.right)-graph.left,y2=ty;
      const path=`M${x1},${y1} C${(x1+x2)/2},${y1} ${(x1+x2)/2},${y2} ${x2},${y2}`;
      routes.push({key,path,angle,x1,y1,c1x:(x1+x2)/2,c1y:y1,c2x:(x1+x2)/2,c2y:y2,x2,y2});
    }
    flow.resize({routes,cx,cy,r,w:graph.width,h:graph.height});
    updateRoutes();
  }
  function updateRoutes(){
    const a=window.klyneActivity.state();
    const svg=$('prod-edges'),keep=new Set();
    function pathNode(id,path,cls){
      keep.add(id);let node=[...svg.children].find(n=>n.dataset.key===id);
      if(!node){node=document.createElementNS('http://www.w3.org/2000/svg','path');node.dataset.key=id;node.setAttribute('pathLength','100');svg.append(node);}
      if(node.getAttribute('d')!==path)node.setAttribute('d',path);
      if(node.getAttribute('class')!==cls)node.setAttribute('class',cls);
    }
    routes.forEach(({key,path})=>{
      const ops=a.connected?a.operations.filter(o=>o.tool===key):[],flashes=a.flashes.filter(f=>f.tool===key);
      pathNode(key,path,`telemetry-route ${ops.length?'lit':''}`);
      for(const o of ops)pathNode('op:'+o.id,path,`energy-packet ${o.status} ${o.direction==='inbound'?'inbound':'outbound'}`);
      for(const flash of flashes)pathNode('result:'+flash.id,path,'energy-packet inbound result');
    });
    for(const node of [...svg.children])if(!keep.has(node.dataset.key))node.remove();
    flow.sync(a,current?.snapshot);
  }
  function drawReactor(now) {
    reactorFrame=0;
    if(document.hidden || root.hidden || conversation || !inViewport)return;
    const state=window.klyneActivity.state(),mode=state.mode;
    if(lastMode!==mode){lastMode=mode;modeSince=now;}
    const age=now-modeSince,working=mode==='working',waiting=mode==='waiting';
    const burst=(mode==='complete'&&age<800)||(mode==='error'&&age<350);
    const calls=state.operations.filter(o=>o.status==='active').length;
    const gpuBoost=(state.gpuPercent||0)/100*.35;
    const targetEnergy=working?Math.min(1,.50+calls*.10+(state.cpuPercent||0)/100*.12+gpuBoost):waiting?Math.min(.85,.42+gpuBoost):.30;
    const dt=Math.min(.08,Math.max(.016,(now-lastSunTime)/1000));lastSunTime=now;
    if(motion.matches){sunEnergy=targetEnergy;sunPointer.strength=0;}
    else {sunEnergy+=(targetEnergy-sunEnergy)*(1-Math.exp(-dt*9));sunPointer.strength+=(sunPointer.target-sunPointer.strength)*(1-Math.exp(-dt*10));}
    if(Math.abs(sunEnergy-targetEnergy)<.002)sunEnergy=targetEnergy;
    if(Math.abs(sunPointer.strength-sunPointer.target)<.002)sunPointer.strength=sunPointer.target;
    // The sun retains a small living flame even at rest; task telemetry adds intensity.
    const alive=!motion.matches;
    if(now-lastReactorFrame>=33 || !alive){
      lastReactorFrame=now;
      const {w,h,dpr}=dimensions;
      if(reactor.width!==w||reactor.height!==h){reactor.width=w;reactor.height=h;}
      brush.clearRect(0,0,w,h);
      const size=Math.min(w,h),cx=w/2,cy=h/2;
      const t=motion.matches?0:now/1000;
      const breath=0;
      const completion=mode==='complete'&&burst?Math.sin(age/800*Math.PI):0;
      const radius=size*(.315+breath-completion*.009);
      const energy=sunEnergy,pull=motion.matches?0:sunPointer.strength;
      const pointerAngle=Math.atan2(sunPointer.y,sunPointer.x);
      const wrap=a=>Math.atan2(Math.sin(a),Math.cos(a));
      const glow=brush.createRadialGradient(cx,cy,radius*.82,cx,cy,size*.5);
      glow.addColorStop(0,'#ff6a0000');
      glow.addColorStop(.34,'rgba(255,91,12,'+(.04+energy*.13)+')');
      glow.addColorStop(1,'#ff6a0000');
      brush.fillStyle=glow;brush.fillRect(0,0,w,h);
      const cell=4*dpr,font=6*dpr;
      brush.font='bold '+font+'px Consolas, monospace';brush.textAlign='center';brush.textBaseline='middle';
      brush.shadowBlur=0;

      const palette=mode==='error'?['#713321','#a14b2f','#d4683c','#ed995f']:['#673018','#ab451a','#ed6c20','#ffab43','#ffe1a1'];
      let glyphCount=0;
      for(let py=cy-size*.49;py<=cy+size*.49;py+=cell){
        for(let px=cx-size*.49;px<=cx+size*.49;px+=cell){
          const dx=px-cx,dy=py-cy,r=Math.hypot(dx,dy),a=Math.atan2(dy,dx);
          if(r<radius*.84 || r>size*.49)continue;
          const toward=Math.exp(-Math.pow(wrap(a-pointerAngle)/.38,2))*pull;
          const height=Math.max(0,(r-radius)/size);
          let flame=0;
          // Anchored tongues taper as they rise. Only their tips bend gently;
          // the base and character grid never orbit or shake.
          for(let j=0;j<11;j++){
            const bearing=j*Math.PI*2/11+.10*Math.sin(j*2.7);
            const length=Math.min(.174,(.135+energy*.065)*(.88+.12*Math.sin(j*4.7))
              *(1+.10*Math.sin(t*.95+j*1.9)));
            const climb=height/Math.max(.001,length);
            if(climb>1)continue;
            const bend=(.20*Math.sin(j*2.3)+.16*Math.sin(t*.85+j*2.3-climb*2.8))*Math.pow(climb,1.35);
            const width=(.25+energy*.045)*(1-climb*.88);
            const distance=wrap(a-bearing-bend);
            const tongue=Math.exp(-Math.pow(distance/width,2))*Math.pow(1-climb,.38);
            flame=Math.max(flame,tongue);
          }
          const shell=Math.max(0,1-Math.abs(r-radius)/(size*.022));
          const pointerFlame=toward*Math.max(0,1-height/(.035+energy*.15));
          const rising=.94+.06*Math.sin(height*65-t*1.2);
          let heat=r>=radius?Math.max(shell*.32,flame*(.43+energy*.75)*rising,pointerFlame*.8):shell*.3;
          if(completion)heat+=completion*.3*shell;
          if(heat<.10)continue;
          heat=Math.min(1,heat*1.25);
          brush.globalAlpha=Math.min(1,(.55+energy*.65)*heat*1.8);
          brush.fillStyle=palette[Math.min(palette.length-1,Math.floor(heat*palette.length))];
          const column=Math.round((px-cx)/cell),row=Math.round((py-cy)/cell);
          const mark=Math.abs((column*73856093)^(row*19349663))%6;
          brush.fillText(r>radius+size*.035?'.,:!|+'[mark]:':;+=*#'[mark],px,py);
          glyphCount++;
        }
      }
      brush.globalAlpha=1;
      reactor.dataset.sunEnergy=energy.toFixed(3);
      reactor.dataset.sunGlyphs=String(glyphCount);
    }
    // Pacing (2026-10-25): full ~30 fps only while thinking/working, during the
    // completion burst, while energy is still settling or the pointer is on the
    // orb. Otherwise a gentle 4 fps idle flicker (no rAF spin). Reduced motion:
    // one static frame per state change. Hidden tab: stopped (visibilitychange).
    if(alive){
      const busy=working||waiting||burst||sunEnergy!==targetEnergy||sunPointer.strength!==sunPointer.target||sunPointer.target>0;
      reactor.dataset.sunPace=busy?'full':'idle';
      if(busy)reactorFrame=requestAnimationFrame(drawReactor);
      else reactorIdle=setTimeout(()=>{reactorIdle=0;reactorFrame=requestAnimationFrame(drawReactor);},250);
    }else reactor.dataset.sunPace='still';
  }
  function stopReactor(){cancelAnimationFrame(reactorFrame);reactorFrame=0;clearTimeout(reactorIdle);reactorIdle=0;}
  // Wake from the idle pace at once when the mode changes (e.g. a new run starts).
  window.klyneActivity.subscribe(()=>{if(reactorIdle&&window.klyneActivity.state().mode!==lastMode){stopReactor();reactorFrame=requestAnimationFrame(drawReactor);}});

  function refreshReactor(){
    stopReactor();reactorFrame=requestAnimationFrame(drawReactor);
    updateRoutes();
  }
  new IntersectionObserver(entries=>{inViewport=entries[0].isIntersecting;root.classList.toggle('prod-offscreen',!inViewport);if(inViewport){measureReactor();refreshReactor();}else stopReactor();}).observe(root);
  document.addEventListener('visibilitychange',()=>{if(document.hidden)stopReactor();else refreshReactor();});
  const continueButton = document.createElement('button');
  continueButton.id = 'prod-continue'; continueButton.type = 'button'; continueButton.hidden = true;
  root.querySelector('.prod-heading').append(continueButton);
  continueButton.onclick = () => {
    if(current?.snapshot?.pending || current?.snapshot?.status==='Needs input') {
      conversation=true;render();
      if(current.snapshot.pending) {$('pending-action').open=true;$('pending-action').scrollIntoView({block:'nearest'});}
      else $('instruction').focus();
    } else $('resume-chat').click();
  };
  $('prod-inspector').querySelector('form').addEventListener('submit',()=>{inspect=null;});
  $('prod-inspector').addEventListener('cancel',()=>{inspect=null;});
  $('prod-surface').insertBefore($('prod-result'),root.querySelector('.prod-pipeline'));
  root.dataset.details='true';
  root.querySelector('.prod-meta').insertBefore(root.querySelector('.prod-pipeline'),$('prod-metrics'));
  root.querySelector('.prod-meta').insertAdjacentHTML('afterend','<section id="ui-callout" role="region" aria-live="polite" hidden></section>');
  $('prod-core-state').insertAdjacentHTML('afterend','<ol id="ui-trail" aria-label="Recent tool calls"></ol>');
  const defaultConversation=false;
  let current = null, selectedKey = null, conversation = defaultConversation, online = true;
  let edgeFrame = 0, animations = [], previousSubmitting = false, inspect = null, viewTransition = null;
  let lastNotice=0;
  function setText(node,text){if(node.textContent!==text){node.textContent=text;if(!motion.matches && !document.hidden)node.animate([{opacity:.3},{opacity:1}],{duration:300});}}
  function markup(element, html) { if(element.dataset.rendered !== html) {element.innerHTML=html;element.dataset.rendered=html;} }
  function actionName(action) {
    if(typeof action==='string') return action;
    if(action?.tool) return action.tool;
    if(action?.action) return actionName(action.action);
    return action && typeof action==='object'?Object.keys(action)[0] || 'Tool':'Tool';
  }
  function category(action) {
    const text = actionName(action);
    if(/^(browser_|fetch_url|FetchUrl|fetch:)/i.test(text)) return 'browser';
    if(/^desktop_/i.test(text)) return 'desktop';
    if(/^(app_|mcp_)/i.test(text)) return 'apps';
    if(/^(skill_|tool_|capability_)/i.test(text)) return 'skills';
    if(/^memory_/i.test(text)) return 'memory';
    if(/^(runtime_|self_improve)/i.test(text)) return 'runtime';
    if(/^(run_shell|RunShell|shell:)/i.test(text)) return 'terminal';
    return 'files';
  }
  function pendingOf(chat) {
    if(!chat?.pending) return null;
    const action=chat.pending.action || chat.pending;
    return {action, agent:chat.pending.agent || chat.tasks?.find(t=>t.status==='Working')?.agent || 'Worker', name:actionName(action), category:category(action)};
  }
  function trailItems(chat,activity){
    const ev=(chat?.evidence||[]).slice(-5).map(e=>({tool:category(e.action),text:(typeof e.action==='string'?e.action:actionName(e.action))+(e.summary?' · '+e.summary:''),ok:String(!!e.ok)}));
    const live=(activity?.operations||[]).map(o=>({tool:o.tool,text:o.operation,ok:'live'}));
    return [...ev,...live].slice(-6);
  }
  function renderTrail(chat,activity){
    const items=trailItems(chat,activity);
    markup($('ui-trail'),items.map(i=>`<li data-ok="${i.ok}" style="--cap:var(--cap-${esc(String(i.tool).replace(/^app:.*/,'apps'))},#888)"><i aria-hidden="true">${i.ok==='live'?'▶':i.ok==='true'?'✓':'✕'}</i><b title="${esc(i.text)}">${esc(i.text)}</b><span class="sr-only">${i.ok==='live'?'running':i.ok==='true'?'succeeded':'failed'}</span></li>`).join(''));
  }
  function describeProposal(p){
    if(!p)return '';
    if(p.kind==='shell')return [p.program,...(p.args||[])].join(' ');
    if(p.kind==='delete')return 'DELETE '+(p.origin||'').replace(/\/$/,'')+(p.path||'');
    if(p.kind==='secret')return 'Use credential '+p.name+' for '+((p.origins||[]).join(', ')||'the command environment');
    if(p.kind==='destructive')return typeof p.action==='object'?JSON.stringify(p.action):String(p.reason||'Destructive action');
    return JSON.stringify(p);
  }
  function renderCallout(chat,ui){
    const box=$('ui-callout');if(!box)return;
    const show=chat&&['approval','input','blocked','error','done'].includes(ui);
    box.hidden=!show;box.dataset.ui=ui;if(!show){markup(box,'');return;}
    const [label,icon]=UI[ui];const msg=lastKlyne(chat);let html='';
    if(ui==='approval'){
      const p=chat.pending.proposal,cmd=describeProposal(p),why=chat.pending.reason||p.reason||(chat.tasks?.find(t=>t.status==='Awaiting approval')?.instruction)||'Klyne needs this to continue the current step.';
      const scopeHint=p.kind==='destructive'?'Allow this kind of action ('+(p.category||'destructive')+') for the rest of this chat. You will still be asked in other chats.':'Allow this exact '+(p.kind==='shell'?'command':'request')+' again in this chat without asking. Anything different still asks; changes to access or the files it uses cancel it.';
      html=`<h3><span aria-hidden="true">${icon}</span>${label}</h3><p>Klyne wants to run this. Nothing has run yet.</p><code>${esc(cmd)}</code>${p.kind==='destructive'?'<p class="ui-warn"><strong>Destructive:</strong> this can delete or overwrite data.</p>':''}<p><strong>Why:</strong> ${esc(why)}</p>${chat.pending.expires_at_ms?`<p>Request expires at ${esc(new Date(chat.pending.expires_at_ms).toLocaleTimeString())}.</p>`:''}<div class="ui-actions"><button class="primary" data-ui-act="approve">✓ Approve once</button><button data-ui-act="approve-chat" title="${esc(scopeHint)}">Approve for this chat</button><button class="danger" data-ui-act="deny">✕ Deny</button><button data-ui-act="edit" title="Copy into the message box so you can change it">✎ Edit</button></div>`;
    } else if(ui==='input'){
      html=`<h3><span aria-hidden="true">${icon}</span>${label}</h3><p>${esc(msg||'Klyne needs more information to continue.')}</p><div class="ui-actions"><button class="primary" data-ui-act="answer">Type your answer</button></div>`;
    } else if(ui==='error'){
      const ep=chat.provider?.endpoint||'the local Ollama address';
      html=`<h3><span aria-hidden="true">${icon}</span>Can’t reach ${esc(({ollama:'Ollama',codex:'Codex',opencode:'OpenCode'})[chat.provider?.kind]||'the AI model')}</h3><p>Klyne couldn’t connect to ${esc(ep)}. Start Ollama (open the Ollama app, or run <b>ollama serve</b>), then press Retry. Your work is saved.</p><div class="ui-actions"><button class="primary" data-ui-act="retry">⟳ Retry</button><button data-ui-act="settings">Connection settings</button></div>`;
    } else if(ui==='blocked'){
      html=`<h3><span aria-hidden="true">${icon}</span>${chat.pending?'Check what happened':label}</h3><p>${esc(chat.pending?'An action may or may not have finished. Check the app or file, then record what happened.':msg||'Work stopped before finishing. Your progress is saved.')}</p>${chat.execution?.failure?.guidance?`<p><strong>Next:</strong> ${esc(chat.execution.failure.guidance)}</p>`:''}<div class="ui-actions"><button class="primary" data-ui-act="${chat.pending?'review':'retry'}">${chat.pending?'Review action':'⟳ Try again'}</button></div>`;
    } else {
      const files=(chat.result?.artifacts||[]).map(a=>a.path||a).filter(Boolean).slice(0,6);
      html=`<h3><span aria-hidden="true">${icon}</span>${label}${chat.result?.outcome==='verified'?' · verified':''}</h3><p>${esc(msg.slice(0,600)||'Finished.')}</p>${files.length?`<ul class="ui-files">${files.map(f=>`<li>${esc(f)}</li>`).join('')}</ul>`:''}<div class="ui-actions"><button data-ui-act="copy">Copy answer</button><button data-ui-act="result">View full result</button></div>`;
    }
    markup(box,html);
  }
  document.addEventListener('click',e=>{
    const b=e.target.closest('[data-ui-act]');if(!b)return;const act=b.dataset.uiAct,chat=current?.snapshot;
    if(act==='approve'||act==='approve-chat'||act==='deny')window.klyneResolve?.(act==='deny'?'abandon':act==='approve'?'approved':'approved_conversation');
    else if(act==='edit'){const t=$('instruction');t.value='Instead, run: '+describeProposal(chat?.pending?.proposal);t.focus();}
    else if(act==='answer'){$('instruction').focus();}
    else if(act==='retry')$('resume-chat').click();
    else if(act==='settings')$('settings-button').click();
    else if(act==='review')continueButton.click();
    else if(act==='copy'){navigator.clipboard?.writeText(lastKlyne(chat)).catch(()=>{});b.textContent='Copied ✓';}
    else if(act==='result')showInspector('result:all');
  });
  function cancelMotion() {
    cancelAnimationFrame(edgeFrame);
    viewTransition?.skipTransition(); viewTransition=null;
    for(const animation of animations) animation.cancel(); animations=[];
  }
  function morphView() {
    cancelMotion();
    if(motion.matches || document.hidden) {render();return;}
    const settle=()=>{render();fit();edges();};
    if(document.startViewTransition) {
      const transition=document.startViewTransition(settle);
      viewTransition=transition;
      transition.ready.catch(()=>{});
      transition.updateCallbackDone.catch(()=>{});
      transition.finished.catch(()=>{}).finally(()=>{if(viewTransition===transition)viewTransition=null;});
      return;
    }
    // Older browsers animate the real composer between its two layout positions.
    const composer=$('chat-form'), before=composer.getBoundingClientRect();
    settle();
    const after=composer.getBoundingClientRect();
    if(before.width && before.height && after.width && after.height) {
      const animation=composer.animate([
        {transformOrigin:'top left',transform:`translate(${before.left-after.left}px,${before.top-after.top}px) scale(${before.width/after.width},${before.height/after.height})`},
        {transformOrigin:'top left',transform:'none'}
      ],{duration:480,easing:'cubic-bezier(.22,1,.36,1)'});
      animations.push(animation);
    }
    const surface=conversation?$('conversation'):$('prod-surface');
    animations.push(surface.animate([{opacity:0,transform:'translateY(8px)'},{opacity:1,transform:'none'}],{duration:320,easing:'ease-out'}));
  }
  function update(input) {
    const newSelection=input.selected!==selectedKey;
    const restoring=newSelection || !current?.snapshot;
    if(newSelection && !previousSubmitting) { cancelMotion();  conversation=defaultConversation; inspect=null;  $('prod-inspector').close(); }
    if(newSelection)graphDirty=true;
    selectedKey=input.selected; current=input;
    window.klyneActivity.snapshot(input);
    if(restoring){lastMode=window.klyneActivity.state().mode;modeSince=performance.now()-1000;}
    document.documentElement.dataset.klyneMode=window.klyneActivity.state().mode;
    const visible=!!input.selected || input.submitting;
    root.hidden=!visible;
    if(!visible) {dockComposer(false);$('prod-inspector').close();cancelMotion();conversation=defaultConversation;document.documentElement.classList.remove('production-on','production-busy');previousSubmitting=false;return;}
    const starting=input.submitting && !previousSubmitting;
    previousSubmitting=input.submitting;
    if(starting) {conversation=false;morphView();} else {if(newSelection && busy.has(input.snapshot?.status))conversation=false;render();}
  }
  function render() {
    if(!current || root.hidden) return;
    const chat=current.snapshot;
    dockComposer(!conversation);updateSideChat(chat);
    const status=current.submitting?'Receiving':chat?.status || 'Opening';
    const active=busy.has(status) || ['Receiving','Opening'].includes(status);
    const activity=window.klyneActivity.state();
    root.dataset.mode=activity.mode;
    const ui=uiState(chat,activity,current.submitting);document.documentElement.dataset.ui=ui;renderCallout(chat,ui);renderTrail(chat,activity);
    document.documentElement.dataset.klyneMode=activity.mode;

    continueButton.hidden=conversation || active || !chat || status==='Completed' || !$('resume-chat');
    continueButton.disabled=!!current.submitting;
    continueButton.textContent=chat?.pending?.proposal?'Review action':chat?.pending?'Resolve action':status==='Needs input'?'Answer question':'Resume work';
    const pending=pendingOf(chat);
    const inTool=active && activity.operations.length>0;
    const waiting=active && activity.mode==='waiting';
    const tasks=chat?.tasks || [], evidence=chat?.evidence || [];
    const access=chat?.access || current.access || {};
    const provider=chat?.activity?.provider || chat?.provider || current.provider || {};
    const lastUser=chat?.messages?.filter(m=>m.role==='user').at(-1);
    document.documentElement.classList.toggle('production-on',!conversation);
    document.documentElement.classList.toggle('production-busy',active && !conversation);
    const previousStatus=root.dataset.status;
    if(previousStatus && previousStatus!==status && !activity.replaying && !motion.matches && !document.hidden){
      if(status==='Completed')$('prod-progress').animate([{filter:'brightness(2)'},{filter:'brightness(1)'}],{duration:650});
      if(activity.mode==='error')$('prod-reactor').animate([{transform:'translateX(0)'},{transform:'translateX(-2px)'},{transform:'translateX(2px)'},{transform:'translateX(0)'}],{duration:250});
    }
    root.dataset.status=status; root.dataset.live=String(active && online); root.dataset.view=conversation?'conversation':'production';
    $('prod-surface').hidden=conversation;
    $('prod-view-toggle').textContent=conversation?'Activity ↗':'Conversation ↗';
    $('prod-view-toggle').setAttribute('aria-pressed',String(conversation));
    const labels={Receiving:'Receiving your instruction',Opening:'Opening saved work',Planning:'Understanding your request',Working:inTool?'Putting tools to work':'Executing the plan',Reviewing:'Checking the result',Completed:'Ready for you','Needs input':'Your input is needed',Blocked:'Work needs attention',Interrupted:'Paused · waiting on you',Stopping:'Stopping current work',Stopped:'Work is stopped',Upgrading:'Handing off the runtime'};
    const tools=activity.operations.map(o=>o.tool);
    const headline=activity.mode==='input'?'Your input is needed':status==='Reviewing'?'Checking the result':activity.mode==='waiting'?'Awaiting response':inTool?(tools.length>1?'Using tools':({browser:'Browsing',files:'Working with files',terminal:'Executing command',desktop:'Controlling desktop',memory:'Using memory',apps:'Calling app API',skills:'Invoking tools',runtime:'Executing runtime'})[tools[0]] || 'Using '+tools[0].replace(/^app:/,'')):labels[status] || status;
    setText($('prod-title'),headline);
    root.querySelector('.prod-pipeline').setAttribute('aria-label','Workflow stage: '+status);
    const operationText=activity.operations.map(o=>o.operation).join(' · ');
    const phase=chat?.activity?.phase,since=chat?.activity?.started_at?Math.max(0,Math.round((Date.now()-Number(chat.activity.started_at))/1000)):0,who=chat?.activity?.agent || chat?.activity?.role || 'model';
    const phaseText=phase==='loading_model'?`Loading the model into memory for the ${who} (${since}s; the first request can take up to a minute)`:phase==='thinking'?`The ${who} model is thinking (${since}s)`:phase==='generating'?`The ${who} model is writing a reply (${chat.activity.generated_chars||0} characters, ${since}s)`:'';
    const live=online?(waiting?(operationText || phaseText || `Awaiting ${who} response`):inTool?operationText:active?status:UI[uiState(chat,activity,false)][0]):'Connection lost to Studio · showing last known state';
    if($('prod-live').textContent!==live) $('prod-live').textContent=live;
    const doneSteps=tasks.filter(t=>t.status==='Done').length;
    $('prod-metrics').textContent=tasks.length?`${doneSteps} of ${tasks.length} action steps${status!=='Completed' && doneSteps===tasks.length?' · final review pending':''}`: 'Preparing';
    const progress=$('prod-progress');
    progress.max=Math.max(1,tasks.length);
    if(tasks.length || status==='Completed')progress.value=status==='Completed'?progress.max:doneSteps;else if(active)progress.removeAttribute('value');else progress.value=0;
    progress.dataset.waiting=String(activity.mode==='waiting');
    $('prod-goal').textContent=current.submitting?current.message:chat?.execution?.original_request || lastUser?.text || chat?.title || 'Loading the saved instruction…';
    const stages=['Planning','Working','Reviewing','Completed'];
    const recordedStage=[...(chat?.activity_events||[])].reverse().find(e=>e.type==='workflow:stage_changed' && stages.includes(e.data?.status))?.data.status;
    const index=stages.indexOf(stages.includes(status)?status:recordedStage);
    if(index>=0)root.dataset.stageIndex=String(index);
    document.querySelectorAll('[data-stage]').forEach(node=>{const n=stages.indexOf(node.dataset.stage),previous=node.dataset.state;node.dataset.state=n===index && status!=='Completed'?'active':index>=0 && n<=index?'done':'waiting';
      if(previous && previous!==node.dataset.state && node.dataset.state==='active' && !motion.matches)node.animate([{opacity:.4},{opacity:1}],{duration:350});
    });
    $('prod-core-state').textContent=({working:'WORKING',waiting:'WAITING',input:'NEEDS YOUR INPUT',complete:'COMPLETE',error:'NEEDS ATTENTION',idle:'READY',disconnected:'DISCONNECTED'})[activity.mode];
    if(status==='Stopped')$('prod-core-state').textContent='STOPPED';
    if(waiting)$('prod-core-state').textContent=operationText?('WAITING - '+operationText):('WAITING - '+(chat?.activity?.role==='reviewer'?'Reviewer response':'Model response'));
    {const u=uiState(chat,activity,current.submitting);const step=operationText||(waiting?(chat?.activity?.role==='reviewer'?'Reviewer is checking':'Model is thinking'):'');$('prod-core-state').dataset.icon=UI[u][1];$('prod-core-state').textContent=(UI[u][0]+(step&&['thinking','acting'].includes(u)?' · '+step:'')).toUpperCase();}
    $('prod-core').dataset.active=String(active && online);
    $('prod-stop').hidden=!active;
    $('prod-health-connection').textContent=online?'Studio online':'Studio offline';
    $('prod-health-connection').dataset.online=String(online);
    $('prod-health-tools').textContent=String(new Set(activity.operations.map(o=>o.tool)).size);
    $('prod-health-evidence').textContent=String(evidence.length);
    $('prod-provider').textContent=({codex:'Codex',ollama:'Ollama',opencode:'OpenCode'})[provider.kind] || 'Model connection';
    $('prod-model').textContent=provider.model || 'Provider default model';
    $('prod-request').textContent=!online?'Waiting for Studio to reconnect':operationText|| (waiting?`Awaiting ${chat?.activity?.role || 'model'} response`:status==='Completed'?'Response received and reviewed':active?'Processing task state':status==='Needs input'?'Your input is needed to continue':'No operation in progress');
    $('prod-task-count').textContent=tasks.length?`${tasks.length}`:'Awaiting plan';
    markup($('prod-workers'),tasks.length?tasks.map((task,i)=>`<button class="prod-worker" id="prod-worker-${i}" data-inspect="task:${i}" data-state="${esc(task.status)}"><span class="prod-worker-icon">${task.status==='Done'?'✓':task.status==='Working'?'◉':String(i+1).padStart(2,'0')}</span><span><strong>${esc(task.agent)}</strong><small>${esc(task.instruction)}</small><small>Step ${i+1} ${task.depends_on?.length?`- after ${task.depends_on.join(", ")}`:"- no dependencies"}</small>${task.expected_result?`<small>Expected: ${esc(task.expected_result)}</small>`:""}</span><em>${esc(task.status==='Pending'?'Queued':task.status)}</em></button>`).join(''):`<div class="prod-awaiting"><span>⌁</span><strong>${status==='Planning'||status==='Receiving'?'A plan is taking shape':'No assignments recorded'}</strong><p>Worker roles appear here when the planner assigns them.</p></div>`);
    $('prod-review-node').dataset.state=status==='Reviewing'?'Working':status==='Completed'?'Done':'Pending';
    for(const [key,label,icon,grant] of capabilities) {
      let card=$('prod-cap-'+key);
      if(!card){card=document.createElement('button');card.id='prod-cap-'+key;card.className='prod-capability';card.dataset.inspect='cap:'+key;card.innerHTML=`<span>${icon}</span><strong>${label}</strong><small></small>`;$('prod-capabilities').append(card);}
      const ops=activity.operations.filter(o=>o.tool===key), flash=activity.flashes.filter(f=>f.tool===key).at(-1);
      const position=capabilities.findIndex(c=>c[0]===key);
      card.style.gridColumn=String(position%2===0?1:3);
      card.style.gridRow=String(Math.floor(position/2)+1);
      card.dataset.connectedApp=String(key.startsWith('app:'));
      const enabled=!grant || !!access[grant];
      const failed=activity.failures.some(f=>f.tool===key);
      const state=ops.length?(ops.every(o=>o.status==='waiting')?'waiting':'active'):flash?.status|| (failed?'failed':enabled?'ready':'disabled');
      const transition=card.dataset.state!==state;
      card.dataset.state=state;
      card.dataset.feedback=flash?'true':'false';
      if(transition && flash && !motion.matches)card.animate([{boxShadow:'inset 0 0 14px '+(state==='failed'?'#dc573950':'#ffc17c40')},{boxShadow:'none'}],{duration:state==='failed'?300:600});
      card.querySelector('small').textContent=ops.length?ops.map(o=>o.operation).join(' · '):flash?flash.status==='failed'?'Operation failed':'✓ Result received':failed?'Operation failed':enabled?'Available':'Access off';
      card.title=ops.length?ops.map(o=>`${o.direction==='inbound'?'Result from':'Request to'} ${label}: ${o.operation}`).join('\n'):label+' · '+card.querySelector('small').textContent;
    }
    $('prod-open-log').textContent='Observations ('+evidence.length+') \u2197';
    $('prod-open-steps').textContent='Steps '+doneSteps+'/'+tasks.length;
    $('prod-observation-count').textContent=evidence.length?`${evidence.length} recorded`:'Waiting for evidence';
    markup($('prod-events'),evidence.length?evidence.slice(-8).reverse().map((event,i)=>`<button class="prod-event" data-inspect="event:${evidence.length-1-i}" data-ok="${!!event.ok}"><span>${event.ok?'✓':'!'}</span><span><strong>${esc(event.action)}</strong><small>${esc(event.agent)} · ${esc(event.summary)}</small></span><span>${event.ok?'Recorded':'Needs review'}</span></button>`).join(''):'<p class="prod-empty">Live activity will appear as tools finish. Give Klyne a goal to begin.</p>');
    const terminal=!active && !!chat;
    $('prod-result').hidden=true;
    noticeButton.hidden=!terminal;noticeButton.textContent=status==='Completed'?'View result':'! '+(labels[status] || status)+' - Details'; $('prod-open-result').hidden=!terminal;
    if(terminal) {
      $('prod-result-icon').textContent=status==='Completed'?'✓':'!';
      $('prod-result-title').textContent=status==='Completed'?(chat.result?.outcome==='verified'?'Klyne - Verified':chat.result?.outcome==='reviewed'?'Klyne - Reviewed':'Klyne'):labels[status] || status;
      if(status==='Completed'&&chat.result?.outcome==='verified'&&chat.contract?.criteria?.some(c=>c.ArtifactRequirements))$('prod-result-title').textContent='Klyne - Report requirements verified';
      $('prod-result-text').textContent=chat.messages?.filter(m=>m.agent==='Klyne').at(-1)?.text || 'Progress is saved. Use the controls below to continue.';
      if(chat.execution?.failure?.guidance)$('prod-result-text').textContent+='\n\nNext: '+chat.execution.failure.guidance;
    }
    $('prod-graph').dataset.layout='flow';
    const rows=Math.ceil(capabilities.length/2);
    $('prod-graph').style.setProperty('--tool-rows',rows);
    $('prod-graph').dataset.overflow=String(rows>6);
    $('prod-model-panel').style.gridRow='1 / '+(rows+1);
    scheduleMorph();
    if(inspect) showInspector(inspect,false);
    if(graphDirty)scheduleEdges();
    requestAnimationFrame(fit);
    refreshReactor();
    if(activity.notice && activity.notice!==lastNotice){
      const animate=activity.notice-lastNotice>300;lastNotice=activity.notice;
      if(animate && !motion.matches && !document.hidden)$('prod-open-log').animate([{color:'#ffd092',transform:'translateY(2px)'},{color:'#b7a99a',transform:'none'}],{duration:300});
    }
  }
  let morphFrame=0,layoutRects=new Map(),layoutAnimations=new Map(),layoutGeneration=0;
  function scheduleMorph(){cancelAnimationFrame(morphFrame);morphFrame=requestAnimationFrame(morphLayout);}
  function morphLayout(){
    morphFrame=0;
    if(root.hidden || conversation || document.hidden){layoutRects.clear();return;}
    const nodes=[...root.querySelectorAll('.prod-capability:not([data-morph-exit])'),$('prod-model-panel')];
    const measurements=nodes.map(node=>{
      const running=layoutAnimations.get(node),visible=running?node.getBoundingClientRect():null;
      running?.cancel();layoutAnimations.delete(node);
      return {node,before:visible||layoutRects.get(node),after:node.getBoundingClientRect()};
    });
    const generation=++layoutGeneration,pending=[];
    for(const {node,before,after}of measurements){
      layoutRects.set(node,after);
      if(motion.matches || !after.width || !after.height)continue;
      if(before && Math.abs(before.x-after.x)+Math.abs(before.y-after.y)+Math.abs(before.width-after.width)+Math.abs(before.height-after.height)<1)continue;
      const from=before?{transformOrigin:'0 0',transform:'translate('+(before.x-after.x)+'px,'+(before.y-after.y)+'px) scale('+(before.width/after.width)+','+(before.height/after.height)+')'}:{opacity:0,transform:'translateY(8px) scale(.98)'};
      const animation=node.animate([from,{opacity:1,transformOrigin:'0 0',transform:'none'}],{duration:380,easing:'cubic-bezier(.22,1,.36,1)'});
      layoutAnimations.set(node,animation);pending.push(animation.finished.catch(()=>{}));
    }
    for(const node of layoutRects.keys())if(!node.isConnected)layoutRects.delete(node);
    if(pending.length){
      root.classList.add('layout-morphing');
      Promise.all(pending).then(()=>{if(generation!==layoutGeneration)return;layoutAnimations.clear();root.classList.remove('layout-morphing');scheduleEdges();});
    }else {root.classList.remove('layout-morphing');scheduleEdges();}
  }
  function scheduleEdges() {cancelAnimationFrame(edgeFrame);edgeFrame=requestAnimationFrame(edges);}
  function fit() {
    if(root.hidden || conversation) {root.dataset.compact='false';return;}
    root.dataset.compact='false';
  }

  function edges() { graphDirty=false;measureReactor(); refreshReactor(); }
  function showInspector(key,open=true) {
    inspect=key;const [kind,...parts]=key.split(':'),id=parts.join(':'),chat=current?.snapshot;let title='',data;
    if(kind==='log') {data=(chat?.evidence || []).map(e=>(e.ok?'\u2713 ':'! ')+(e.action||'Observation')+'\n'+(e.summary||'')).join('\n\n') || 'No observations recorded yet.';title='Observations';}
    if(kind==='result') {data=chat?.messages?.filter(m=>m.agent==='Klyne').at(-1)?.text || 'Progress is saved.';
      if(chat?.result?.checks?.length)data+='\n\nAcceptance checks:\n'+chat.result.checks.map(c=>`${c.outcome||(c.passed?'passed':'failed')}: ${c.evidence}`).join('\n');
      if(chat?.execution?.failure?.guidance)data+='\n\nNext: '+chat.execution.failure.guidance;
      title=chat?.status==='Completed'?'Result':'Work needs attention';}
    if(kind==='steps'){data=chat?.tasks || [];title='Steps';}
    if(kind==='task') {data=chat?.tasks?.[Number(id)];title='Assignment';}
    if(kind==='event') {data=chat?.evidence?.[Number(id)];title='Recorded observation';}
    if(kind==='core'){title='Klyne activity';data=window.klyneActivity.state();}
    if(kind==='cap') {const cap=capabilities.find(c=>c[0]===id);title=cap?.[1] || 'Capability';data={access:!cap?.[3] || !!(chat?.access || current?.access)?.[cap[3]],observations:(chat?.evidence || []).filter(e=>window.klyneActivity.category(e.action)===id),pending:pendingOf(chat)?.category===id?pendingOf(chat).action:null,connection:connectedApps.find(a=>'app:'+a.name===id)};}
    $('prod-inspector-title').textContent=title;
    $('prod-inspector-body').textContent=typeof data==='string'?data:JSON.stringify(data || {},null,2);
    if(open && !$('prod-inspector').open) $('prod-inspector').showModal();
  }
  $('prod-core').role='button';$('prod-core').tabIndex=0;$('prod-core').setAttribute('aria-label','Inspect Klyne activity');$('prod-core').onclick=()=>showInspector('core:all');$('prod-core').onkeydown=e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();showInspector('core:all');}};
  $('prod-open-log').onclick=()=>showInspector('log:all');
  $('prod-open-result').onclick=()=>showInspector('result:all');
  root.addEventListener('click',event=>{const node=event.target.closest('[data-inspect]');if(node)showInspector(node.dataset.inspect);});
  $('prod-view-toggle').onclick=()=>{$('prod-inspector').close();inspect=null;conversation=!conversation;morphView();};
  new ResizeObserver(()=>{graphDirty=true;scheduleMorph();requestAnimationFrame(fit);}).observe($('prod-graph'));
  new ResizeObserver(()=>{scheduleMorph();requestAnimationFrame(fit);}).observe($('main'));
  window.addEventListener('resize',()=>{
    // Desktop rectangles must not become animation origins in a mobile layout.
    // Their translated/scaled cards can otherwise extend beyond the viewport.
    ++layoutGeneration;cancelAnimationFrame(morphFrame);
    for(const animation of layoutAnimations.values())animation.cancel();
    layoutAnimations.clear();layoutRects.clear();root.classList.remove('layout-morphing');
    graphDirty=true;cancelMotion();render();
  });
  motion.addEventListener('change',()=>{for(const a of layoutAnimations.values())a.cancel();layoutAnimations.clear();cancelMotion();render();});
  document.documentElement.dataset.pageHidden=String(document.hidden);
  document.addEventListener('visibilitychange',()=>{document.documentElement.dataset.pageHidden=String(document.hidden);root.classList.toggle('prod-paused',document.hidden);if(document.hidden){for(const a of layoutAnimations.values())a.cancel();layoutAnimations.clear();layoutRects.clear();cancelMotion();clearTimeout(appTimer);}else {refreshApps();render();}});
  window.productionView={update,connection(value){if(online!==value){online=value;window.klyneActivity.emit('system:connection_changed',{connected:value});if(current)window.klyneActivity.snapshot(current);render();}}};
  window.addEventListener('klyne-connections',event=>syncApps(event.detail));
  refreshApps();
  let activityFrame=0;
  window.klyneActivity.subscribe(()=>{cancelAnimationFrame(activityFrame);activityFrame=requestAnimationFrame(()=>{if(current)render();});});
})();
