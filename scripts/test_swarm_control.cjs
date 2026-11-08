// Transport/lifecycle acceptance with a controlled model stub. This is NOT a
// real-model intelligence or business certification test.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {spawn}=require('node:child_process');
const assert=require('node:assert/strict');
const [hub,bundle,stage]=process.argv.slice(2);
const optionalGlobalGuard=process.argv[5];
if(!hub||!bundle||!stage)throw Error('usage: node test_swarm_control.cjs HUB BUNDLE STAGE');
const apiPort=39875,modelPort=39876;
const root=path.resolve(stage),project=path.join(root,'project'),data=path.join(root,'data');
fs.mkdirSync(project,{recursive:true});fs.mkdirSync(path.join(data,'settings'),{recursive:true});
fs.writeFileSync(path.join(project,'invoice.js'),'export function total(items) { return items.reduce((sum,item)=>sum+item.price*item.quantity,0); }\n');
fs.writeFileSync(path.join(data,'settings','llm.json'),JSON.stringify({enabled:true,provider:'openai',baseUrl:`http://127.0.0.1:${modelPort}/v1`,apiKey:'controlled-fixture-only',model:'gemini-3.8-flash-high',fastModel:'gemini-3.8-flash-high',accounts:[],roleModels:{},maxTokens:1024,temperature:0,uaFileLimit:48,uaSampleFileLimit:12,uaMaxFileChars:60000,updatedAt:'2026-11-06T00:00:00Z'}));
const transcript=[],requests=[],observations=[];
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function poll(fn,budget=20000){const until=Date.now()+budget;while(Date.now()<until){const v=await fn();if(v)return v;await sleep(150);}throw Error('bounded acceptance poll expired');}
async function api(url,body,method){const response=await fetch(`http://127.0.0.1:${apiPort}${url}`,{method:method||(body?'POST':'GET'),headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined,signal:AbortSignal.timeout(5000)});const value=await response.json();observations.push({url,status:response.status});return {status:response.status,value};}
function reply(res,body,message){
 const finish=message.tool_calls?'tool_calls':'stop';
 if(body.stream){res.writeHead(200,{'Content-Type':'text/event-stream'});res.write('data: '+JSON.stringify({id:'controlled',object:'chat.completion.chunk',choices:[{index:0,delta:{role:'assistant',...message},finish_reason:null}]})+'\n\n');res.write('data: '+JSON.stringify({id:'controlled',choices:[{index:0,delta:{},finish_reason:finish}],usage:{prompt_tokens:30,completion_tokens:10,total_tokens:40}})+'\n\n');res.end('data: [DONE]\n\n');}
 else{res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({id:'controlled',object:'chat.completion',model:body.model,choices:[{index:0,message:{role:'assistant',...message},finish_reason:finish}],usage:{prompt_tokens:30,completion_tokens:10,total_tokens:40}}));}
}
const mock=http.createServer(async(req,res)=>{try{
 let raw='';for await(const chunk of req)raw+=chunk;const body=JSON.parse(raw);const text=JSON.stringify(body.messages||[]);
 requests.push({path:req.url,model:body.model,at:Date.now(),tools:(body.tools||[]).map(t=>t.function.name),kind:text.includes('PARENT_CONTROL_HOLD')?'parent':text.includes('CHILD_HOLD')?'hold':'review'});
 if(text.includes('PARENT_CONTROL_HOLD')||text.includes('CHILD_HOLD'))return;
 if(!body.messages.some(m=>m.role==='tool'))reply(res,body,{content:null,tool_calls:[
  {id:'policy-denial',type:'function',function:{name:'write_to_file',arguments:JSON.stringify({TargetFile:'forbidden.txt',CodeContent:'must be denied'})}},
  {id:'shell-denial',type:'function',function:{name:'run_command',arguments:JSON.stringify({CommandLine:'echo BYPASS > shell-forbidden.txt'})}},
  {id:'physical-read',type:'function',function:{name:'view_file',arguments:JSON.stringify({AbsolutePath:'invoice.js'})}}
 ]});else reply(res,body,{content:'已读取 invoice.js。审查子任务的写入尝试被权限守卫拒绝；保持只读，未修改源文件。'});
 }catch(e){res.writeHead(500);res.end(JSON.stringify({error:String(e)}));}});
let child,taskId;
(async()=>{let error;const started=Date.now();try{
 await new Promise((resolve,reject)=>{mock.once('error',reject);mock.listen(modelPort,'127.0.0.1',resolve);});
 child=spawn(path.resolve(hub),['service',String(apiPort)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:path.resolve(bundle)}});
 child.stdout.on('data',d=>transcript.push(d));child.stderr.on('data',d=>transcript.push(d));
 await poll(async()=>{try{return (await api('/api/swarm/status')).status===200;}catch{return false;}});
 const launch=await api('/api/agents/run',{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'yolo',timeoutSeconds:120,prompt:'PARENT_CONTROL_HOLD 调查本项目，不要修改文件。'});
 assert.equal(launch.status,200);taskId=launch.value.taskId;assert.ok(taskId);
 const endpoint=`/api/swarm/runs/${taskId}/children`;
 const binding=await poll(async()=>{const v=(await api(endpoint)).value;return v.parent?.parentRunId?v:null;});
 assert.ok(binding.parent.parentSessionId);assert.equal(path.resolve(binding.parent.originProject),project);
 assert.equal(binding.parent.childReadOnly,true,'parent investigation scope was not frozen before admission');
 assert.equal(path.resolve(binding.parent.workspacePath),path.join(data,'runs',taskId,'workspace'));
 const first=await api(endpoint,{role:'opaque-custom-role',prompt:'CHILD_POLICY_PROBE 只读审查 invoice.js。',correlationId:'review-A'});
 const second=await api(endpoint,{role:'researcher',prompt:'CHILD_POLICY_PROBE 只读调查 invoice.js。',correlationId:'review-B'});
 assert.equal(first.status,202);assert.equal(second.status,202);assert.equal(first.value.executionStarted,false);
 const result=await poll(async()=>{const v=(await api(endpoint)).value;return v.children.length===2&&v.children.every(c=>c.processReaped&&['succeeded','failed'].includes(c.state))?v:null;},45000);
 for(const c of result.children){assert.equal(c.state,'succeeded',c.error);assert.ok(c.processId>0);assert.equal(c.handle.parentRunId,binding.parent.parentRunId);assert.equal(c.handle.parentSessionId,binding.parent.parentSessionId);assert.equal(c.handle.toolPolicy,'plan');assert.equal(c.businessVerified,false);assert.match(c.result,/拒绝/);}
 assert.equal(new Set(result.children.map(c=>c.processId)).size,2);assert.equal(fs.existsSync(path.join(binding.parent.workspacePath,'forbidden.txt')),false);
 assert.equal(fs.existsSync(path.join(binding.parent.workspacePath,'shell-forbidden.txt')),false,'read-only worker created a file via shell prefix');
 assert.ok(result.controlReceipts.filter(r=>r.receipt.ok).length>=2);
 const duplicate=await api(endpoint,{role:'opaque-custom-role',prompt:'CHILD_POLICY_PROBE 只读审查 invoice.js。',correlationId:'review-A'});assert.equal(duplicate.status,202);
 await poll(async()=>{const v=(await api(endpoint)).value;return v.controlReceipts.length>=3?v:null;});
 assert.equal((await api(endpoint)).value.children.length,2);
 const hanging=await api(endpoint,{role:'reviewer',prompt:'CHILD_HOLD 只读等待。',correlationId:'cancel-C'});assert.equal(hanging.status,202);
 const running=await poll(async()=>{const v=(await api(endpoint)).value;return v.children.find(c=>c.handle.correlationId==='cancel-C'&&c.state==='running'&&c.processId>0);});
 await poll(async()=>{const v=(await api(endpoint)).value;return v.children.find(c=>c.handle.subagentId===running.handle.subagentId&&c.lastHeartbeatAt);});
 assert.equal((await api(`${endpoint}/${running.handle.subagentId}/cancel`,{},'POST')).status,202);
 await poll(async()=>{const v=(await api(endpoint)).value;return v.children.find(c=>c.handle.subagentId===running.handle.subagentId&&c.state==='cancelled'&&c.processReaped);});
 const isolation=await api(endpoint,{role:'coder',prompt:'CHILD_POLICY_PROBE inspect isolated workspace',correlationId:'worktree-D',workspaceMode:'worktree'});assert.equal(isolation.status,202);
 const isolationSuccess=await poll(async()=>{const v=(await api(endpoint)).value;return v.children.find(c=>c.handle.correlationId==='worktree-D'&&c.state==='succeeded'&&c.processReaped);});
 assert.ok(isolationSuccess.processId>0);assert.notEqual(path.resolve(isolationSuccess.executionWorkspace),path.resolve(binding.parent.workspacePath));assert.equal(fs.readFileSync(path.join(isolationSuccess.executionWorkspace,'invoice.js'),'utf8'),fs.readFileSync(path.join(binding.parent.workspacePath,'invoice.js'),'utf8'));assert.ok(isolationSuccess.workspaceIdentity);
 await api(`/api/tasks/${taskId}/cancel`,{},'POST');
 const terminal=await poll(async()=>{const v=(await api(endpoint)).value;return v.observedOnly?v:null;});
 assert.equal(terminal.parentActive,false);
 assert.equal((await api(endpoint,{role:'reviewer',prompt:'inspect',correlationId:'too-late'})).status,409);
 const parentRequest=requests.find(r=>r.kind==='parent');assert.ok(parentRequest);
 for(const name of ['invoke_subagent','check_worker_status','intervene_worker','cancel_subagent','collect_subagent_changes','retry_subagent'])assert.ok(parentRequest.tools.includes(name),'parent model cannot see '+name);
 for(const req of requests.filter(r=>r.kind!=='parent'))for(const name of ['invoke_subagent','check_worker_status','intervene_worker','cancel_subagent','collect_subagent_changes','retry_subagent'])assert.ok(!req.tools.includes(name),'leaf model sees delegation tool '+name);
 if(optionalGlobalGuard){
  await new Promise((resolve,reject)=>{const guard=spawn(process.execPath,[path.resolve(optionalGlobalGuard),'--base',`http://127.0.0.1:${apiPort}`,'--out',path.join(root,'global-guard.json')],{windowsHide:true,stdio:['ignore','pipe','pipe']});const output=[];guard.stdout.on('data',d=>output.push(d));guard.stderr.on('data',d=>output.push(d));guard.once('error',reject);guard.once('exit',code=>{fs.writeFileSync(path.join(root,'global-guard.log'),Buffer.concat(output));code===0?resolve():reject(Error('frozen global guard rejected final candidate'));});});
 }
 fs.writeFileSync(path.join(root,'acceptance.json'),JSON.stringify({ok:true,scope:'controlled-model transport; not intelligence certification',elapsedMs:Date.now()-started,hub:path.resolve(hub),taskId,binding:binding.parent,children:terminal.children,requests,observations},null,2));
 }catch(e){error=e;fs.writeFileSync(path.join(root,'acceptance-failed.json'),JSON.stringify({ok:false,error:e.stack,taskId,requests,observations},null,2));}
 finally{
  if(taskId)await api(`/api/tasks/${taskId}/cancel`,{},'POST').catch(()=>{});
  if(child){child.kill();await Promise.race([new Promise(r=>child.once('exit',r)),sleep(3000)]);}
  mock.closeAllConnections();await new Promise(r=>mock.close(r));
  fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(transcript));
 }
 if(error){console.error(error.message);process.exitCode=1;}else console.log('controlled swarm transport acceptance passed');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
