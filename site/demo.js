'use strict';
let submitting=false;
let snapshot;
const $=id=>document.getElementById(id);
let timers=[];
function render(){
  window.productionView.update({snapshot,selected:'demo',submitting:false,message:$('instruction').value,provider:snapshot.provider,access:{}});
  $('conversation').hidden=document.documentElement.classList.contains('production-on');
  $('conversation').replaceChildren(...snapshot.messages.map(message=>{
    const row=document.createElement('p'),label=document.createElement('strong');
    label.textContent=message.role==='user'?'You: ':'Klyne: ';row.append(label,document.createTextNode(message.text));return row;
  }));
  const working=['Planning','Working','Reviewing'].includes(snapshot.status);
  $('demo-stop').hidden=!working;$('demo-run').disabled=working;
}
function run(){
  timers.forEach(clearTimeout);timers=[];
  const goal=$('instruction').value.trim()||'Create a small project plan';
  snapshot={id:'demo',title:goal,status:'Planning',provider:{kind:'ollama',model:'Your local model'},access:{},messages:[{role:'user',text:goal}],tasks:[],evidence:[],activity:{kind:'model',role:'planner',agent:'Planner',started_at:Date.now()}};
  render();
  function later(ms,fn){timers.push(setTimeout(()=>{fn();render();},ms));}
  later(1400,()=>{snapshot.status='Working';snapshot.tasks=[{agent:'Shape the idea',instruction:'Clarify the scope and outline the work.',expected_result:'A focused outline',status:'Working',depends_on:[]},{agent:'Check the plan',instruction:'Review the outline for missing steps.',expected_result:'A practical next step',status:'Queued',depends_on:[1]}];snapshot.activity={kind:'model',role:'worker',agent:'Writer',started_at:Date.now()};});
  later(3600,()=>{snapshot.tasks[0].status='Done';snapshot.tasks[1].status='Working';snapshot.evidence=[{agent:'Writer',action:'sample_outline',ok:true,summary:'Sample outline prepared',data:'Demo evidence only'}];});
  later(5500,()=>{snapshot.tasks[1].status='Done';snapshot.status='Reviewing';snapshot.activity={kind:'model',role:'reviewer',agent:'Reviewer',started_at:Date.now()};});
  later(7300,()=>{snapshot.status='Completed';snapshot.activity=null;snapshot.messages.push({role:'assistant',agent:'Klyne',text:'Start small: define one useful outcome, build the simplest version, then check it against your goal.\n\nThis is a sample response showing the workspace—not a model-generated result.'});});
}
$('chat-form').addEventListener('submit',event=>{event.preventDefault();run();});
$('demo-stop').onclick=()=>{timers.forEach(clearTimeout);timers=[];snapshot.status='Stopped';snapshot.activity=null;snapshot.messages.push({role:'assistant',agent:'Klyne',text:'Demo stopped. Run it again whenever you like.'});render();};
new MutationObserver(()=>{$('conversation').hidden=document.documentElement.classList.contains('production-on');}).observe(document.documentElement,{attributes:true,attributeFilter:['class']});
run();
