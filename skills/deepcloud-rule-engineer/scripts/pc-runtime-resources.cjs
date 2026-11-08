// Discover the real PC runtime after skills are copied beside a Hub executable.
const fs=require('node:fs'),path=require('node:path');
function resolvePcRuntime(scriptDir=__dirname,env=process.env){
 const candidates=[];
 if(env.HIKER_PC_RUNTIME_ROOT){
  const explicit=path.resolve(env.HIKER_PC_RUNTIME_ROOT);
  if(!['hk-engine.js','hk-db.js','hiker-rule-parser.js'].every(f=>fs.existsSync(path.join(explicit,f))))throw Error('Explicit HIKER_PC_RUNTIME_ROOT is incomplete; no fallback was selected.');
  return fs.realpathSync.native(explicit);
 }
 let current=path.resolve(scriptDir);
 for(let i=0;i<12;i++){
  candidates.push(path.join(current,'native/runtime'),path.join(current,'deepcloud-ui/native/runtime'));
  const parent=path.dirname(current);if(parent===current)break;current=parent;
 }
 const selected=candidates.find(dir=>['hk-engine.js','hk-db.js','hiker-rule-parser.js'].every(f=>fs.existsSync(path.join(dir,f))));
 if(!selected)throw Error('PC production runtime unavailable. Set HIKER_PC_RUNTIME_ROOT to the actual native/runtime directory; simulation is not a replacement.');
 return fs.realpathSync.native(selected);
}
module.exports={resolvePcRuntime};
