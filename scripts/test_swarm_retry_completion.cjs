// Controlled model/real processes: snapshot collection, restart and cleanup.
// This does not certify real-model autonomy or business correctness.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const [hubArg,bundleArg,stageArg]=process.argv.slice(2);if(!stageArg)throw Error('usage: HUB BUNDLE STAGE');
const hub=path.resolve(hubArg),bundle=path.resolve(bundleArg),root=path.resolve(stageArg),project=path.join(root,'project'),data=path.join(root,'data');
const port=39880,modelPort=39881,requests=[],observations=[],proofs=[],logs=[];
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
let active,service;const tasks=[],turns=[];
const call=(name,args)=>({content:null,tool_calls:[{id:'gate-'+turns.length,type:'function',function:{name,arguments:JSON.stringify(args)}}]});
function find(value,key){if(typeof value==='string'){try{return find(JSON.parse(value),key);}catch{return;}}if(!value||typeof value!=='object')return;if(value[key]!==undefined)return value[key];for(const v of Object.values(value)){const found=find(v,key);if(found!==undefined)return found;}}
const mock=http.createServer(async(req,res)=>{try{
 let raw='';for await(const chunk of req)raw+=chunk;const body=JSON.parse(raw),text=JSON.stringify(body.messages),tools=body.messages.filter(m=>m.role==='tool');
 requests.push({at:Date.now(),messages:body.messages});
 if(!text.includes('PARENT_GATE_CASE')){
  if(!active.retryRequested){res.writeHead(400,{'Content-Type':'application/json'});return res.end(JSON.stringify({error:{message:'controlled initial worker failure'}}));}
  return reply(res,body,tools.length?{content:'已在隔离目录完成 alpha.js 修改并实际运行原有检查，成果等待父任务验收。'}:{content:null,tool_calls:[
   {id:'read',type:'function',function:{name:'view_file',arguments:JSON.stringify({AbsolutePath:'alpha.js'})}},
   {id:'write',type:'function',function:{name:'write_to_file',arguments:JSON.stringify({TargetFile:'alpha.js',CodeContent:"module.exports = 'changed-alpha';\n"})}},
   {id:'check',type:'function',function:{name:'run_command',arguments:JSON.stringify({CommandLine:'node check-alpha.cjs'})}}]});
 }
 await poll(()=>active?.id,5000);let state=(await api(active.endpoint)).value;const original=state.children.find(c=>!c.handle.replacesSubagentId);let child=state.children.find(c=>c.handle.replacesSubagentId===original?.handle.subagentId)||original;let message;
 if(!child)message=call('invoke_subagent',{Role:'coder',Prompt:'CHILD_GATE_WRITE 请修改alpha.js并运行原有检查。',WorkspaceMode:'worktree',Correlation_ID:'gate-child',TimeoutSeconds:30});
 else if(child===original&&child.processReaped&&child.state==='failed'&&!active.retryRequested){active.retryRequested=true;message=call('retry_subagent',{Subagent_ID:child.handle.subagentId});}
 else if(!child.processReaped)message=call('check_worker_status',{Subagent_ID:child.handle.subagentId,WaitSeconds:3});
 else if(active.kind==='text-only')message={content:'子任务已成功完成全部代码修改和验证，现在可以交付。'};
 else if(!active.inspected){active.inspected=true;message=call('collect_subagent_changes',{Subagent_ID:child.handle.subagentId,Action:'inspect'});}
 else if(!active.applied){const digest=[...tools].reverse().map(t=>find(t.content,'proposalDigest')).find(Boolean);if(!digest)throw Error('missing actual inspect digest');active.applied=true;message=call('collect_subagent_changes',{Subagent_ID:child.handle.subagentId,Action:'apply',ExpectedDigest:digest});}
 else if(!active.read){active.read=true;message=call('view_file',{AbsolutePath:'alpha.js'});}
 else if(!active.checked){active.checked=true;message=call('run_command',{CommandLine:'node check-alpha.cjs'});}
 else if(active.kind==='review-reject'&&!active.reviewed){active.reviewed=true;message=call('code_review',{mode:'files',files:['risk.js']});}
 else {if(active.kind==='stale'&&!active.injected){active.injected=true;fs.writeFileSync(path.join(state.parent.workspacePath,'alpha.js'),"module.exports = 'changed-after-check';\n");}message={content:'alpha.js 已修改并完成测试，保留原有检查脚本，所有实施和验收均已完成，可以交付。'};}
 turns.push({kind:active.kind,message});reply(res,body,message);
 }catch(e){res.writeHead(500);res.end(JSON.stringify({error:String(e)}));}});
async function startService(){service=spawn(hub,['service',String(port)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:bundle}});service.stdout.on('data',d=>logs.push(d));service.stderr.on('data',d=>logs.push(d));await poll(async()=>{try{return(await api('/api/swarm/status')).status===200;}catch{return false;}});}
(async()=>{let error;try{
 fs.writeFileSync(path.join(project,'risk.js'),"const password = 'controlled-review-fixture';\nmodule.exports = password;\n");
 await new Promise((resolve,reject)=>{mock.once('error',reject);mock.listen(modelPort,'127.0.0.1',resolve);});await startService();
 for(const kind of ['pass','review-reject','stale']){
  active={kind};const r=await api('/api/agents/run',{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'yolo',timeoutSeconds:80,prompt:`PARENT_GATE_CASE ${kind} 请委派一个子任务修改alpha.js，显式合并，再读取并运行已有测试完成开发交付。`});assert.equal(r.status,200);tasks.push(r.value.taskId);active.id=r.value.taskId;active.endpoint=`/api/swarm/runs/${active.id}/children`;
  const task=await poll(async()=>{const v=(await api(`/api/tasks/${active.id}`)).value;return ['completed','failed','timeout','cancelled'].includes(v.status)?v:null;},85000);
  const children=(await api(active.endpoint)).value.children;assert.equal(children.length,2);const failed=children.find(c=>!c.handle.replacesSubagentId),replacement=children.find(c=>c.handle.replacesSubagentId);assert.equal(failed.state,'failed');assert.equal(replacement.state,'succeeded');assert.equal(replacement.handle.replacesSubagentId,failed.handle.subagentId);assert.ok(children.every(c=>c.processReaped));
  assert.equal(task.status,kind==='pass'?'completed':'failed',JSON.stringify(task));
  if(kind==='text-only')assert.match(task.error,/uncollected artifacts/);
  if(kind==='review-reject'){
   assert.match(task.error,/code_review .* rejected or failed/);
   assert.ok(requests.some(r=>r.messages.some(m=>m.role==='tool'&&find(m.content,'critical_count')>0)),'actual critical review receipt missing');
  }
  if(kind==='stale')assert.match(task.error,/current code fingerprint/);
  const bound=(await api(active.endpoint)).value.parent;
  if(kind==='pass')assert.match(fs.readFileSync(path.join(bound.workspacePath,'alpha.js'),'utf8'),/changed-alpha/);
  proofs.push({kind,task,children});fs.writeFileSync(path.join(root,'progress.json'),JSON.stringify({proofs,turns},null,2));
 }
 assert.match(fs.readFileSync(path.join(project,'alpha.js'),'utf8'),/original-alpha/,'origin project was changed');
 assert.ok(requests.some(r=>r.messages.some(m=>m.role==='tool'&&find(m.content,'parent_workspace_verification')?.workspaceFingerprint)),'actual parent fingerprint receipt missing');
 fs.writeFileSync(path.join(root,'acceptance.json'),JSON.stringify({ok:true,scope:'controlled model, actual normal parent task completion and child process; not autonomy certification',hub,proofs,turns,observations},null,2));
 }catch(e){error=e;fs.writeFileSync(path.join(root,'acceptance-failed.json'),JSON.stringify({error:e.stack,active,proofs,turns,observations},null,2));}
 finally{for(const id of tasks)await api(`/api/tasks/${id}/cancel`,{},'POST').catch(()=>{});if(service){const child=service;child.kill();await Promise.race([new Promise(r=>child.once('exit',r)),sleep(3000)]);}mock.closeAllConnections();await new Promise(r=>mock.close(r));fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(logs));fs.writeFileSync(path.join(root,'model-requests.json'),JSON.stringify(requests,null,2));}
 if(error){console.error(error.message);process.exitCode=1;}else console.log('linked replacement parent completion passed: 3 cases');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
