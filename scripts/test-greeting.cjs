const fs=require('fs'),path=require('path'),{spawnSync}=require('child_process');
const temp=path.resolve('workspace/greeting-test-temp');fs.mkdirSync(temp,{recursive:true});
const binary=process.argv[2];
for(const filter of ['greeting_completes','completion_guard','model_response_recovers','emergency_stop_clears','clarification::tests']){
 const result=spawnSync(binary,[filter],{stdio:'inherit',env:{...process.env,TEMP:temp,TMP:temp}});
 if(result.status!==0)process.exit(result.status||1);
}
