'use strict';
(() => {
  const $ = id => document.getElementById(id);
  const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  const busy = new Set(['Planning','Working','Reviewing','Stopping','Upgrading']);
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
  root.querySelector('.prod-assignment-panel').insertAdjacentHTML('afterbegin','<section class="prod-health"><p class="prod-eyebrow">SYSTEM PULSE</p><div class="prod-gauge"><strong id="prod-cpu">—</strong><span>CPU</span></div><div class="prod-gauge" title="Whole NVIDIA GPU usage, including other apps"><strong id="prod-gpu">?</strong><span>GPU</span></div><dl><dt>Connection</dt><dd id="prod-health-connection">Connected</dd><dt>Tools in use</dt><dd id="prod-health-tools">0</dd><dt>Observations</dt><dd id="prod-health-evidence">0</dd></dl><p class="prod-health-note">Live process measurement</p></section>');
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
    const users=(chat?.messages||[]).filter(m=>m.role==='user');
    markup($('prod-input-history'),users.map(m=>'<p>'+esc(m.text)+'</p>').join(''));
    const answers=(chat?.messages||[]).filter(m=>m.role!=='user'&&m.agent==='Klyne');
    const output=$('prod-output-content'),nearEnd=output.scrollHeight-output.scrollTop-output.clientHeight<50;
    markup(output,answers.length?answers.map(m=>'<article>'+esc(m.text)+'</article>').join(''):'<p class="prod-output-empty">Klyne’s response will appear here.</p>');
    if(nearEnd)output.scrollTop=output.scrollHeight;
  }
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
  let reactorFrame=0,lastReactorFrame=0,inViewport=true,modeSince=0,lastMode='idle';
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
      routes.push({key,path,angle});
    }
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
    if(alive)reactorFrame=requestAnimationFrame(drawReactor);
  }

  function refreshReactor(){
    cancelAnimationFrame(reactorFrame);reactorFrame=requestAnimationFrame(drawReactor);
    updateRoutes();
  }
  new IntersectionObserver(entries=>{inViewport=entries[0].isIntersecting;root.classList.toggle('prod-offscreen',!inViewport);if(inViewport){measureReactor();refreshReactor();}else cancelAnimationFrame(reactorFrame);}).observe(root);
  document.addEventListener('visibilitychange',()=>{if(document.hidden)cancelAnimationFrame(reactorFrame);else refreshReactor();});
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
    const labels={Receiving:'Receiving your instruction',Opening:'Opening saved work',Planning:'Understanding your request',Working:inTool?'Putting tools to work':'Executing the plan',Reviewing:'Checking the result',Completed:'Ready for you','Needs input':'Your input is needed',Blocked:'Work needs attention',Interrupted:'Saved work · interrupted',Stopping:'Stopping current work',Stopped:'Work is stopped',Upgrading:'Handing off the runtime'};
    const tools=activity.operations.map(o=>o.tool);
    const headline=activity.mode==='input'?'Your input is needed':status==='Reviewing'?'Checking the result':activity.mode==='waiting'?'Awaiting response':inTool?(tools.length>1?'Using tools':({browser:'Browsing',files:'Working with files',terminal:'Executing command',desktop:'Controlling desktop',memory:'Using memory',apps:'Calling app API',skills:'Invoking tools',runtime:'Executing runtime'})[tools[0]] || 'Using '+tools[0].replace(/^app:/,'')):labels[status] || status;
    setText($('prod-title'),headline);
    root.querySelector('.prod-pipeline').setAttribute('aria-label','Workflow stage: '+status);
    const operationText=activity.operations.map(o=>o.operation).join(' · ');
    const live=online?(waiting?(operationText || `Awaiting ${chat?.activity?.agent || chat?.activity?.role || 'model'} response`):inTool?operationText:status):'Connection lost · showing last known state';
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
    $('prod-core').dataset.active=String(active && online);
    $('prod-stop').hidden=!active;
    $('prod-health-connection').textContent=online?'Connected':'Disconnected';
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
