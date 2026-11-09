// Controlled model / real foreground checker, edits, persisted recovery and false-positive rejection.
// This does not certify real-model autonomy or business correctness.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const [hubArg,bundleArg,stageArg]=process.argv.slice(2);if(!stageArg)throw Error('usage: HUB BUNDLE STAGE');
const hub=path.resolve(hubArg),bundle=path.resolve(bundleArg),root=path.resolve(stageArg),project=path.join(root,'project'),data=path.join(root,'data');
const port=39886,modelPort=39887,requests=[],observations=[],proofs=[],logs=[];
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
 if(active==='repair'||active==='write-fail'){
  if(phaseStep===0)return tool('run_command',{CommandLine:'node check-alpha.cjs'});
  if(phaseStep===1)return tool('view_file',{AbsolutePath:'alpha.js'});
  if(phaseStep===2)return tool('write_to_file',{TargetFile:'alpha.js',CodeContent:"module.exports = 'changed-alpha';\n",Overwrite:true});
  if(phaseStep===3)return tool('run_command',{CommandLine:'node check-alpha.cjs'});
  return reply(res,b,{content:'已依据实际检查失败修改alpha.js，并重新运行同一检查通过。原有检查脚本保持原样，修改已在当前工作目录验证完成。'});
 }
 if(active==='secret'){
  if(phaseStep===0)return tool('super_memory',{action:'store',key:'rejected_demo',content:'sk-test-memory-canary-12345678901234567890'});
  const toolMessages=b.messages.filter(m=>m.role==='tool');assert.ok(!JSON.stringify(toolMessages).includes('sk-test-memory-canary'));
  return reply(res,b,{content:'敏感示例被拒绝，没有保存这条笔记；不会在回复中重复敏感值。'});
 }
 if(phaseStep===0)return tool('view_file',{AbsolutePath:'diagnostic.txt'});
 return reply(res,b,{content:'这只是文本文档，包含error、failed和BUILD SUCCESS等字样；本轮没有执行修复或验证，因此没有已解决经验。'});
 }catch(e){res.writeHead(500);res.end(JSON.stringify({error:String(e)}));}});
async function start(){service=spawn(hub,['service',String(port)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:bundle}});service.stdout.on('data',x=>logs.push(x));service.stderr.on('data',x=>logs.push(x));await poll(async()=>{try{return(await api('/api/swarm/status')).status===200;}catch{return false;}});}
async function stop(){const child=service;service=null;if(child){child.kill();await Promise.race([new Promise(r=>child.once('exit',r)),sleep(3000)]);}}
async function run(phase){active=phase;phaseStep=0;
 const sessionDir=path.join(data,'agent-sessions');const priorSessions=new Set(fs.existsSync(sessionDir)?fs.readdirSync(sessionDir):[]);
 const r=await api('/api/agents/run',{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:phase==='plan'?'plan':'yolo',timeoutSeconds:90,
 prompt:(phase==='repair'||phase==='write-fail')?'alpha.js存在检查失败。请先运行原有node check-alpha.cjs，调查实际错误，修复实现并原样重跑检查。不要修改检查脚本，不需要创建任务清单。':phase==='secret'?'使用super_memory保存项目笔记敏感演示，若工具拒绝请如实说明，不要重复敏感内容。':'只读查看diagnostic.txt，解释文本含义；不运行命令、不改文件、不存笔记。'});
 assert.equal(r.status,200);tasks.push(r.value.taskId);
 const t=await poll(async()=>{const t=(await api('/api/tasks/'+r.value.taskId)).value;return ['completed','failed','cancelled'].includes(t.status)?t:null;},90000);
 if(phase==='secret')assert.ok(['completed','failed'].includes(t.status));else assert.equal(t.status,'completed',JSON.stringify(t));
 const newSessions=fs.readdirSync(sessionDir).filter(f=>f.endsWith('.events.jsonl')&&!priorSessions.has(f));
 const session=t.receipt?.sessionId||(()=>{assert.equal(newSessions.length,1,'failed Run must correlate to exactly one newly created session ledger');return newSessions[0].slice(0,-'.events.jsonl'.length);})();
 const file=path.join(sessionDir,session+'.events.jsonl');
 const events=fs.readFileSync(file,'utf8').trim().split(/\r?\n/).map(l=>JSON.parse(l));
 const audits=events.filter(e=>e.type==='MemoryPersistenceRecorded');assert.equal(audits.length,1,'one audit required per live run');
 const audit=audits[0].payload.receipt;assert.equal(audit.sessionId,session);assert.equal(audit.decisions.length,4);assert.ok(events.findIndex(e=>e.type==='RunFinished')<events.findIndex(e=>e.type==='MemoryPersistenceRecorded'));
 assert.ok(!JSON.stringify(audit).includes('sk-test-memory-canary'));
 const proof={phase,taskId:t.id,status:t.status,sessionId:session,audit};proofs.push(proof);return proof;
}
function memoryFiles(){const dir=path.join(data,'projects','.project-memory');if(!fs.existsSync(dir))return[];
 return fs.readdirSync(dir,{withFileTypes:true}).filter(x=>x.isDirectory()).flatMap(x=>['evolution.json','supermemory.json'].map(f=>path.join(dir,x.name,f)).filter(f=>fs.existsSync(f)));}
function lessons(){return memoryFiles().flatMap(f=>{const b=JSON.parse(fs.readFileSync(f,'utf8'));return(b.lessons||[]).filter(l=>l.recoveryEvidence).map(l=>({file:f,lesson:l}));});}
(async()=>{let error;try{
 fs.writeFileSync(path.join(project,'diagnostic.txt'),'error failed BUILD FAILURE BUILD SUCCESS; ordinary data only');
 const checker=fs.readFileSync(path.join(project,'check-alpha.cjs'),'utf8');
 await new Promise((resolve,reject)=>{mock.once('error',reject);mock.listen(modelPort,'127.0.0.1',resolve);});await start();await run('repair');
 const stored=lessons();assert.equal(stored.length,1);const c=stored[0].lesson.recoveryEvidence;
 const first=proofs[0].audit;assert.ok(first.decisions.slice(0,3).every(d=>d.status==='stored'&&d.recordIds.length&&d.readbackDigests.length===d.recordIds.length));
 assert.deepEqual(first.decisions[1].recordIds,[stored[0].lesson.id]);
 const snapshots=memoryFiles().map(file=>({file,bytes:fs.readFileSync(file)}));
 for(const phase of ['readonly','plan']){const p=await run(phase);assert.ok(p.audit.decisions.every(d=>d.status==='skipped'&&['read_only_task','read_only_policy'].includes(d.reason)));for(const item of snapshots)assert.ok(item.bytes.equals(fs.readFileSync(item.file)),'read-only task changed memory');}
 const moved=[];
 try {
  for(const item of snapshots){const backup=item.file+'.original';assert.ok(path.resolve(backup).startsWith(root+path.sep));assert.ok(!fs.existsSync(backup));fs.renameSync(item.file,backup);fs.mkdirSync(item.file);fs.writeFileSync(path.join(item.file,'sentinel'),'unchanged');moved.push({file:item.file,backup});}
  const blocked=await run('write-fail');assert.ok(blocked.audit.decisions.slice(0,3).every(d=>d.status==='failed'&&d.reason==='write_failed'&&!d.recordIds.length),'write failure must not claim stored');
 } finally {for(const item of moved){assert.equal(fs.readFileSync(path.join(item.file,'sentinel'),'utf8'),'unchanged');fs.unlinkSync(path.join(item.file,'sentinel'));fs.rmdirSync(item.file);fs.renameSync(item.backup,item.file);}}
 for(const item of snapshots)assert.ok(item.bytes.equals(fs.readFileSync(item.file)),'original memory changed after blocked write');
 const secret=await run('secret');assert.ok(secret.audit.decisions.every(d=>!JSON.stringify(d).includes('sk-test-memory-canary')));
 const notes=memoryFiles().filter(f=>f.endsWith('supermemory.json')).map(f=>JSON.parse(fs.readFileSync(f,'utf8')));assert.ok(notes.every(b=>!JSON.stringify(b).includes('sk-test-memory-canary')));
 assert.equal(fs.readFileSync(path.join(project,'check-alpha.cjs'),'utf8'),checker);
 assert.equal(fs.readFileSync(path.join(project,'alpha.js'),'utf8'),"module.exports = 'original-alpha';\n");
 fs.writeFileSync(path.join(root,'acceptance.json'),JSON.stringify({ok:true,scope:'受控HTTP模型；真实Hub、Node、隔离工作区、写盘失败与会话审计；不新增真实模型自主得分',proofs,stored,observations,modelResponses:requests.length},null,2));
 }catch(e){error=e;fs.writeFileSync(path.join(root,'acceptance-failed.json'),JSON.stringify({error:e.stack,proofs,observations,stored:lessons()},null,2));}
 finally{for(const id of tasks)await api('/api/tasks/'+id+'/cancel',{},'POST').catch(()=>{});await stop();mock.closeAllConnections();await new Promise(r=>mock.close(r));fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(logs));}
 if(error){console.error(error.message);process.exitCode=1;}else console.log('memory persistence audit live acceptance passed: 5 cases');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
