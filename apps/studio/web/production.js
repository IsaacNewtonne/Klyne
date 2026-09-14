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
  const root = document.createElement('section');
  root.id = 'production'; root.hidden = true;
  root.setAttribute('aria-label','Production workspace');
  root.innerHTML = `<div class="prod-heading"><div><p class="prod-eyebrow">KLYNE / WORKSPACE</p><h2 id="prod-title">Putting your intent in motion</h2></div><button id="prod-details" type="button" aria-pressed="false">Details</button><button id="prod-view-toggle" type="button" aria-pressed="false">Conversation ↗</button></div>
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
  $('prod-details').onclick=()=>{const expanded=root.dataset.details!=='true';root.dataset.details=String(expanded);$('prod-details').setAttribute('aria-pressed',String(expanded));scheduleEdges();};
  const defaultConversation=!!$('resume-chat');
  let current = null, selectedKey = null, conversation = defaultConversation, online = true;
  let edgeFrame = 0, animations = [], previousSubmitting = false, inspect = null, viewTransition = null;
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
    if(newSelection && !previousSubmitting) { cancelMotion();  conversation=defaultConversation; inspect=null;  $('prod-inspector').close(); }
    selectedKey=input.selected; current=input;
    const visible=!!input.selected || input.submitting;
    root.hidden=!visible;
    if(!visible) {$('prod-inspector').close();cancelMotion();conversation=defaultConversation;document.documentElement.classList.remove('production-on','production-busy');previousSubmitting=false;return;}
    const starting=input.submitting && !previousSubmitting;
    previousSubmitting=input.submitting;
    if(starting) {conversation=defaultConversation;morphView();} else render();
  }
  function render() {
    if(!current || root.hidden) return;
    const chat=current.snapshot;
    const status=current.submitting?'Receiving':chat?.status || 'Opening';
    const active=busy.has(status) || ['Receiving','Opening'].includes(status);
    $('prod-details').hidden=conversation;
    continueButton.hidden=conversation || active || !chat || status==='Completed' || !$('resume-chat');
    continueButton.disabled=!!current.submitting;
    continueButton.textContent=chat?.pending?.proposal?'Review action':chat?.pending?'Resolve action':status==='Needs input'?'Answer question':'Resume work';
    const pending=pendingOf(chat);
    const inTool=active && pending;
    const waiting=active && chat?.activity?.kind==='model' && !pending;
    const tasks=chat?.tasks || [], evidence=chat?.evidence || [];
    const access=chat?.access || current.access || {};
    const provider=chat?.provider || current.provider || {};
    const lastUser=chat?.messages?.filter(m=>m.role==='user').at(-1);
    document.documentElement.classList.toggle('production-on',!conversation);
    document.documentElement.classList.toggle('production-busy',active && !conversation);
    root.dataset.status=status; root.dataset.live=String(active && online); root.dataset.view=conversation?'conversation':'production';
    $('prod-surface').hidden=conversation;
    $('prod-view-toggle').textContent=conversation?'Activity ↗':'Conversation ↗';
    $('prod-view-toggle').setAttribute('aria-pressed',String(conversation));
    const labels={Receiving:'Receiving your instruction',Opening:'Opening saved work',Planning:'Understanding your request',Working:inTool?'Putting tools to work':'Executing the plan',Reviewing:'Checking the result',Completed:'Ready for you','Needs input':'Your input is needed',Blocked:'Work needs attention',Interrupted:'Saved work · interrupted',Stopping:'Stopping current work',Stopped:'Work is stopped',Upgrading:'Handing off the runtime'};
    $('prod-title').textContent=labels[status] || status;
    const live=online?(waiting?`Thinking · ${chat.activity.agent || chat.activity.role}`:inTool?`Using ${pending.name}`:status):'Connection lost · showing last known state';
    if($('prod-live').textContent!==live) $('prod-live').textContent=live;
    const doneSteps=tasks.filter(t=>t.status==='Done').length;
    $('prod-metrics').textContent=tasks.length?`${doneSteps} of ${tasks.length} action steps${status!=='Completed' && doneSteps===tasks.length?' · final review pending':''}`: 'Preparing';
    $('prod-progress').max=Math.max(1,tasks.length+1);$('prod-progress').value=doneSteps+(status==='Completed'?1:0);
    $('prod-goal').textContent=current.submitting?current.message:chat?.execution?.original_request || lastUser?.text || chat?.title || 'Loading the saved instruction…';
    const stages=['Planning','Working','Reviewing','Completed'];
    const index=stages.indexOf(status);
    document.querySelectorAll('[data-stage]').forEach(node=>{const n=stages.indexOf(node.dataset.stage);node.dataset.state=n===index?'active':index>=0 && n<index?'done':'waiting';});
    $('prod-core-state').textContent=!online?'Disconnected':waiting?'Thinking':inTool?'Acting':({Planning:'Planning',Working:'Working',Reviewing:'Reviewing',Completed:'Complete',Receiving:'Connecting',Opening:'Opening',Upgrading:'Upgrading'})[status] || 'On hold';
    $('prod-core').dataset.active=String(active && online);
    $('prod-provider').textContent=({codex:'Codex',ollama:'Ollama',opencode:'OpenCode'})[provider.kind] || 'Model connection';
    $('prod-model').textContent=provider.model || 'Provider default model';
    $('prod-request').textContent=!online?'Waiting for Studio to reconnect':waiting?`${chat.activity.role} request · ${Math.max(0,Math.floor((Date.now()-Number(chat.activity.started_at))/1000))}s awaiting response`:inTool?pending.name:status==='Completed'?'Response received and reviewed':active?'Waiting for the next runtime update':'No model request in progress';
    $('prod-task-count').textContent=tasks.length?`${tasks.length}`:'Awaiting plan';
    markup($('prod-workers'),tasks.length?tasks.map((task,i)=>`<button class="prod-worker" id="prod-worker-${i}" data-inspect="task:${i}" data-state="${esc(task.status)}"><span class="prod-worker-icon">${task.status==='Done'?'✓':task.status==='Working'?'◉':String(i+1).padStart(2,'0')}</span><span><strong>${esc(task.agent)}</strong><small>${esc(task.instruction)}</small><small>Step ${i+1} ${task.depends_on?.length?`- after ${task.depends_on.join(", ")}`:"- no dependencies"}</small>${task.expected_result?`<small>Expected: ${esc(task.expected_result)}</small>`:""}</span><em>${esc(task.status==='Pending'?'Queued':task.status)}</em></button>`).join(''):`<div class="prod-awaiting"><span>⌁</span><strong>${status==='Planning'||status==='Receiving'?'A plan is taking shape':'No assignments recorded'}</strong><p>Worker roles appear here when the planner assigns them.</p></div>`);
    $('prod-review-node').dataset.state=status==='Reviewing'?'Working':status==='Completed'?'Done':'Pending';
    markup($('prod-capabilities'),capabilities.map(([key,label,icon,grant])=>{
      const observed=evidence.filter(e=>category(e.action)===key);
      const enabled=!grant || !!access[grant];
      const state=inTool && pending.category===key?'active':observed.length?(observed.at(-1).ok?'used':'failed'):enabled?'ready':'disabled';
      return `<button id="prod-cap-${key}" class="prod-capability" data-inspect="cap:${key}" data-state="${state}"><span>${icon}</span><strong>${label}</strong><small>${state==='active'?'In use':observed.length?`${observed.length} observed`:enabled?'Available':'Access off'}</small></button>`;
    }).join(''));
    $('prod-observation-count').textContent=evidence.length?`${evidence.length} recorded`:'Waiting for evidence';
    markup($('prod-events'),evidence.length?evidence.slice(-4).reverse().map((event,i)=>`<button class="prod-event" data-inspect="event:${evidence.length-1-i}" data-ok="${!!event.ok}"><span>${event.ok?'✓':'!'}</span><span><strong>${esc(event.action)}</strong><small>${esc(event.agent)} · ${esc(event.summary)}</small></span><span>↗</span></button>`).join(''):'<p class="prod-empty">Tool calls and checks will appear here as they finish.</p>');
    const terminal=!active && !!chat;
    $('prod-result').hidden=!terminal; $('prod-open-result').hidden=!terminal;
    if(terminal) {
      $('prod-result-icon').textContent=status==='Completed'?'✓':'!';
      $('prod-result-title').textContent=status==='Completed'?(chat.result?.outcome==='verified'?'Klyne - Verified':chat.result?.outcome==='reviewed'?'Klyne - Reviewed':'Klyne'):labels[status] || status;
      $('prod-result-text').textContent=chat.messages?.filter(m=>m.agent==='Klyne').at(-1)?.text || 'Progress is saved. Use the controls below to continue.';
      if(chat.execution?.failure?.guidance)$('prod-result-text').textContent+='\n\nNext: '+chat.execution.failure.guidance;
    }
    $('prod-graph').dataset.layout='flow';
    if(inspect) showInspector(inspect,false);
    scheduleEdges();
    requestAnimationFrame(fit);
  }
  function scheduleEdges() {cancelAnimationFrame(edgeFrame);edgeFrame=requestAnimationFrame(edges);}
  function fit() {
    if(root.hidden || conversation) {root.dataset.compact='false';return;}
    const surface=$('prod-surface');
    if(!surface) return;
    const avail=surface.getBoundingClientRect().height;
    if(!avail) return;
    let want=0;
    for(const child of surface.children) {
      if(child.hidden) continue;
      const r=child.getBoundingClientRect();
      const cs=getComputedStyle(child);
      want+=r.height+parseFloat(cs.marginTop||0)+parseFloat(cs.marginBottom||0);
    }
    const over=want-avail>2;
    if(over && root.dataset.compact!=='true') root.dataset.compact='true';
    else if(!over && root.dataset.compact==='true') {
      root.dataset.compact='false';
      requestAnimationFrame(()=>{
        if(root.hidden || conversation) return;
        const a=surface.getBoundingClientRect().height;
        if(!a) return;
        let w=0;
        for(const child of surface.children) {
          if(child.hidden) continue;
          const r=child.getBoundingClientRect();
          const cs=getComputedStyle(child);
          w+=r.height+parseFloat(cs.marginTop||0)+parseFloat(cs.marginBottom||0);
        }
        if(w-a>2) root.dataset.compact='true';
      });
    }
  }
  function edges() {
    if(root.hidden || conversation) return;
    const graph=$('prod-graph'), box=graph.getBoundingClientRect(), svg=$('prod-edges');
    svg.setAttribute('viewBox',`0 0 ${box.width} ${box.height}`);
    const chat=current?.snapshot, tasks=chat?.tasks || [], pending=pendingOf(chat), active=busy.has(chat?.status) && online;
    const lines=[];
    function connect(fromId,toId,state) {
      const a=$(fromId)?.getBoundingClientRect(),b=$(toId)?.getBoundingClientRect();if(!a?.width||!b?.width)return;
      const vertical=box.width<650;
      const x1=(vertical?a.left+a.width/2:a.right)-box.left,y1=(vertical?a.bottom:a.top+a.height/2)-box.top;
      const x2=(vertical?b.left+b.width/2:b.left)-box.left,y2=(vertical?b.top:b.top+b.height/2)-box.top;
      const path=vertical?`M${x1},${y1} C${x1},${(y1+y2)/2} ${x2},${(y1+y2)/2} ${x2},${y2}`:`M${x1},${y1} C${(x1+x2)/2},${y1} ${(x1+x2)/2},${y2} ${x2},${y2}`;
      lines.push(`<path d="${path}" class="prod-edge ${state}"/>`);
    }
    tasks.forEach((task,i)=>{const state=active && task.status==='Working'?'active':task.status==='Done'?'done':'queued';const deps=task.depends_on||[];if(deps.length)deps.forEach(id=>connect(`prod-worker-${id-1}`,`prod-worker-${i}`,state));else connect('prod-model-panel',`prod-worker-${i}`,state);});
    if(chat?.status==='Reviewing'||chat?.status==='Completed') connect('prod-model-panel','prod-review-node',chat.status==='Reviewing' && online?'active':'done');
    const relationships=new Map();
    for(const event of chat?.evidence || []) {
      const task=tasks.findIndex(t=>t.agent===event.agent);if(task<0)continue;
      relationships.set(`${task}:${category(event.action)}`,event.ok?'done':'failed');
    }
    if(active && pending) {const task=tasks.findIndex(t=>t.agent===pending.agent);if(task>=0)relationships.set(`${task}:${pending.category}`,'active');}
    for(const [key,state] of relationships) {const [task,cap]=key.split(':');connect(`prod-worker-${task}`,`prod-cap-${cap}`,state);}
    markup(svg,lines.join(''));
  }
  function showInspector(key,open=true) {
    inspect=key;const [kind,id]=key.split(':'),chat=current?.snapshot;let title='',data;
    if(kind==='log') {data=chat?.evidence || [];title='All observations';}
    if(kind==='result') {data=chat?.messages?.filter(m=>m.agent==='Klyne').at(-1)?.text || '';title='Full result';}
    if(kind==='task') {data=chat?.tasks?.[Number(id)];title='Assignment';}
    if(kind==='event') {data=chat?.evidence?.[Number(id)];title='Recorded observation';}
    if(kind==='cap') {const cap=capabilities.find(c=>c[0]===id);title=cap?.[1] || 'Capability';data={access:!cap?.[3] || !!(chat?.access || current?.access)?.[cap[3]],observations:(chat?.evidence || []).filter(e=>category(e.action)===id),pending:pendingOf(chat)?.category===id?pendingOf(chat).action:null};}
    $('prod-inspector-title').textContent=title;
    $('prod-inspector-body').textContent=typeof data==='string'?data:JSON.stringify(data || {},null,2);
    if(open && !$('prod-inspector').open) $('prod-inspector').showModal();
  }
  $('prod-open-log').onclick=()=>showInspector('log:all');
  $('prod-open-result').onclick=()=>showInspector('result:all');
  root.addEventListener('click',event=>{const node=event.target.closest('[data-inspect]');if(node)showInspector(node.dataset.inspect);});
  $('prod-view-toggle').onclick=()=>{$('prod-inspector').close();inspect=null;conversation=!conversation;morphView();};
  new ResizeObserver(()=>{scheduleEdges();requestAnimationFrame(fit);}).observe($('prod-graph'));
  new ResizeObserver(()=>{requestAnimationFrame(fit);}).observe($('main'));
  window.addEventListener('resize',()=>{cancelMotion();render();});
  motion.addEventListener('change',()=>{cancelMotion();render();});
  document.addEventListener('visibilitychange',()=>{root.classList.toggle('prod-paused',document.hidden);if(document.hidden){cancelMotion();}else render();});
  window.productionView={update,connection(value){if(online!==value){online=value;render();}}};
})();
