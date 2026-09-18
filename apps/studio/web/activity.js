'use strict';
// One operation per ID, not per category: simultaneous calls can finish independently.
(() => {
  const listeners=new Set(), operations=new Map(), flashes=new Map(), failures=new Map();
  let selected=null, cursor=0, initialized=false, mode='idle', stage='Idle', connected=true;
  let observationCount=0, notice=0, cpuPercent=null, gpuPercent=null, expiryTimer=0, replaying=true;
  const modes={thinking:'working',reviewing:'working',idle:'idle',waiting:'waiting',working:'working',error:'error',complete:'complete',input:'input',disconnected:'disconnected'};
  const name=action=>typeof action==='string'?action:action?.tool || (action?.action?name(action.action):Object.keys(action||{})[0]||'');
  function category(action) {
    const n=name(action);
    if(/^(browser_|fetch_url|FetchUrl|fetch:)/i.test(n))return 'browser';
    if(/^desktop_/i.test(n))return 'desktop';
    if(/^app_(call|invoke|inspect|operations)$/i.test(n)){
      const target=action?.action || action;
      if(typeof target?.name==='string')return 'app:'+target.name;
    }
    if(/^(app_|mcp_)/i.test(n))return 'apps';
    if(/^(skill_|tool_|capability_)/i.test(n))return 'skills';
    if(/^(memory_|experience_)/i.test(n))return 'memory';
    if(/^(runtime_|self_improve)/i.test(n))return 'runtime';
    if(/^(run_shell|RunShell|shell:|process_)/i.test(n))return 'terminal';
    if(/^(read|write|list|search|find|patch|copy|move|rename|delete|create|hash|stat|file|directory)/i.test(n))return 'files';
    return null;
  }
  function label(action) {
    const target=action?.action || action;
    if(target?.tool==='app_invoke')return 'Calling '+(target.operation || target.name || 'app');
    if(target?.tool==='app_call')return (target.method || 'GET')+' '+(target.path || target.name || 'app');
    const n=name(action), words=n.replace(/([a-z])([A-Z])/g,'$1 $2').split(':')[0].replaceAll('_',' ');
    const known={browser_read:'Reading webpage',browser_open:'Opening browser',browser_navigate:'Opening page',browser_download:'Downloading',browser_click:'Interacting with page',run_shell:'Executing command',RunShell:'Executing command',read_file:'Reading file',write_file:'Writing file',memory_search:'Searching memory',memory_store:'Storing context'};
    return known[n] || (words?words[0].toUpperCase()+words.slice(1):'Tool operation');
  }
  function notify(){
    clearTimeout(expiryTimer);
    if(flashes.size)expiryTimer=setTimeout(()=>{expire(performance.now());notify();},Math.max(1,800-(performance.now()-Math.min(...[...flashes.values()].map(f=>f.at)))));
    for(const listener of listeners)listener(state());
  }
  function expire(now){for(const [id,f]of flashes)if(now-f.at>=800)flashes.delete(id);}
  function state(){return {mode,stage,connected,operations:[...operations.values()],flashes:[...flashes.values()],failures:[...failures.values()],observationCount,notice,cpuPercent,gpuPercent,replaying};}
  function emit(type,data={},replay=false) {
    const id=data.id, named=category(data.action), tool=data.connection?'app:'+data.connection:(!data.tool || data.tool==='apps')?(named||data.tool):data.tool;
    if(type==='system:gpu_updated')gpuPercent=Number.isFinite(data.gpu_percent)?Math.max(0,Math.min(100,data.gpu_percent)):null;
    if(type==='system:cpu_updated')cpuPercent=Number.isFinite(data.cpu_percent)?Math.max(0,Math.min(100,data.cpu_percent)):null;
    if(type==='tool:start' && id && tool) {failures.delete(tool);operations.set(id,{...data,tool,operation:data.operation||label(data.action),direction:data.direction||'outbound',status:'active'});}
    if(type==='tool:waiting' && operations.has(id)) operations.set(id,{...operations.get(id),status:'waiting',operation:data.operation||'Awaiting response'});
    if(['tool:complete','tool:error'].includes(type)) {
      const old=operations.get(id);operations.delete(id);
      if(type==='tool:error' && (tool||old?.tool))failures.set(tool||old.tool,{tool:tool||old.tool,status:'failed'});
      if(!replay && (tool||old?.tool)) flashes.set(id,{...old,...data,tool:tool||old.tool,status:type==='tool:error'?'failed':'complete',at:performance.now(),direction:'inbound'});
    }
    if(type==='tool:result' && operations.has(id)) operations.set(id,{...operations.get(id),direction:'inbound'});
    if(type==='system:connection_changed'){if(connected!==!!data.connected){cursor=0;initialized=false;}connected=!!data.connected;if(!connected){operations.clear();flashes.clear();cursor=0;initialized=false;mode='disconnected';}}
    if(type==='observation:new'){observationCount=data.count;if(!replay)notice=performance.now();}
    if(type==='workflow:stage_changed')stage=data.status||data.stage;
    if(type.startsWith('klyne:')) mode=modes[type.slice(6)]||'idle';
    if(!replay)window.dispatchEvent(new CustomEvent(type,{detail:data}));
  }
  function snapshot(input) {
    const chat=input.snapshot, changed=selected!==input.selected;
    if(changed){selected=input.selected;cursor=0;operations.clear();flashes.clear();failures.clear();initialized=false;notice=0;}
    replaying=changed || !initialized;
    const records=chat?.activity_events||[];
    const gap=cursor>0 && records.length && records[0].seq>cursor+1;
    if(gap){operations.clear();flashes.clear();initialized=false;replaying=true;}
    for(const event of records)if(event.seq>cursor){emit(event.type,event.data,!initialized);cursor=event.seq;}
    // Legacy snapshots can report a currently dispatched operation, but history
    // never becomes live activity. Approval/uncertainty is not execution.
    const running=['Planning','Working','Reviewing','Upgrading','Stopping'].includes(chat?.status);
    if(!records.length){
      const pending=running && chat.pending && !chat.pending.proposal?chat.pending:null;
      const id=pending?JSON.stringify(pending):null;
      for(const key of operations.keys())if(key!==id)operations.delete(key);
      if(id && !operations.has(id))emit('tool:start',{id,action:pending.action||pending,direction:'outbound'},true);
    }
    if(!running || !connected)operations.clear();
    if(chat)initialized=true;
    stage=chat?.status||'Idle';
    const previous=mode;
    mode=!connected?'disconnected':chat?.status==='Needs input'||chat?.pending?.proposal?'input':['Blocked','Interrupted'].includes(stage)?'error':stage==='Completed'?'complete':
      operations.size?([...operations.values()].every(o=>o.status==='waiting')?'waiting':'working'):
      running?(chat?.activity?'waiting':'working'):input.submitting?'waiting':'idle';
    if(previous!==mode && initialized && !changed)window.dispatchEvent(new CustomEvent('klyne:'+mode,{detail:{stage}}));
    observationCount=chat?.evidence?.length||0;
    notify();
  }
  window.klyneActivity={snapshot,state,category,label,emit(type,data){replaying=false;emit(type,data);if(type.startsWith('tool:') && connected)mode=operations.size?([...operations.values()].every(o=>o.status==='waiting')?'waiting':'working'):type==='tool:error'?'error':'waiting';notify();},subscribe(fn){listeners.add(fn);return()=>listeners.delete(fn);},expire};
})();
