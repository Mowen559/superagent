// Controlled model/real processes: snapshot collection, restart and cleanup.
// This does not certify real-model autonomy or business correctness.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const [hubArg,bundleArg,stageArg]=process.argv.slice(2);if(!stageArg)throw Error('usage: HUB BUNDLE STAGE');
const hub=path.resolve(hubArg),bundle=path.resolve(bundleArg),root=path.resolve(stageArg),project=path.join(root,'project'),data=path.join(root,'data');
const port=39878,modelPort=39879,requests=[],observations=[],proofs=[],logs=[];
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
const mock=http.createServer(async(req,res)=>{try{let raw='';for await(const chunk of req)raw+=chunk;const body=JSON.parse(raw),text=JSON.stringify(body.messages);const hold=text.includes('PARENT_HOLD')||text.includes('CHILD_HOLD');requests.push({at:Date.now(),kind:hold?'hold':text.includes('CHILD_WRITE_A')?'alpha':'beta'});if(hold)return;const name=text.includes('CHILD_WRITE_A')?'alpha':'beta';if(!body.messages.some(m=>m.role==='tool'))reply(res,body,{content:null,tool_calls:[
 {id:'read-'+name,type:'function',function:{name:'view_file',arguments:JSON.stringify({AbsolutePath:name+'.js'})}},
 {id:'write-'+name,type:'function',function:{name:'write_to_file',arguments:JSON.stringify({TargetFile:name+'.js',CodeContent:`module.exports = 'changed-${name}';\n`})}},
 {id:'check-'+name,type:'function',function:{name:'run_command',arguments:JSON.stringify({CommandLine:'node check-'+name+'.cjs'})}}
 ]});else reply(res,body,{content:`已在隔离工作区读取并修改 ${name}.js，运行原有 check-${name}.cjs 完成实际检查；没有修改检查脚本。成果等待父任务独立检查和合并，不代表总任务已通过业务验收。`});}catch(e){res.writeHead(500);res.end(JSON.stringify({error:String(e)}));}});
let service;const tasks=[];
async function startService(){service=spawn(hub,['service',String(port)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:bundle}});service.stdout.on('data',d=>logs.push(d));service.stderr.on('data',d=>logs.push(d));await poll(async()=>{try{return(await api('/api/swarm/status')).status===200;}catch{return false;}});}
async function stopService(){if(!service)return;const owned=service;service=undefined;owned.kill();await Promise.race([new Promise(r=>owned.once('exit',r)),sleep(3000)]);}
async function parent(timeout=120){const r=await api('/api/agents/run',{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'yolo',timeoutSeconds:timeout,prompt:'PARENT_HOLD 请实现并修改 alpha.js 和 beta.js，运行测试完成代码任务。'});assert.equal(r.status,200);tasks.push(r.value.taskId);const endpoint=`/api/swarm/runs/${r.value.taskId}/children`;const bound=await poll(async()=>{const v=(await api(endpoint)).value;return v.parent?.parentRunId?v:null;});assert.equal(bound.parent.childReadOnly,false);return{id:r.value.taskId,endpoint,bound};}
async function launch(p,key,prompt,mode='inherit'){assert.equal((await api(p.endpoint,{role:'coder',prompt,correlationId:key,workspaceMode:mode})).status,202);return poll(async()=>{const v=(await api(p.endpoint)).value;return v.children.find(c=>c.handle.correlationId===key&&c.processId>0);});}
async function collect(p,c,action,digest){const before=(await api(p.endpoint)).value.controlReceipts.length;const r=await api(`${p.endpoint}/${c.handle.subagentId}/changes`,{action,...(digest?{expectedDigest:digest}:{})});assert.equal(r.status,202);return poll(async()=>{const v=(await api(p.endpoint)).value;return v.controlReceipts.slice(before).find(r=>r.correlationId===`changes:${c.handle.subagentId}:${action}`)?.receipt;});}
(async()=>{let error;const started=Date.now();try{
 await new Promise((resolve,reject)=>{mock.once('error',reject);mock.listen(modelPort,'127.0.0.1',resolve);});await startService();
 const p=await parent();
 const a=await launch(p,'writer-A','CHILD_WRITE_A 请读取并修改alpha.js，运行原有检查。','worktree');
 const b=await launch(p,'writer-B','CHILD_WRITE_B 请读取并修改beta.js，运行原有检查。','worktree');
 const done=await poll(async()=>{const v=(await api(p.endpoint)).value;return v.children.length===2&&v.children.every(c=>c.processReaped&&['succeeded','failed'].includes(c.state))?v:null;},45000);
 for(const c of done.children){assert.equal(c.state,'succeeded',c.error);assert.ok(c.workspaceIdentity);assert.notEqual(path.resolve(c.executionWorkspace),path.resolve(p.bound.parent.workspacePath));}
 const parentRoot=p.bound.parent.workspacePath;assert.match(fs.readFileSync(path.join(parentRoot,'alpha.js'),'utf8'),/original-alpha/);
 const ia=await collect(p,a,'inspect');assert.equal(ia.ok,true,ia.error);assert.equal(ia.result.changes.conflicts.length,0);
 const applied=await collect(p,a,'apply',ia.result.proposalDigest);assert.equal(applied.ok,true,applied.error);assert.equal(applied.result.alreadyApplied,false);
 assert.match(fs.readFileSync(path.join(parentRoot,'alpha.js'),'utf8'),/changed-alpha/);assert.match(fs.readFileSync(path.join(project,'alpha.js'),'utf8'),/original-alpha/);
 const ib=await collect(p,b,'inspect');assert.equal(ib.ok,true,ib.error);
 fs.writeFileSync(path.join(parentRoot,'beta.js'),"module.exports = 'parent-conflict';\n");
 const conflict=await collect(p,b,'apply',ib.result.proposalDigest);assert.equal(conflict.ok,false);assert.match(conflict.error,/conflict/);assert.match(fs.readFileSync(path.join(parentRoot,'beta.js'),'utf8'),/parent-conflict/);
 fs.writeFileSync(path.join(parentRoot,'beta.js'),"module.exports = 'original-beta';\n");
 const fixed=await collect(p,b,'apply',ib.result.proposalDigest);assert.equal(fixed.ok,true,fixed.error);
 fs.writeFileSync(path.join(parentRoot,'beta.js'),"module.exports = 'after-apply-manual';\n");
 const duplicate=await collect(p,b,'apply',ib.result.proposalDigest);assert.equal(duplicate.ok,true);assert.equal(duplicate.result.alreadyApplied,true);assert.match(fs.readFileSync(path.join(parentRoot,'beta.js'),'utf8'),/after-apply-manual/);
 const callsBefore=requests.length;await stopService();await startService();
 const recovered=await poll(async()=>{const v=(await api(p.endpoint)).value;return !v.parentActive&&v.children.length===2?v:null;});
 assert.ok(recovered.children.every(c=>c.state==='succeeded'&&c.collectionReceipt));
 assert.equal((await api(`/api/tasks/${p.id}`)).value.status,'failed');
 assert.match(fs.readFileSync(path.join(parentRoot,'beta.js'),'utf8'),/after-apply-manual/);
 await sleep(300);assert.equal(requests.length,callsBefore,'restart replayed a model execution');
 proofs.push({case:'owned snapshot / fingerprint / conflict / idempotent apply / completed recovery',parent:p.id,children:recovered.children});
 const abandoned=await parent();await launch(abandoned,'orphan-A','CHILD_HOLD 等待。');await launch(abandoned,'orphan-B','CHILD_HOLD 等待。');
 await poll(async()=>{const v=(await api(abandoned.endpoint)).value;return v.children.length===2&&v.children.every(c=>c.lastHeartbeatAt)?v:null;});
 const calls=requests.length;await stopService();await startService();
 const interrupted=(await api(abandoned.endpoint)).value;assert.ok(interrupted.children.every(c=>c.state==='failed'&&c.processReaped));
 await sleep(300);assert.equal(requests.length,calls);proofs.push({case:'crashed owner / no orphan / no replay',parent:abandoned.id,children:interrupted.children});
 const timed=await parent(8);await launch(timed,'timeout-A','CHILD_HOLD 等待。');
 const timedResult=await poll(async()=>{const v=(await api(timed.endpoint)).value;return v.children.some(c=>c.state==='timed_out'&&c.processReaped)?v:null;},20000);
 await poll(async()=>{const v=(await api(`/api/tasks/${timed.id}`)).value;return v.status==='timeout'?v:null;});proofs.push({case:'parent timeout reconciles child',parent:timed.id,children:timedResult.children});
 const cancelled=await parent();await launch(cancelled,'cancel-A','CHILD_HOLD 等待。');await api(`/api/tasks/${cancelled.id}/cancel`,{},'POST');
 const cancelledResult=await poll(async()=>{const v=(await api(cancelled.endpoint)).value;return v.children.some(c=>c.state==='cancelled'&&c.processReaped)?v:null;});
 proofs.push({case:'parent cancel reconciles child',parent:cancelled.id,children:cancelledResult.children});
 fs.writeFileSync(path.join(root,'acceptance.json'),JSON.stringify({ok:true,scope:'controlled model, real Hub processes; not intelligence certification',hub,elapsedMs:Date.now()-started,proofs,requests,observations},null,2));
 }catch(e){error=e;fs.writeFileSync(path.join(root,'acceptance-failed.json'),JSON.stringify({ok:false,error:e.stack,tasks,proofs,requests,observations},null,2));}
 finally{for(const task of tasks)await api(`/api/tasks/${task}/cancel`,{},'POST').catch(()=>{});await stopService();mock.closeAllConnections();await new Promise(r=>mock.close(r));fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(logs));}
 if(error){console.error(error.message);process.exitCode=1;}else console.log('controlled swarm delivery and recovery acceptance passed');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
