'use strict';
const $ = id => document.getElementById(id);
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const running = chat => ['Planning','Working','Reviewing','Stopping'].includes(chat?.status);
const names = {codex:'Codex',ollama:'Ollama',opencode:'OpenCode'};
// Keep the composer about the task; access decisions live together in Settings.
const accessSummary=document.createElement('button');accessSummary.id='access-summary';accessSummary.type='button';accessSummary.className='access';
const accessControls=document.querySelector('.access-controls');
accessControls.before(accessSummary);accessSummary.after($('apps-button'));
$('execution-settings').querySelector('summary').after(accessControls);
$('execution-settings').append($('terminal-note'),$('desktop-note'));
accessSummary.onclick=()=>{$('settings').showModal();$('execution-settings').open=true;$('execution-settings').scrollIntoView({block:'nearest'});};
$('activity-details').querySelector('summary').textContent='Details';
$('activity-details').querySelector('summary').after($('tasks'));
const workerMessages=document.createElement('div');workerMessages.id='worker-messages';$('activity-details').append(workerMessages);
const recoveryLink = document.createElement('a');
recoveryLink.textContent = 'Recovery tools'; recoveryLink.className = 'quiet';
recoveryLink.href = `http://127.0.0.1:${Number(location.port || 4317) + 1}/`;
recoveryLink.target = '_blank'; recoveryLink.rel = 'noopener';
recoveryLink.title = 'Open the independent recovery workspace';
$('execution-settings').append(recoveryLink);
fetch('/api/recovery').then(r=>r.ok?r.json():null).then(info=>{
  const port=info?.state?.recovery_port;
  if(Number.isInteger(port) && port>0 && port<65536) recoveryLink.href=`http://127.0.0.1:${port}/`;
}).catch(()=>{});
const linkedChat = new URLSearchParams(location.hash.slice(1)).get('chat');
let selected = linkedChat && /^[0-9-]{1,79}$/.test(linkedChat) ? linkedChat : null, snapshot = null, chats = [], polling = false, submitting = false, revision = 0, toastTimer;
const taskIndicator=document.createElement('span');
taskIndicator.id='task-indicator';taskIndicator.setAttribute('role','status');taskIndicator.setAttribute('aria-live','polite');
$('connection').after(taskIndicator);
let online=false;
function renderStatus(){
  const activeChats=chats.filter(c=>['Planning','Working','Reviewing','Stopping','Upgrading'].includes(c.status));
  const state=!online?'offline':submitting?'working':running(snapshot)||snapshot?.status==='Upgrading'?'working':snapshot?.status==='Completed'?'finished':['Blocked','Interrupted','Needs input','Stopped'].includes(snapshot?.status)?'attention':activeChats.length?'working':'ready';
  taskIndicator.dataset.state=state;
  const label={offline:'Offline',working:'Working',finished:'Finished',attention:snapshot?.status==='Needs input'?'Needs your input':snapshot?.status==='Stopped'?'Stopped':'Needs attention',ready:'Ready'}[state];
  taskIndicator.textContent=label;
  taskIndicator.title=state==='working'?(snapshot?.activity?.agent||`${activeChats.length || 1} active task`):state==='finished'?'The selected task finished. Open its result for details.':label;
  document.title=`${label} - Klyne`;
}
const drafts = new Map();
let configuredChat=null;
function hydrateExecution(chat){
  if(!chat||configuredChat===chat.id)return;
  configuredChat=chat.id;
  $('command-policy').value=chat.execution?.command_policy||'ask';
  $('max-tokens').value=chat.execution?.max_tokens||0;$('max-cost').value=chat.execution?.max_cost_usd||0;
  $('max-steps').value=chat.limit||0;$('max-seconds').value=chat.execution?.timeout_seconds||0;$('max-reviews').value=chat.execution?.max_review_rounds||0;
}

function toast(message) { $('toast').textContent=message; $('toast').hidden=false; clearTimeout(toastTimer); toastTimer=setTimeout(()=>$('toast').hidden=true,4000); }
async function api(path, body) {
  const response=await fetch(path,body===undefined?{cache:'no-store',signal:AbortSignal.timeout(8000)}:{method:'POST',headers:{'Content-Type':'application/json','X-Klyne-Request':'1'},body:JSON.stringify(body)});
  const result=await response.json(); if(!response.ok) throw new Error(result.error || 'Could not complete the request.'); return result;
}
function setMarkup(element, markup) { if(element.dataset.markup!==markup) {element.innerHTML=markup; element.dataset.markup=markup;} }
function renderList() {
  const query=$('chat-search').value.toLowerCase();
  const filtered=chats.filter(chat=>`${chat.title} ${chat.status}`.toLowerCase().includes(query));
  const focused=document.activeElement?.closest('[data-chat]')?.dataset.chat;
  setMarkup($('chat-list'), filtered.map(chat=>`<div class="chat-row"><button class="chat-item" data-chat="${escape(chat.id)}" data-state="${escape(chat.status)}" aria-current="${selected===chat.id}"><strong>${escape(chat.title)}</strong><small>${escape(chat.status)}</small></button><button class="chat-options" data-options="${escape(chat.id)}" aria-label="Options for ${escape(chat.title)}" aria-haspopup="dialog">⋯</button></div>`).join('') || `<p>${chats.length?'No matching conversations.':'Your conversations will appear here.'}</p>`);
  const count = document.querySelector('.conversation-count');
  if(count) count.textContent = filtered.length;
  if(focused) [...$('chat-list').querySelectorAll('[data-chat]')].find(button=>button.dataset.chat===focused)?.focus({preventScroll:true});
}
function render() {
  hydrateExecution(snapshot);
  renderStatus();
  window.productionView?.update({snapshot,selected,submitting,message:$('instruction').value,provider:providerConfig(),access:{web:$('access-web').checked,terminal:$('access-terminal').checked,desktop:$('access-desktop').checked,apps:$('access-apps').checked}});
  const active=running(snapshot);
  $('writing-mode').disabled=active || submitting;
  $('welcome').hidden=!!selected; $('conversation').hidden=!selected;
  $('chat-title').textContent=snapshot?.title || (selected?'Opening conversation…':'Your workspace');
  $('send').hidden=active; $('send').disabled=submitting || !!selected&&!snapshot;
  $('stop-chat').hidden=!active;
  $('resume-chat').hidden=!snapshot || active || snapshot.status==='Completed' || !!snapshot.pending;
  $('pending-action').hidden=!snapshot?.pending || active;
  if(snapshot?.pending && !active) {
    const approval=!!snapshot.pending.proposal;
    const panel=$('pending-action');
    panel.querySelector('summary').textContent=approval?'Review requested action':'Resolve an uncertain action';
    const request=snapshot.pending.proposal;
    panel.querySelector('p').textContent=!approval?'Inspect the affected app or file, then record what happened before continuing.':request.kind==='shell'?`Run ${request.program} with arguments ${(request.args||[]).map(arg=>JSON.stringify(arg)).join(' ')}. This command has not run.`:request.kind==='delete'?`Allow DELETE ${request.origin?.replace(/\/$/,'')||''}${request.path}. Review the request body in Exact request.`:request.kind==='secret'?`Allow credential ${request.name} for ${(request.origins||[]).join(', ')||'the command environment'}.`:'This action has not run. Review the exact request below.';
    let proposal=$('pending-proposal');
    if(!proposal) {proposal=document.createElement('pre');proposal.id='pending-proposal';const exact=document.createElement('details'),label=document.createElement('summary');label.textContent='Exact request';exact.append(label,proposal);panel.querySelector('p').after(exact);}
    let expiry=$('pending-expiry');if(!expiry){expiry=document.createElement('p');expiry.id='pending-expiry';proposal.parentElement.after(expiry);}expiry.textContent=approval && snapshot.pending.expires_at_ms ? `Approval expires ${new Date(snapshot.pending.expires_at_ms).toLocaleTimeString()} | ${(snapshot.execution?.approval_queue?.length||0)+1} request(s) awaiting review` : '';
    proposal.textContent=JSON.stringify(snapshot.pending.proposal || snapshot.pending.action || snapshot.pending,null,2);
    const choices=approval?[['approved','Approve this action'],['abandon','Decline this action']]:[['completed','Already completed'],['not_applied','Did not happen'],['abandon','Abandon this action']];
    setMarkup($('pending-disposition'),choices.map(([value,label])=>`<option value="${value}">${label}</option>`).join(''));
    panel.querySelector('label').textContent=approval?'Note (optional)':'What did you verify?';
    $('resolve-action').textContent=approval && $('pending-disposition').value==='approved'?'Approve and continue':'Record decision';
  }
  $('instruction').placeholder=active?'You can draft your next instruction while Klyne works…':selected?'Add an instruction or ask a follow-up…':'Describe what you’d like to do…';
  $('composer-hint').textContent=active?'Your team is working · You can stop at any time':'Enter to send · Shift + Enter for a new line';
  for(const id of ['access-web','access-terminal','access-desktop','access-apps','command-policy','max-tokens','max-cost']) $(id).disabled=active || submitting;
  if(!snapshot) { $('messages').replaceChildren(); delete $('messages').dataset.markup; $('work-panel').hidden=true; return; }
  const main=$('main'), nearBottom=main.scrollHeight-main.scrollTop-main.clientHeight<100;
  const messageMarkup=snapshot.messages.filter(m=>m.role==='user'||m.agent==='Klyne').map(m=>`<article class="message ${m.role==='user'?'user':m.agent==='Klyne'?'assistant':'worker'}"><p class="message-label">${escape(m.agent)}</p><div class="message-text">${escape(m.text)}</div></article>`).join('');
  const changed=$('messages').dataset.markup!==messageMarkup;
  setMarkup($('messages'),messageMarkup);
  setMarkup(workerMessages,snapshot.messages.filter(m=>m.role!=='user'&&m.agent!=='Klyne').map(m=>`<p><strong>${escape(m.agent)}</strong> ${escape(m.text)}</p>`).join(''));
  $('work-panel').hidden=false;
  $('work-status').textContent=snapshot.status==='Completed'?'Done · reviewed by AI':snapshot.status;
  $('work-status').dataset.active=String(active);
  $('work-count').textContent=`${snapshot.used}${snapshot.limit ? ` / ${snapshot.limit}` : ''} steps`;
  setMarkup($('tasks'),snapshot.tasks.map(task=>`<div class="task"><span class="task-icon">${task.status==='Done'?'✓':task.status==='Working'?'◉':'○'}</span><div class="task-copy">${escape(task.agent)}<p>${escape(task.instruction)}</p></div><small>${escape(task.status)}</small></div>`).join(''));
  // Keep open evidence entries and keyboard focus while new observations arrive.
  const evidence=$('evidence');
  if(evidence.dataset.chat!==snapshot.id) {evidence.replaceChildren();evidence.dataset.chat=snapshot.id;}
  snapshot.evidence.slice(evidence.children.length).forEach(e=>{
    const detail=document.createElement('details'),summary=document.createElement('summary'),pre=document.createElement('pre');
    summary.textContent=`${e.ok?'✓':'!'} ${e.agent} · ${e.action}`;pre.textContent=`${e.summary}\n\n${e.data}`;
    detail.append(summary,pre);evidence.append(detail);
  });
  $('workspace-path').textContent=`Workspace: ${snapshot.workspace}`;
  if(changed&&nearBottom&&!document.documentElement.classList.contains('production-on')) main.scrollTop=main.scrollHeight;
}
async function refresh() {
  if(polling) return;
  polling=true;
  try {
    const data=await api('/api/chats'); chats=data.chats; renderList();
    if(selected) { const id=selected, fresh=await api(`/api/chats/${id}`); if(selected===id) {snapshot=fresh;render();} }
    online=true;renderStatus();$('connection').textContent='Connected';$('connection').dataset.state='connected';window.productionView?.connection(true);
  } catch(e) { online=false;renderStatus();$('connection').textContent='Offline · retrying';$('connection').dataset.state='offline';window.productionView?.connection(false); }
  finally {polling=false;}
}
function closeSidebar() {$('sidebar').classList.remove('open');$('menu').setAttribute('aria-expanded','false');}
async function select(id) {
  drafts.set(selected,$('instruction').value);selected=id;snapshot=null;
  $('instruction').value=drafts.get(id)||'';$('form-error').textContent='';closeSidebar();render();renderList();
  if(!id) { $('command-policy').value='ask';$('max-tokens').value=0;$('max-cost').value=0;$('instruction').focus();return; }
  try { const fresh=await api(`/api/chats/${id}`);if(id!==selected)return;snapshot=fresh;
    $('command-policy').value=fresh.execution?.command_policy||'ask';$('max-tokens').value=fresh.execution?.max_tokens||0;$('max-cost').value=fresh.execution?.max_cost_usd||0;$('max-steps').value=fresh.limit||0; $('max-seconds').value=fresh.execution?.timeout_seconds||0; $('max-reviews').value=fresh.execution?.max_review_rounds||0; $('host-workspace').value=fresh.access.terminal?fresh.workspace:'';
    $('writing-mode').value=fresh.prompt_maker?'prompt_maker':'general';modeChanged();
    $('access-apps').checked=!!fresh.access.apps;$('access-web').checked=fresh.access.web;$('access-terminal').checked=fresh.access.terminal;$('access-desktop').checked=!!fresh.access.desktop;accessChanged();render();$('main').scrollTop=0;
  }catch(e){if(id===selected)$('form-error').textContent=e.message;}
}
$('chat-list').onclick=e=>{const options=e.target.closest('[data-options]');if(options){openChatOptions(options.dataset.options);return;}const button=e.target.closest('[data-chat]');if(button)select(button.dataset.chat);};
$('new-chat').onclick=()=>{if(!submitting)select(null);};
$('chat-search').oninput=renderList;
$('menu').onclick=()=>{$('menu').setAttribute('aria-expanded',String($('sidebar').classList.toggle('open')));};
document.addEventListener('keydown',e=>{if(e.key==='Escape')closeSidebar();});
document.querySelectorAll('[data-prompt]').forEach(button=>button.onclick=()=>{$('writing-mode').value=button.dataset.mode||'general';modeChanged();$('instruction').value=button.dataset.prompt;$('instruction').focus();});
function modeChanged() {
  const enabled=$('writing-mode').value==='prompt_maker';
  $('prompt-mode-note').hidden=!enabled;
  $('prompt-mode-note').textContent=$('provider-kind').value==='ollama'?'Prompt maker · Concise, clear, copy-ready prompts. Temperature 0.2 · top_p 0.5.':'Prompt maker · Concise, clear, copy-ready prompts. Sampling settings are unavailable through this connector.';
}
$('writing-mode').onchange=modeChanged;
function accessChanged() {const count=['access-web','access-terminal','access-desktop','access-apps'].filter(id=>$(id).checked).length;accessSummary.textContent=count?`Access · ${count} enabled`:'Access · Workspace';$('terminal-note').hidden=!$('access-terminal').checked;$('desktop-note').hidden=!$('access-desktop').checked;}
$('access-terminal').onchange=accessChanged;
$('access-desktop').onchange=accessChanged;$('access-web').onchange=accessChanged;$('access-apps').onchange=accessChanged;accessChanged();
$('instruction').onkeydown=e=>{if(e.key==='Enter'&&!e.shiftKey&&!e.isComposing){e.preventDefault();if(!running(snapshot)&&!submitting)$('chat-form').requestSubmit();}};
$('chat-form').onsubmit=async e=>{
  e.preventDefault();if(submitting||running(snapshot))return;
  const message=$('instruction').value.trim();if(!message)return;
  if($('provider-kind').value!=='codex'&&!$('provider-model').value.trim()) {$('form-error').textContent='Choose your AI model in Settings first.';$('settings').showModal();return;}
  submitting=true;$('send').textContent='…';$('form-error').textContent='';render();
  const source=selected;
  try {
    const data=await api('/api/chats',{id:source,message,execution:executionConfig(),workspace:$('host-workspace').value.trim(),prompt_maker:$('writing-mode').value==='prompt_maker',provider:providerConfig(),access:{web:$('access-web').checked,terminal:$('access-terminal').checked,apps:$('access-apps').checked,desktop:$('access-desktop').checked}});
    drafts.delete(source);$('instruction').value='';selected=data.id;snapshot=null;render();await refresh();
  }catch(error){$('form-error').textContent=error.message;}
  finally{submitting=false;$('send').textContent='↑';render();}
};
$('stop-chat').onclick=async()=>{
  const id=selected;$('stop-chat').disabled=true;
  try{await api(`/api/chats/${id}/stop`,{});toast('Stopping after the current call finishes.');}
  catch(e){toast(e.message);}finally{$('stop-chat').disabled=false;}
};
$('export').onclick=()=>{
  if(!snapshot)return;const url=URL.createObjectURL(new Blob([JSON.stringify(snapshot,null,2)],{type:'application/json'}));
  const a=document.createElement('a');a.href=url;a.download=`klyne-chat-${snapshot.id}.json`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
};
function providerConfig(){return {kind:$('provider-kind').value,endpoint:$('provider-endpoint').value.trim(),model:$('provider-model').value.trim()};}
function saveProvider(){try{localStorage.setItem('klyne-provider',JSON.stringify(providerConfig()));}catch(_){}$('model-shortcut').textContent=`${names[$('provider-kind').value]}${$('provider-model').value?' · '+$('provider-model').value:''} ▾`;}
function configureProvider(config={}){
  ++revision;const kind=$('provider-kind').value;
  $('endpoint-field').hidden=kind==='codex';$('provider-endpoint').value=config.endpoint||({ollama:'http://127.0.0.1:11434',opencode:'http://127.0.0.1:4096'}[kind]||'');
  $('provider-model').value=config.model||'';$('model-optional').textContent=kind==='codex'?'(optional)':'';
  $('provider-model').placeholder=kind==='codex'?'Use the Codex default':'Check connection to find models';
  $('provider-help').textContent=({codex:'Uses your existing Codex CLI login. Model inference may use the cloud.',ollama:'Uses models installed on this computer. Start Ollama, then check the connection.',opencode:'Start your local OpenCode server, then check the connection. Model hosting depends on its provider.'})[kind];
  $('provider-status').textContent='Not checked';$('provider-models').replaceChildren();saveProvider();modeChanged();
}
$('provider-kind').onchange=()=>configureProvider();
for(const id of ['provider-endpoint','provider-model'])$(id).oninput=()=>{++revision;$('provider-status').textContent='Settings changed. Check again.';saveProvider();};
$('settings-button').onclick=$('model-shortcut').onclick=()=>{$('settings').showModal();loadApiConnections();};
let installedApps=[];
function renderApps() {
  const query=$('apps-search').value.toLowerCase();
  const filtered=installedApps.filter(app=>`${app.Name} ${app.AppID}`.toLowerCase().includes(query));
  setMarkup($('apps-list'), filtered.map(app=>`<button type="button" class="app-row" data-app="${escape(app.AppID)}" data-name="${escape(app.Name)}"><span class="app-avatar" aria-hidden="true">${escape(app.Name.slice(0,1).toUpperCase())}</span><span class="app-copy"><strong>${escape(app.Name)}</strong></span><span class="app-select-arrow" aria-hidden="true">&#8599;</span></button>`).join('') || `<p>${installedApps.length?'No matching apps.':'No installed apps found.'}</p>`);
}
$('apps-search').oninput=renderApps;
$('apps-button').onclick=async()=>{
  $('apps').showModal();$('apps-search').value='';
  if(installedApps.length){renderApps();return;}
  $('apps-status').textContent='Finding your apps...';setMarkup($('apps-list'),'');
  try{const result=await api('/api/desktop/apps');installedApps=(Array.isArray(result.apps)?result.apps:[]).sort((a,b)=>a.Name.localeCompare(b.Name));
    $('apps-status').textContent=installedApps.length?`${installedApps.length} apps`:'No apps found';renderApps();
  }catch(e){$('apps-status').textContent=e.message;}
};
async function loadApiConnections(){
  try{
    const result=await api('/api/apps');
    setMarkup($('api-connections'),result.connections.map(app=>`<div class="api-row"><div class="app-copy"><strong>${escape(app.name)}</strong><small>${escape(app.base_url)} · ${app.inspected_at?'Schema saved; operations untested':'Not inspected'}</small></div><div class="api-actions"><button data-api="${escape(app.name)}" data-action="use">Use</button><button data-api="${escape(app.name)}" data-action="inspect">Discover</button>${app.inspected_at?`<button data-api="${escape(app.name)}" data-action="operations">Operations</button>`:''}<button data-api="${escape(app.name)}" data-action="forget">Forget</button></div></div>`).join('')||'<p class="settings-copy">No API connections saved yet.</p>');
  }catch(e){$('api-status').textContent=e.message;}
}
$('api-connect-form').onsubmit=async e=>{
  e.preventDefault();$('api-save').disabled=true;$('api-status').textContent='Saving…';
  try{await api('/api/apps',{tool:'app_connect',name:$('api-name').value.trim(),base_url:$('api-origin').value.trim(),...($('api-bearer-env').value.trim()?{auth:{bearer_env:$('api-bearer-env').value.trim()}}:{})});$('api-status').textContent='Connection saved. Discover its API or describe a task to Klyne.';await loadApiConnections();}
  catch(error){$('api-status').textContent=error.message;}finally{$('api-save').disabled=false;}
};
let apiOperationsName='',apiOperationsOffset=0;
async function showApiOperations(name,offset=0){
  const result=await api('/api/apps',{tool:'app_operations',name,offset});
  apiOperationsName=name;apiOperationsOffset=result.next_offset;
  $('api-operation-details').hidden=false;$('api-operation-details').open=true;
  $('api-operation-list').textContent=JSON.stringify(result.operations,null,2);
  $('api-operation-more').hidden=result.next_offset===null;
}
$('api-operation-more').onclick=async()=>{
  $('api-operation-more').disabled=true;
  try{await showApiOperations(apiOperationsName,apiOperationsOffset);}catch(e){$('api-status').textContent=e.message;}finally{$('api-operation-more').disabled=false;}
};
$('api-connections').onclick=async e=>{
  const button=e.target.closest('[data-api]');if(!button||button.disabled)return;
  const name=button.dataset.api,action=button.dataset.action;
  if(action==='use'){
    if(submitting||['Planning','Working','Reviewing'].includes(snapshot?.status)){toast('Wait for the current instruction or stop it first.');return;}
    $('access-apps').checked=true;$('instruction').value=`Using the ${name} API connection, ${$('instruction').value}`;$('settings').close();$('instruction').focus();return;
  }
  button.disabled=true;$('api-status').textContent=action==='inspect'?'Reading the app’s API description…':'Working…';
  try{
    if(action==='operations'){await showApiOperations(name);$('api-status').textContent='Saved API definitions; operations have not been tested.';return;}
    const result=await api('/api/apps',{tool:action==='inspect'?'app_inspect':'app_forget',name});
    $('api-status').textContent=action==='inspect'?`${result.discovered} operations discovered. Definitions saved; operations are not yet tested.`:'Connection forgotten.';
    $('api-operation-details').hidden=true;await loadApiConnections();
  }catch(error){$('api-status').textContent=error.message;}finally{button.disabled=false;}
};
$('apps-list').onclick=async e=>{
  if(submitting||['Planning','Working','Reviewing'].includes(snapshot?.status)){toast('Wait for the current task to finish or stop it first.');return;}
  const button=e.target.closest('[data-app]');if(!button||button.disabled)return;
  button.disabled=true;const name=button.dataset.name||'the app';
  $('apps-status').textContent=`Checking connections for ${name}…`;
  try{
    const route=await api('/api/apps/select',{app_id:button.dataset.app});
    $('access-apps').checked=true;
    const connection=(route.instruction||'Use desktop controls. Observe first and verify the result.')+' ';
    $('instruction').value=connection+$('instruction').value;
    $('access-desktop').checked=true;accessChanged();
    $('apps').close();
    if(!$('instruction').value.startsWith(`In ${name}, `))$('instruction').value=`In ${name}, ${$('instruction').value}`;
    $('instruction').focus();
    toast(route.route==='mcp'?`${name}: MCP checked and ready. Uses a separate browser profile.`:`${name}: desktop controls ready.`);
  }catch(error){$('apps-status').textContent=error.message;}finally{button.disabled=false;}
};
$('check-provider').onclick=async()=>{
  const current=revision;$('check-provider').disabled=true;$('provider-status').textContent='Checking…';
  try{const result=await api('/api/connections/check',providerConfig());if(current!==revision)return;
    $('provider-status').textContent=result.message;$('provider-models').replaceChildren(...result.models.map(model=>{const option=document.createElement('option');option.value=model;return option;}));
    if(!$('provider-model').value&&result.models.length)$('provider-model').value=result.models[0];saveProvider();
  }catch(e){if(current===revision)$('provider-status').textContent=e.message;}finally{$('check-provider').disabled=false;}
};
let saved={};try{saved=JSON.parse(localStorage.getItem('klyne-provider')||'{}')||{};}catch(_){}
if(Object.hasOwn(names,saved.kind))$('provider-kind').value=saved.kind;else saved={};
configureProvider(saved);render();refresh();setInterval(()=>{if(!document.hidden)refresh();},1500);

function executionConfig(){return {max_steps:Number($('max-steps').value)||0,timeout_seconds:Number($('max-seconds').value)||0,max_review_rounds:Number($('max-reviews').value)||0,command_policy:$('command-policy').value,max_tokens:Number($('max-tokens').value)||0,max_cost_usd:Number($('max-cost').value)||0};}
$('resume-chat').onclick=async()=>{
  if(!snapshot||submitting)return; submitting=true;
  try {await api('/api/chats',{id:selected,message:'Continue the saved goal from its current task state.',resume:true,execution:executionConfig(),provider:snapshot.provider,access:snapshot.access});await refresh();}
  catch(e){toast(e.message);}finally{submitting=false;render();}
};
$('pending-disposition').onchange=()=>{$('resolve-action').textContent=snapshot?.pending?.proposal && $('pending-disposition').value==='approved'?'Approve and continue':'Record decision';};
$('resolve-action').onclick=async()=>{
  if(submitting || !snapshot?.pending)return;
  const id=selected,approval=!!snapshot.pending.proposal,disposition=$('pending-disposition').value;
  const continuation={id,message:'Continue the saved goal after approval.',resume:true,execution:executionConfig(),provider:snapshot.provider,access:snapshot.access};
  submitting=true;$('resolve-action').disabled=true;
  try {const resolution=await api(`/api/chats/${id}/resolve`,{request_id:snapshot.pending.request_id,disposition,note:$('pending-note').value.trim() || (approval?`User selected ${disposition} for the displayed proposal.`:'')});if(selected===id)$('pending-note').value='';if(approval && disposition==='approved') {if(resolution.renewal_required)toast('This request expired or changed. Klyne will prepare a fresh proposal.');await api('/api/chats',continuation);}await refresh();}
  catch(e){toast(e.message);}finally{submitting=false;$('resolve-action').disabled=false;render();}
};

$('trusted-laptop').onclick=()=>{
  if(running(snapshot)){toast('Stop current work before changing access.');return;}
  for(const id of ['access-web','access-terminal','access-desktop','access-apps'])$(id).checked=true;
  $('command-policy').value='autonomous';
  try{localStorage.setItem('klyne-trusted-laptop','true');}catch(_){}
  accessChanged();toast('Trusted laptop access enabled for new work.');
};
try{if(localStorage.getItem('klyne-trusted-laptop')==='true'){for(const id of ['access-web','access-terminal','access-desktop','access-apps'])$(id).checked=true;accessChanged();}}catch(_){}

// A small heat field rendered as text: no video, external assets or GPU context.
(() => {
  const mark = document.querySelector('.living-mark');
  const form = document.querySelector('#chat-form');
  if (!mark || !form) return;
  const canvas = document.createElement('canvas');
  canvas.className = 'ascii-fire';
  canvas.setAttribute('aria-hidden', 'true');
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  mark.replaceChildren(canvas);
  mark.classList.add('digital-fire');
  const sidebarMark = document.querySelector('.sidebar .brand-mark');
  const mini = document.createElement('canvas');
  mini.className = 'sidebar-fire';
  mini.width = 120; mini.height = 96;
  mini.setAttribute('aria-hidden', 'true');
  const miniCtx = mini.getContext('2d');
  if (miniCtx && sidebarMark) sidebarMark.replaceChildren(mini);
  const label = document.querySelector('.sidebar-label');
  const count = document.createElement('span');
  count.className = 'conversation-count'; count.textContent = chats.length;
  label.append(count);
  const mobile = matchMedia('(max-width: 700px)');
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  const columns = 30;
  const rows = 24, width = 240, height = 192;
  const heat = new Float32Array(columns * rows);
  const next = new Float32Array(heat.length);
  const glyphs = ' .,:;!|+xX#%@';
  const colors = ['#82351c', '#b34720', '#ed5823', '#ff792f', '#ffa653', '#ffdb97'];
  let frame = 0, last = 0, raf = 0;
  function size() {
    const dpr = Math.min(devicePixelRatio || 1, 2);
    canvas.width = width * dpr; canvas.height = height * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.font = 'bold 9px Consolas, monospace';
    ctx.textBaseline = 'top';
  }
  let wind = 0, windTarget = 0, gustFrames = 0;
  const firePointer = {x:0,y:0,vx:0,time:0,active:false};
  let cursorLean = 0, cursorHeat = 0;
  addEventListener('pointermove', event => {
    if(event.pointerType === 'touch' || motion.matches) return;
    const now=performance.now(), elapsed=Math.max(16,now-firePointer.time)/1000;
    firePointer.vx=firePointer.active ? Math.max(-900,Math.min(900,(event.clientX-firePointer.x)/elapsed)) : 0;
    firePointer.x=event.clientX; firePointer.y=event.clientY; firePointer.time=now; firePointer.active=true;
  }, {passive:true});
  function releaseFirePointer() { firePointer.active=false; firePointer.vx=0; }
  document.documentElement.addEventListener('pointerleave',releaseFirePointer);
  addEventListener('blur',releaseFirePointer);
  function influence() {
    let strength=0, lean=0;
    if(firePointer.active && !motion.matches && !document.hidden) {
      for(const target of [canvas,mini]) {
        const rect=target.getBoundingClientRect();
        if(!rect.width || !rect.height || rect.bottom<0 || rect.top>innerHeight) continue;
        const dx=firePointer.x-(rect.left+rect.width/2), dy=firePointer.y-(rect.top+rect.height/2);
        const radius=Math.max(100,rect.width*1.3);
        const proximity=Math.max(0,1-Math.hypot(dx,dy)/radius);
        if(proximity>strength) {
          strength=proximity;
          const sweep=firePointer.vx/600*Math.exp(-(performance.now()-firePointer.time)/220);
          lean=Math.max(-1.7,Math.min(1.7,dx/(rect.width/2)+sweep))*proximity;
        }
      }
    }
    cursorLean+=(lean-cursorLean)*.22;
    cursorHeat+=(strength-cursorHeat)*.16;
  }
  const sources = Array.from({length: 4}, () => ({x: 8 + Math.random() * 14, strength: .4, target: .8, life: 0}));
  const sparks = [];
  function advance() {
    frame++;
    influence();
    if (--gustFrames <= 0) { windTarget = (Math.random() - .5) * 1.8; gustFrames = 8 + Math.random() * 35; }
    wind += (windTarget - wind) * .09;
    for (const source of sources) {
      if (--source.life <= 0) {
        source.x = 6 + Math.random() * 18;
        source.target = .35 + Math.random() * .8;
        source.life = 5 + Math.random() * 28;
      }
      source.strength += (source.target - source.strength) * .18;
    }
    for (let x = 0; x < columns; x++) {
      let fuel = 0;
      for (const source of sources) fuel += Math.exp(-(((x - source.x) / 3.2) ** 2)) * source.strength;
      heat[(rows - 1) * columns + x] = Math.min(1, fuel * (.65+cursorHeat*.35)) * (.8 + Math.random() * .2);
    }
    for (let y = 0; y < rows - 1; y++) {
      for (let x = 0; x < columns; x++) {
        const below = (y + 1) * columns;
        const offset = (wind-cursorLean*1.5) * (1 - y / rows) + (Math.random() - .5) * 1.1;
        const sample = Math.max(1, Math.min(columns - 2, Math.round(x + offset)));
        next[y * columns + x] = Math.max(0, (heat[below + sample - 1] + heat[below + sample] * 2 + heat[below + sample + 1]) / 4 - .015 - Math.random() * .04 + cursorHeat*.008);
      }
    }
    heat.set(next.subarray(0, (rows - 1) * columns));
    if (Math.random() < .35+cursorHeat*.4 && sparks.length < 18) sparks.push({x: 7 + Math.random() * 16, y: 16 + Math.random() * 5, life: 12 + Math.random() * 18});
    for (let i = sparks.length - 1; i >= 0; i--) {
      const spark = sparks[i]; spark.y -= .35 + Math.random() * .5 + cursorHeat*.25; spark.x -= (wind-cursorLean*1.5) * .3 + (Math.random() - .5) * .5;
      if (--spark.life <= 0 || spark.y < 0) sparks.splice(i, 1);
    }
  }
  function draw() {
    ctx.clearRect(0, 0, width, height);
    for (let y = 0; y < rows; y++) for (let x = 0; x < columns; x++) {
      const value = heat[y * columns + x];
      if (value < .055) continue;
      ctx.fillStyle = colors[Math.min(colors.length - 1, Math.floor(value * colors.length))];
      ctx.globalAlpha = Math.min(1, value * 3);
      ctx.fillText(glyphs[Math.min(glyphs.length - 1, Math.floor(value * glyphs.length))], x * 8, y * 8);
    }
    for (const spark of sparks) {
      ctx.globalAlpha = Math.min(.8, spark.life / 18);
      ctx.fillStyle = '#ffc281';
      ctx.fillText(spark.life > 12 ? '+' : '.', spark.x * 8, spark.y * 8);
    }
    ctx.globalAlpha = 1;
    if (miniCtx) {
      miniCtx.clearRect(0, 0, mini.width, mini.height);
      miniCtx.drawImage(canvas, 0, 0, mini.width, mini.height);
    }
  }
  function tick(time) {
    if (time - last >= 80) { advance(); draw(); last = time; }
    raf = requestAnimationFrame(tick);
  }
  function sync() {
    cancelAnimationFrame(raf);
    const paused = document.hidden || motion.matches;
    if(paused) { releaseFirePointer(); cursorLean=cursorHeat=0; }
    document.documentElement.classList.toggle('effects-paused', paused);
    const sidebarVisible = mobile.matches ? document.querySelector('#sidebar').classList.contains('open') : !document.querySelector('#sidebar').classList.contains('collapsed');
    if (!paused && (!document.querySelector('#welcome').hidden || sidebarVisible)) raf = requestAnimationFrame(tick);
  }
  size();
  for (let i = 0; i < 70; i++) advance();
  draw(); sync();
  motion.addEventListener('change', sync);
  mobile.addEventListener('change', sync);
  document.addEventListener('visibilitychange', sync);
  new MutationObserver(sync).observe(document.querySelector('#welcome'), {attributes: true, attributeFilter: ['hidden']});
  new MutationObserver(sync).observe(document.querySelector('#sidebar'), {attributes: true, attributeFilter: ['class']});
  form.addEventListener('pointermove', event => {
    if (motion.matches || event.pointerType === 'touch') return;
    const bounds = form.getBoundingClientRect();
    form.style.setProperty('--pointer-x', `${((event.clientX - bounds.left) / bounds.width) * 100}%`);
    form.style.setProperty('--pointer-y', `${((event.clientY - bounds.top) / bounds.height) * 100}%`);
  });
  form.addEventListener('pointerleave', () => {
    form.style.removeProperty('--pointer-x'); form.style.removeProperty('--pointer-y');
  });
})();


// Conversation management stays separate from selecting a conversation.
const chatOptions = document.createElement('dialog');
chatOptions.id = 'chat-options-dialog';
chatOptions.setAttribute('aria-labelledby', 'chat-options-heading');
chatOptions.innerHTML = `<form method="dialog" class="dialog-heading"><h2 id="chat-options-heading">Conversation options</h2><button class="icon-button" aria-label="Close conversation options">×</button></form>
<label for="chat-rename-title">Conversation name</label><input id="chat-rename-title" maxlength="120">
<button id="chat-rename-save" type="button">Save name</button>
<details id="chat-delete-confirm"><summary>Delete conversation…</summary><p class="settings-copy">Remove this chat from your conversations? Its history and local artifacts move to the deleted folder. Files in your selected project stay where they are.</p><button id="chat-delete-yes" type="button">Delete conversation</button></details>
<p id="chat-management-status" role="status" class="settings-copy"></p>`;
document.body.append(chatOptions);
let managedChat = null;
function openChatOptions(id) {
  const chat = chats.find(c => c.id === id); if(!chat) return;
  managedChat = id;
  $('chat-rename-title').value = chat.title;
  $('chat-delete-confirm').open = false;
  $('chat-management-status').textContent = '';
  chatOptions.showModal();
  $('chat-rename-title').focus();
}
async function manageChat(action) {
  const id = managedChat;
  $('chat-rename-save').disabled = $('chat-delete-yes').disabled = true;
  try {
    await api(`/api/chats/${id}/manage`, {action, title:$('chat-rename-title').value, confirmed:action==='delete'});
    if(action==='delete') {
      if(selected===id) await select(null);
      drafts.delete(id);
    }
    chatOptions.close();
    await refresh();
    toast(action==='delete' ? 'Conversation deleted.' : 'Conversation renamed.');
  } catch(error) { $('chat-management-status').textContent = error.message; }
  finally { $('chat-rename-save').disabled = $('chat-delete-yes').disabled = false; }
}
$('chat-rename-save').onclick=()=>manageChat('rename');
$('chat-delete-yes').onclick=()=>manageChat('delete');


// Persisted desktop width; mobile keeps its independent drawer behavior.
(() => {
  const sidebar = $('sidebar'), menu = $('menu'), mobile = matchMedia('(max-width:700px)');
  const collapse = document.createElement('button');
  collapse.id = 'collapse-sidebar'; collapse.className = 'icon-button';
  collapse.type = 'button'; collapse.textContent = '‹';
  collapse.setAttribute('aria-label', 'Collapse sidebar');
  sidebar.prepend(collapse);
  const handle = document.createElement('div');
  handle.id = 'sidebar-resize'; handle.tabIndex = 0;
  handle.setAttribute('role', 'separator'); handle.setAttribute('aria-orientation', 'vertical');
  handle.setAttribute('aria-label', 'Resize sidebar');
  handle.setAttribute('aria-controls', 'sidebar');
  sidebar.append(handle);
  let width = 280, collapsed = false, dragging = false;
  try { width = Number(localStorage.getItem('klyne-sidebar-width')) || 280; collapsed = localStorage.getItem('klyne-sidebar-collapsed') === 'true'; } catch (_) {}
  function applyWidth(value, save = true) {
    const max = Math.min(480, Math.floor(innerWidth * .45));
    width = Math.max(240, Math.min(max, value));
    document.documentElement.style.setProperty('--sidebar-width', `${width}px`);
    handle.setAttribute('aria-valuemin', '240'); handle.setAttribute('aria-valuemax', String(max)); handle.setAttribute('aria-valuenow', String(width));
    if(save) try { localStorage.setItem('klyne-sidebar-width', String(width)); } catch (_) {}
  }
  function applyCollapsed() {
    sidebar.classList.toggle('collapsed', collapsed && !mobile.matches);
    document.documentElement.classList.toggle('sidebar-collapsed', collapsed && !mobile.matches);
    sidebar.inert = collapsed && !mobile.matches;
    menu.setAttribute('aria-expanded', String(mobile.matches ? sidebar.classList.contains('open') : !collapsed));
    menu.setAttribute('aria-label', mobile.matches ? 'Toggle conversations' : collapsed ? 'Expand sidebar' : 'Collapse sidebar');
  }
  function toggle() {
    if(mobile.matches) { sidebar.classList.toggle('open'); }
    else { collapsed = !collapsed; try { localStorage.setItem('klyne-sidebar-collapsed', String(collapsed)); } catch (_) {} }
    applyCollapsed(); menu.focus();
  }
  menu.onclick = toggle;
  collapse.onclick = () => { if(mobile.matches) sidebar.classList.remove('open'); else collapsed = true; try {localStorage.setItem('klyne-sidebar-collapsed', String(collapsed));} catch (_) {} applyCollapsed(); menu.focus(); };
  handle.addEventListener('pointerdown', e => { if(mobile.matches || e.button !== 0) return; dragging = true; handle.setPointerCapture(e.pointerId); document.documentElement.classList.add('sidebar-resizing'); e.preventDefault(); });
  handle.addEventListener('pointermove', e => { if(dragging) applyWidth(e.clientX); });
  const finish = () => { dragging = false; document.documentElement.classList.remove('sidebar-resizing'); };
  handle.addEventListener('pointerup', finish); handle.addEventListener('pointercancel', finish); handle.addEventListener('lostpointercapture', finish);
  handle.addEventListener('keydown', e => {
    if(!['ArrowLeft','ArrowRight','Home','End'].includes(e.key)) return;
    e.preventDefault(); applyWidth(e.key === 'Home' ? 240 : e.key === 'End' ? 480 : width + (e.key === 'ArrowRight' ? 16 : -16));
  });
  handle.addEventListener('dblclick', () => applyWidth(280));
  mobile.addEventListener('change', () => { finish(); sidebar.classList.remove('open'); applyCollapsed(); });
  addEventListener('resize', () => { if(!mobile.matches) applyWidth(width, false); });
  new MutationObserver(() => { menu.setAttribute('aria-expanded', String(mobile.matches ? sidebar.classList.contains('open') : !collapsed)); }).observe(sidebar, {attributes:true, attributeFilter:['class']});
  applyWidth(width, false); applyCollapsed();
})();

// Ambient embers: one lightweight canvas behind the interface.
(() => {
  const canvas = document.createElement('canvas');
  canvas.id = 'ambient-embers'; canvas.setAttribute('aria-hidden', 'true');
  document.body.prepend(canvas);
  const ctx = canvas.getContext('2d'); if (!ctx) { canvas.remove(); return; }
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  let width = 0, height = 0, raf = 0, previous = 0;
  const particles = [];
  let cpuFraction = 0, displayedLoad = 0, particleBudget = 60, baselineEmbers = 24;
  let loadTimer = 0, loadPending = false;
  const loadLabel = document.createElement('span');
  loadLabel.id = 'runtime-load'; loadLabel.textContent = 'Studio CPU · measuring…';
  loadLabel.title = 'Studio process CPU as a share of total CPU capacity. Separate model servers, child processes, GPU work and cloud inference are not included. Ember density increases during active tasks, with additional response to this reading.';
  document.querySelector('.sidebar-bottom').replaceChildren(Object.assign(document.createElement('span'), {className:'local-dot'}),loadLabel);
  async function sampleLoad() {
    clearTimeout(loadTimer);
    if(document.hidden || loadPending) return;
    loadPending=true;
    try {
      const load=await api('/api/runtime/load');
      window.dispatchEvent(new CustomEvent('klyne-telemetry',{detail:load}));
      if(typeof load.cpu_percent==='number' && Number.isFinite(load.cpu_percent)) {
        cpuFraction=Math.max(0,Math.min(1,load.cpu_percent/100));
        loadLabel.textContent=`Studio CPU · ${load.cpu_percent.toFixed(1)}%`;
      } else {
        cpuFraction=0;
        loadLabel.textContent=load.available?'Studio CPU · measuring…':'Studio CPU · unavailable';
      }
    } catch (_) { cpuFraction=0; loadLabel.textContent='Studio CPU · unavailable'; window.dispatchEvent(new CustomEvent('klyne-telemetry',{detail:{cpu_percent:null}})); }
    finally { loadPending=false; if(!document.hidden) loadTimer=setTimeout(sampleLoad,2000); }
  }
  document.addEventListener('visibilitychange', () => { clearTimeout(loadTimer); if(!document.hidden) sampleLoad(); });
  sampleLoad();
  const pointer = {x:0, y:0, active:false, vx:0, vy:0, time:0};
  addEventListener('pointermove', event => {
    if(event.pointerType === 'touch' || motion.matches) return;
    const now = performance.now(), elapsed = Math.max(16, now-pointer.time)/1000;
    pointer.vx = pointer.active ? Math.max(-700,Math.min(700,(event.clientX-pointer.x)/elapsed)) : 0;
    pointer.vy = pointer.active ? Math.max(-700,Math.min(700,(event.clientY-pointer.y)/elapsed)) : 0;
    pointer.x=event.clientX; pointer.y=event.clientY; pointer.time=now; pointer.active=true;
  }, {passive:true});
  function releasePointer() { pointer.active=false; pointer.vx=pointer.vy=0; }
  document.documentElement.addEventListener('pointerleave',releasePointer);
  addEventListener('blur',releasePointer);
  function spawn(initial = false) {
    return {x:Math.random()*width,y:initial?Math.random()*height:height+12,
      speed:12+Math.random()*34,drift:(Math.random()-.5)*18,
      radius:.8+Math.random()*1.6,alpha:.35+Math.random()*.4,
      phase:Math.random()*Math.PI*2,frequency:.25+Math.random()*.65,
      life:0,turn:Math.random()*6,red:Math.random()>.25,vx:0,vy:0,visibility:0};
  }
  function resize() {
    width = innerWidth; height = innerHeight;
    const dpr = Math.min(devicePixelRatio || 1, 1.5);
    canvas.width = Math.round(width*dpr); canvas.height = Math.round(height*dpr);
    ctx.setTransform(dpr,0,0,dpr,0,0);
    particles.length = 0;
    particleBudget=Math.min(150,Math.max(50,Math.round(width*height/12000)));
    baselineEmbers=Math.min(42,Math.max(18,Math.round(width*height/28000)));
    for(let i=0;i<particleBudget;i++) { const p=spawn(true);p.visibility=i<baselineEmbers?1:0;particles.push(p); }
    draw(0);
  }
  function draw(dt) {
    ctx.clearRect(0,0,width,height);
    const working=submitting || ['Planning','Working','Reviewing','Upgrading'].includes(snapshot?.status);
    const targetLoad=working?Math.max(.85,cpuFraction):cpuFraction;
    displayedLoad+=(targetLoad-displayedLoad)*(1-Math.exp(-dt*1.5));
    const targetCount=baselineEmbers+(particleBudget-baselineEmbers)*displayedLoad;
    for(let i=0;i<particles.length;i++) {
      particles[i].visibility+=((i<targetCount?1:0)-particles[i].visibility)*(1-Math.exp(-dt*2));
      if(particles[i].visibility<.005) continue;
      let p=particles[i]; p.life+=dt;
      let proximity=0;
      if(pointer.active && !motion.matches) {
        const dx=p.x-pointer.x, dy=p.y-pointer.y, distance=Math.hypot(dx,dy);
        proximity=Math.max(0,1-distance/190);
        if(proximity>0) {
          const nx=dx/Math.max(1,distance), ny=dy/Math.max(1,distance), force=proximity*proximity;
          const wake=Math.exp(-Math.max(0,performance.now()-pointer.time)/180);
          p.vx+=(nx*170-ny*115+pointer.vx*wake*.65)*force*dt;
          p.vy+=(ny*170+nx*115+pointer.vy*wake*.65)*force*dt;
        }
      }
      const damping=Math.exp(-2.2*dt);
      p.vx=Math.max(-160,Math.min(160,p.vx*damping));
      p.vy=Math.max(-160,Math.min(160,p.vy*damping));
      p.y+=(-p.speed+p.vy)*dt;
      p.x+=(p.drift+Math.sin(p.phase+p.life*p.frequency)*12+p.vx)*dt;
      if(p.y < -20 || p.x < -30 || p.x > width+30) { const visibility=p.visibility;p=particles[i]=spawn();p.visibility=visibility; }
      const fade=Math.min(1,Math.max(0,p.y/90));
      const alpha=Math.min(.85,p.alpha*fade*(.76+.24*Math.sin(p.phase+p.life*1.8))*(1+proximity*.65));
      ctx.save(); ctx.translate(p.x,p.y); ctx.rotate(p.turn+p.life*.3);
      ctx.fillStyle=p.red?'#ff392b':'#ff6b35';
      ctx.globalAlpha=alpha*p.visibility*.13;
      ctx.beginPath();ctx.arc(0,0,p.radius*4,0,Math.PI*2);ctx.fill();
      ctx.globalAlpha=alpha*p.visibility;
      ctx.fillRect(-p.radius/2,-p.radius,p.radius,p.radius*2.4);
      ctx.restore();
    }
  }
  function tick(time) {
    if(time-previous>=32) {
      draw(previous?Math.min((time-previous)/1000,.06):0);
      previous=time;
    }
    raf=requestAnimationFrame(tick);
  }
  function sync() {
    cancelAnimationFrame(raf); previous=0;
    releasePointer();
    if(!document.hidden && !motion.matches) raf=requestAnimationFrame(tick);
    else draw(0);
  }
  addEventListener('resize',resize);
  document.addEventListener('visibilitychange',sync);
  motion.addEventListener('change',sync);
  resize(); sync();
})();
