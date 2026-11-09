// Controlled model / real foreground checker, edits, persisted recovery and false-positive rejection.
// This does not certify real-model autonomy or business correctness.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const [hubArg,bundleArg,stageArg]=process.argv.slice(2);if(!stageArg)throw Error('usage: HUB BUNDLE STAGE');
const hub=path.resolve(hubArg),bundle=path.resolve(bundleArg),root=path.resolve(stageArg),project=path.join(root,'project'),data=path.join(root,'data');
const port=39884,modelPort=39885,requests=[],observations=[],proofs=[],logs=[];
fs.mkdirSync(project,{recursive:true});fs.mkdirSync(path.join(data,'settings'),{recursive:true});
for(const name of ['alpha','beta']){
 fs.writeFileSync(path.join(project,name+'.js'),`module.exports = 'original-${name}';\n`);
 fs.writeFileSync(path.join(project,'check-'+name+'.cjs'),`require('node:assert/strict').equal(require('./${name}.js'),'changed-${name}');\n`);
}
fs.writeFileSync(path.join(data,'settings/llm.json'),JSON.stringify({enabled:true,provider:'openai',baseUrl:`http://127.0.0.1:${modelPort}/v1`,apiKey:'controlled-fixture-only',model:'gemini-3.8-flash-high',fastModel:'gemini-3.8-flash-high',accounts:[],roleModels:{},maxTokens:1024,temperature:0,uaFileLimit:48,uaSampleFileLimit:12,uaMaxFileChars:60000,updatedAt:'2026-11-06T00:00:00Z'}));
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function poll(fn,ms=25000){const end=Date.now()+ms;while(Date.now()<end){const v=await fn();if(v)return v;await sleep(150);}throw Error('bounded delivery acceptance poll expired');}
async function api(url,body,method){const r=await fetch(`http://127.0.0.1:${port}${url}`,{method:method||(body?'POST':'GET'),headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined,signal:AbortSignal.timeout(6000)});const value=await r.json();observations.push({url,status:r.status});return{status:r.status,value};}
function reply(res,body,message){const finish=message.tool_calls?'tool_calls':'stop';if(body.stream){res.writeHead(200,{'Content-Type':'text/event-stream'});res.write('data: '+JSON.stringify({id:'controlled',object:'chat.completion.chunk',choices:[{index:0,delta:{role:'assistant',...message},finish_reason:null}]})+'\n\n');res.write('data: '+JSON.stringify({id:'controlled',choices:[{index:0,delta:{},finish_reason:finish}],usage:{prompt_tokens:30,completion_tokens:10,total_tokens:40}})+'\n\n');res.end('data: [DONE]\n\n');}else{res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({id:'controlled',model:body.model,choices:[{index:0,message:{role:'assistant',...message},finish_reason:finish}],usage:{prompt_tokens:30,completion_tokens:10,total_tokens:40}}));}}
let service,active;const tasks=[];let phaseStep=0;
const mock=http.createServer(async(req,res)=>{try{let raw='';for await(const x of req)raw+=x;const b=JSON.parse(raw);
 assert.ok(!JSON.stringify(b.messages).includes('_execution_evidence'),'host evidence must not enter model context');
 const tool=(name,args)=>reply(res,b,{content:null,tool_calls:[{id:`${active}-${phaseStep++}`,type:'function',function:{name,arguments:JSON.stringify(args)}}]});
 requests.push({phase:active,step:phaseStep,tools:b.messages.filter(m=>m.role==='tool').map(m=>({id:m.tool_call_id,result:m.content}))});
 if(active==='repair'){
  if(phaseStep===0)return tool('run_command',{CommandLine:'node check-alpha.cjs'});
  if(phaseStep===1)return tool('view_file',{AbsolutePath:'alpha.js'});
  if(phaseStep===2)return tool('write_to_file',{TargetFile:'alpha.js',CodeContent:"module.exports = 'changed-alpha';\n",Overwrite:true});
  if(phaseStep===3)return tool('run_command',{CommandLine:'node check-alpha.cjs'});
  return reply(res,b,{content:'已依据实际检查失败修改alpha.js，并重新运行同一检查通过。原有检查脚本保持原样，修改已在当前工作目录验证完成。'});
 }
 if(phaseStep===0)return tool('view_file',{AbsolutePath:'diagnostic.txt'});
 return reply(res,b,{content:'这只是文本文档，包含error、failed和BUILD SUCCESS等字样；本轮没有执行修复或验证，因此没有已解决经验。'});
 }catch(e){res.writeHead(500);res.end(JSON.stringify({error:String(e)}));}});
async function start(){service=spawn(hub,['service',String(port)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:bundle}});service.stdout.on('data',x=>logs.push(x));service.stderr.on('data',x=>logs.push(x));await poll(async()=>{try{return(await api('/api/swarm/status')).status===200;}catch{return false;}});}
async function stop(){const child=service;service=null;if(child){child.kill();await Promise.race([new Promise(r=>child.once('exit',r)),sleep(3000)]);}}
async function run(phase){active=phase;phaseStep=0;
 const r=await api('/api/agents/run',{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'yolo',timeoutSeconds:90,
 prompt:phase==='repair'?'alpha.js存在检查失败。请先运行原有node check-alpha.cjs，调查实际错误，修复实现并原样重跑检查。不要修改检查脚本，不需要创建任务清单。':'只读查看diagnostic.txt，解释文本含义；不运行命令、不改文件、不存笔记。'});
 assert.equal(r.status,200);tasks.push(r.value.taskId);
 const t=await poll(async()=>{const t=(await api('/api/tasks/'+r.value.taskId)).value;return ['completed','failed','cancelled'].includes(t.status)?t:null;},90000);
 assert.equal(t.status,'completed',JSON.stringify(t));proofs.push({phase,taskId:t.id,status:t.status});
}
function memoryFiles(){const dir=path.join(data,'projects','.project-memory');if(!fs.existsSync(dir))return[];
 return fs.readdirSync(dir,{withFileTypes:true}).filter(x=>x.isDirectory()).flatMap(x=>['evolution.json','supermemory.json'].map(f=>path.join(dir,x.name,f)).filter(f=>fs.existsSync(f)));}
function lessons(){return memoryFiles().flatMap(f=>{const b=JSON.parse(fs.readFileSync(f,'utf8'));return(b.lessons||[]).filter(l=>l.recoveryEvidence).map(l=>({file:f,lesson:l}));});}
(async()=>{let error;try{
 fs.writeFileSync(path.join(project,'diagnostic.txt'),'error failed BUILD FAILURE BUILD SUCCESS; ordinary data only');
 const checker=fs.readFileSync(path.join(project,'check-alpha.cjs'),'utf8');
 await new Promise((resolve,reject)=>{mock.once('error',reject);mock.listen(modelPort,'127.0.0.1',resolve);});await start();await run('repair');
 const stored=lessons();assert.equal(stored.length,1,'one real recovery candidate required');const c=stored[0].lesson.recoveryEvidence;
 assert.equal(c.state,'candidate');assert.equal(c.failure.exitCode,1);assert.equal(c.validation.exitCode,0);assert.equal(c.failure.actionDigest,c.validation.actionDigest);
 assert.notEqual(c.revisions[0].fileRevision.beforeSha256,c.revisions[0].fileRevision.afterSha256);assert.ok(c.unverified.length);
 const reflected=memoryFiles().filter(f=>f.endsWith('supermemory.json')).flatMap(f=>{const b=JSON.parse(fs.readFileSync(f,'utf8'));return b.memories.filter(r=>(b.memory_context||{})[r.id]?.source==='automatic_reflection');});
 assert.equal(reflected.length,1,'postmortem reflection must persist the same candidate');
 const reflectionEvidence=JSON.parse(reflected[0].summary.split('Evidence: ')[1]);assert.equal(reflectionEvidence.failure.callId,c.failure.callId);assert.equal(reflectionEvidence.validation.callId,c.validation.callId);assert.equal(reflectionEvidence.failure.runId,c.failure.runId);
 await run('prose');assert.equal(lessons().length,1,'ordinary error words must not create recovery');
 assert.equal(fs.readFileSync(path.join(project,'check-alpha.cjs'),'utf8'),checker);
 assert.equal(fs.readFileSync(path.join(project,'alpha.js'),'utf8'),"module.exports = 'original-alpha';\n",'origin must remain unchanged by isolated run');
 fs.writeFileSync(path.join(root,'acceptance.json'),JSON.stringify({ok:true,scope:'controlled HTTP model and real Hub/Node/file edits; no new real-model autonomy claim',proofs,stored,reflected,requests,observations},null,2));
 }catch(e){error=e;fs.writeFileSync(path.join(root,'acceptance-failed.json'),JSON.stringify({error:e.stack,proofs,requests,observations,stored:lessons()},null,2));}
 finally{for(const id of tasks)await api('/api/tasks/'+id+'/cancel',{},'POST').catch(()=>{});await stop();mock.closeAllConnections();await new Promise(r=>mock.close(r));fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(logs));}
 if(error){console.error(error.message);process.exitCode=1;}else console.log('recovery candidate live acceptance passed');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
