// Durable goals use the same execution engine, access settings and Stop controls.
(() => {
  const button=document.createElement('button');button.id='goals-button';button.className='quiet';button.textContent='Goals';
  document.querySelector('#settings-button').before(button);
  const dialog=document.createElement('dialog');dialog.id='goals-dialog';
  dialog.innerHTML='<form method="dialog" class="dialog-heading"><h2>Goal board</h2><button aria-label="Close goals">×</button></form><p>Saved goals continue while Klyne runs, including in background mode. Two independent workers can run together. Goals using Terminal, Desktop or app APIs run alone.</p><label for="goal-instructions">New goal</label><textarea id="goal-instructions" rows="3" maxlength="8192" placeholder="Describe the outcome you want"></textarea><label><input type="checkbox" id="goal-after-current"> Wait for all unfinished board goals</label><label><input type="checkbox" id="goal-reviewed"> Accept model review when no host-verifiable contract is supplied</label><p>Uses the AI connection and access currently selected in Settings. Each goal gets finite limits: 64 steps, 15 minutes, 100,000 tokens and 3 review passes unless you selected other nonzero limits.</p><button id="goal-add" type="button">Add goal</button><p id="goal-error" role="status"></p><div id="goal-list" aria-live="polite"></div>';
  document.body.append(dialog);
  let jobs=[],busy=false,pendingSubmission=null;
  async function refresh(){
    if(busy||!dialog.open)return;
    busy=true;
    try{
      const response=await fetch('/api/goals');const data=await response.json();if(!response.ok)throw Error(data.error||'Cannot load goals');jobs=data.jobs;
      const list=document.querySelector('#goal-list');list.replaceChildren();
      for(const job of jobs){
        const card=document.createElement('article');card.className='goal-card';
        const title=document.createElement('h3');title.textContent=job.goal;card.append(title);
        const state=document.createElement('p');state.textContent=`${job.state.replaceAll('_',' ')} · ${job.require_verified?'Host verification required':'Reviewed acceptance'}${job.depends_on.length?' · Depends on '+job.depends_on.join(', '):''}`;card.append(state);
        const note=document.createElement('p');note.textContent=job.activity||job.note;card.append(note);
        if(job.chat){const open=document.createElement('button');open.textContent='Open work';open.onclick=()=>{dialog.close();select(job.chat);};card.append(open);}
        if(job.state==='needs_attention'&&job.chat){const recheck=document.createElement('button');recheck.textContent='Recheck result';recheck.onclick=async()=>{try{await api('/api/goals',{action:'recheck',id:job.id});await refresh();}catch(e){document.querySelector('#goal-error').textContent=e.message;}};card.append(recheck);}
        const action=['queued','running'].includes(job.state)?'cancel':['completed','cancelled','needs_attention'].includes(job.state)?'archive':null;
        if(action){const control=document.createElement('button');control.textContent=action==='cancel'?'Cancel goal':'Archive';control.onclick=async()=>{try{await api('/api/goals',{action,id:job.id});await refresh();}catch(e){document.querySelector('#goal-error').textContent=e.message;}};card.append(control);}
        list.append(card);
      }
      if(!jobs.length)list.textContent='No saved goals yet.';
    }catch(e){document.querySelector('#goal-error').textContent=e.message;}finally{busy=false;}
  }
  button.onclick=()=>{dialog.showModal();refresh();};
  document.querySelector('#goal-add').onclick=async()=>{
    const add=document.querySelector('#goal-add');add.disabled=true;
    try{
      const message=document.querySelector('#goal-instructions').value.trim();if(!message)throw Error('Describe a goal first.');
      const execution=executionConfig();for(const [key,value] of Object.entries({max_steps:64,timeout_seconds:900,max_tokens:100000,max_review_rounds:3}))if(!execution[key])execution[key]=value;
      const request={message,provider:providerConfig(),reviewer:reviewerConfig(),execution,...reportContract(message),learn_procedures:document.querySelector('#learn-procedures').checked,access:{web:document.querySelector('#access-web').checked,terminal:document.querySelector('#access-terminal').checked,apps:document.querySelector('#access-apps').checked,desktop:document.querySelector('#access-desktop').checked}};
      if(request.access.terminal)request.workspace=document.querySelector('#host-workspace').value.trim();
      const verified=!document.querySelector('#goal-reviewed').checked,after=document.querySelector('#goal-after-current').checked;
      const signature=JSON.stringify({request,verified,after});
      if(!pendingSubmission||pendingSubmission.signature!==signature)pendingSubmission={signature,job:{id:crypto.randomUUID(),request,require_verified:verified,depends_on:after?jobs.filter(j=>j.state!=='completed').map(j=>j.id):[]}};
      await api('/api/goals',{action:'submit',jobs:[pendingSubmission.job]});pendingSubmission=null;
      document.querySelector('#goal-instructions').value='';document.querySelector('#goal-error').textContent='Goal saved.';await refresh();
    }catch(e){document.querySelector('#goal-error').textContent=e.message;}finally{add.disabled=false;}
  };
  setInterval(refresh,2000);
})();
