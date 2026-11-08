// Controlled model/real processes: snapshot collection, restart and cleanup.
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
let service,active,storeTarget;const tasks=[];
function valueOf(s){try{return JSON.parse(s);}catch{return {};}}
const mock=http.createServer(async(req,res)=>{try{let raw='';for await(const x of req)raw+=x;const b=JSON.parse(raw),results=b.messages.filter(m=>m.role==='tool');
 if(!results.length)return reply(res,b,{content:null,tool_calls:[{id:'memory-operation',type:'function',function:{name:'super_memory',arguments:JSON.stringify(active==='store'?{action:'store',key:'scope_http_canary',content:'scope_http_value_20261108'}:{action:'retrieve',key:'scope_http_canary'})}}]});
 const result=valueOf(results.at(-1).content);requests.push({phase:active,result});
 if(active==='store') {assert.equal(result.status,'stored');storeTarget=result.store_target;}
 else if(active==='recall')assert.ok(JSON.stringify(result).includes('scope_http_value_20261108'));
 else assert.ok(!JSON.stringify(result).includes('scope_http_value_20261108'),'cross-project note leaked');
 reply(res,b,{content:active==='store'?'项目笔记已保存，工具已完成实际读回确认。':active==='recall'?'已从当前来源项目实际读回 scope_http_value_20261108，属于未验证的历史笔记。':'当前来源项目中没有这条笔记，未猜测内容。'});
 }catch(e){res.writeHead(500);res.end(JSON.stringify({error:String(e)}));}});
async function start(){service=spawn(hub,['service',String(port)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:bundle}});service.stdout.on('data',x=>logs.push(x));service.stderr.on('data',x=>logs.push(x));await poll(async()=>{try{return(await api('/api/swarm/status')).status===200;}catch{return false;}});}
async function stop(){const child=service;service=null;if(child){child.kill();await Promise.race([new Promise(r=>child.once('exit',r)),sleep(3000)]);}}
async function run(phase,origin){active=phase;const r=await api('/api/agents/run',{agent:'superagent',project:origin,model:'gemini-3.8-flash-high',workMode:'yolo',timeoutSeconds:60,prompt:phase==='store'?'请使用super_memory保存项目笔记：key=scope_http_canary，content=scope_http_value_20261108。本轮只操作项目笔记，无需修改业务代码。':'请使用super_memory精确读取项目笔记scope_http_canary，本轮只读取笔记，不修改任何内容；找不到就说明没有，不要猜测。'});tasks.push(r.value.taskId);const t=await poll(async()=>{const task=(await api('/api/tasks/'+r.value.taskId)).value;return ['completed','failed','timeout'].includes(task.status)?task:null;},65000);assert.equal(t.status,'completed',JSON.stringify(t));proofs.push({phase,taskId:t.id,status:t.status});}
(async()=>{let error;try{
 await new Promise((resolve,reject)=>{mock.once('error',reject);mock.listen(modelPort,'127.0.0.1',resolve);});await start();await run('store',project);
 assert.ok(path.resolve(storeTarget).startsWith(path.resolve(data,'projects','.project-memory')+path.sep),'store is not program-managed');assert.ok(!fs.existsSync(path.join(project,'.supermemory')));
 await stop();await start();await run('recall',project);
 const other=path.join(root,'other-project');fs.mkdirSync(other,{recursive:true});await run('other',other);
 const saved=JSON.parse(fs.readFileSync(storeTarget,'utf8'));assert.ok(JSON.stringify(saved).includes('scope_http_canary'));assert.equal(new Set(proofs.map(p=>p.taskId)).size,3);
 fs.writeFileSync(path.join(root,'acceptance.json'),JSON.stringify({ok:true,scope:'controlled HTTP model, actual independent Hub Runs and process restart; not real-model memory intelligence',proofs,storeTarget,requests,observations},null,2));
 }catch(e){error=e;fs.writeFileSync(path.join(root,'acceptance-failed.json'),JSON.stringify({error:e.stack,proofs,requests,observations},null,2));}
 finally{for(const id of tasks)await api('/api/tasks/'+id+'/cancel',{},'POST').catch(()=>{});await stop();mock.closeAllConnections();await new Promise(r=>mock.close(r));fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(logs));}
 if(error){console.error(error.message);process.exitCode=1;}else console.log('project memory live scope acceptance passed');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
