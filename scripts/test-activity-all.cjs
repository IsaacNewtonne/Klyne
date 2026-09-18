const {spawnSync}=require('child_process');
for(const args of [['scripts/test-welcome-effects.cjs'],['scripts/test-activity-state.cjs'],['scripts/test-activity-ui.cjs']]){
 const result=spawnSync(process.execPath,args,{stdio:'inherit'});
 if(result.status!==0)process.exit(result.status||1);
}
