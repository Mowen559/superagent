// Controlled model/real processes: snapshot collection, restart and cleanup.
// This does not certify real-model autonomy or business correctness.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const assert=require('node:assert/strict'),{spawn}=require('node:child_process');
const [hubArg,bundleArg,stageArg]=process.argv.slice(2);if(!stageArg)throw Error('usage: HUB BUNDLE STAGE');
const hub=path.resolve(hubArg),bundle=path.resolve(bundleArg),root=path.resolve(stageArg),project=path.join(root,'project'),data=path.join(root,'data');
const port=39890,modelPort=39891,requests=[],observations=[],proofs=[],logs=[];
fs.mkdirSync(project,{recursive:true});fs.mkdirSync(path.join(data,'settings'),{recursive:true});
for(const name of ['alpha','beta']){
 fs.writeFileSync(path.join(project,name+'.js'),`module.exports = 'original-${name}';\n`);
 fs.writeFileSync(path.join(project,'check-'+name+'.cjs'),`require('node:assert/strict').equal(require('./${name}.js'),'changed-${name}');\n`);
}
fs.writeFileSync(path.join(data,'settings/llm.json'),JSON.stringify({enabled:true,provider:'openai',baseUrl:`http://127.0.0.1:${modelPort}/v1`,apiKey:'controlled-fixture-only',model:'gemini-3.8-flash-high',fastModel:'gemini-3.8-flash-high',accounts:[],roleModels:{},maxTokens:1024,temperature:0,uaFileLimit:48,uaSampleFileLimit:12,uaMaxFileChars:60000,updatedAt:'2026-11-06T00:00:00Z'}));
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function poll(fn,ms=25000){const end=Date.now()+ms;while(Date.now()<end){const v=await fn();if(v)return v;await sleep(150);}throw Error('bounded delivery acceptance poll expired');}
async function api(url,body,method){const r=await fetch(`http://127.0.0.1:${port}${url}`,{method:method||(body?'POST':'GET'),headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined,signal:AbortSignal.timeout(6000)});const raw=await r.text();let value;try{value=JSON.parse(raw);}catch{value={rejectionText:raw};}observations.push({url,status:r.status});return{status:r.status,value};}
function reply(res,body,message){modelResponses++;const finish=message.tool_calls?'tool_calls':'stop';if(body.stream){res.writeHead(200,{'Content-Type':'text/event-stream'});res.write('data: '+JSON.stringify({id:'controlled',object:'chat.completion.chunk',choices:[{index:0,delta:{role:'assistant',...message},finish_reason:null}]})+'\n\n');res.write('data: '+JSON.stringify({id:'controlled',choices:[{index:0,delta:{},finish_reason:finish}],usage:{prompt_tokens:30,completion_tokens:10,total_tokens:40}})+'\n\n');res.end('data: [DONE]\n\n');}else{res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({id:'controlled',model:body.model,choices:[{index:0,message:{role:'assistant',...message},finish_reason:finish}],usage:{prompt_tokens:30,completion_tokens:10,total_tokens:40}}));}}

const crypto=require('node:crypto');
const GOOD="module.exports = input => Array.isArray(input) ? input.filter(x=>typeof x==='string').map(x=>x.trim()).filter(Boolean) : [];";
const md=code=>`---\nname: normalize-proposal\ndescription: unverified normalization proposal\n---\n\n\x60\x60\x60js\n${code}\n\x60\x60\x60\n`;
let modelRequests=0,modelResponses=0;
let service,active,step=0;const tasks=[],known={},checks=[];let wantReadable=false,wantMemory=false;
function parsed(content){try{return JSON.parse(content);}catch{return {};}}
const call=(name,args)=>({content:null,tool_calls:[{id:`${active}-${step++}`,type:'function',function:{name,arguments:JSON.stringify(args)}}]});
const mock=http.createServer(async(req,res)=>{try{let raw='';for await(const x of req)raw+=x;const b=JSON.parse(raw);modelRequests++;const names=(b.tools||[]).map(t=>t.function?.name);for(const required of ["skill_tool","super_memory","hermes_workflow","capability_status"])assert.ok(names.includes(required),required+" missing from actual model tool schema");const tools=b.messages.filter(m=>m.role==='tool');
 if(active==='good'||active==='bad'){
  if(step===0)return reply(res,b,call('skill_tool',{action:active==='good'?'save':'update',skill_name:'normalize-proposal',description:'Unverified normalization proposal',instructions:md(active==='good'?GOOD:'module.exports = input => input;')}));
  const result=parsed(tools.at(-1).content);assert.equal(result.status,'candidate');known[active]=result;
  return reply(res,b,{content:'隔离技能候选已保存。它还没有通过不同目标的独立留出检查，因此尚未激活，也没有修改现有技能文件；后续应通过统一管理入口验证。'});
 }
 if(active==='memory'){
  if(step===0)return reply(res,b,call('hermes_workflow',{Action:'record_lesson',Domain:'normalize',Query:'normalize proposal',TrapPattern:'input normalization edge cases',GoldenSolution:md(GOOD)}));
  const result=parsed(tools.at(-1).content);assert.equal(result.status,'candidate');known.memory=result;
  return reply(res,b,{content:'经验草稿已保存为隔离候选，还没有取得独立留出检查收据，不能当作已验证经验或活动规则。后续晋升或撤销应通过统一治理入口处理。'});
 }
 if(active==='inspect'){
  if(step===0)return reply(res,b,call('skill_tool',{action:'read',skill_name:known.good.skill_name}));
  const raw=tools.at(-1).content;assert.equal(raw.includes('module.exports = input'),wantReadable);checks.push({phase:'skill-read',available:wantReadable});
  return reply(res,b,{content:wantReadable?'已通过实际只读工具读取当前有效技能正文，说明它此时可用。这只表示当前受信套件的限定验证仍有效，不是通用能力认证。':'当前只读工具未能读取该技能；它可能仍在候选区、已撤销或证据已失效。我不会猜测正文，也不会自动创建或激活它。'});
 }
 if(active==='memory-read'){
  if(step===0)return reply(res,b,call('super_memory',{action:'retrieve',key:known.memory.candidate_id}));
  const result=parsed(tools.at(-1).content);assert.equal((result.results||[]).length>0,wantMemory);checks.push({phase:'memory-read',available:wantMemory});
  return reply(res,b,{content:wantMemory?'已从当前来源项目实际读回有效的候选经验正文；这是受信留出套件限定的记录，使用前仍需核对当前目标和授权。':'当前项目没有可召回的这条经验；尚未验证或撤销的候选不会自动参与召回。本轮仅查询，没有写入新记忆。'});
 }
 if(active==='hermes-read'){
  if(step===0)return reply(res,b,call('hermes_workflow',{Action:'query_lessons',Query:'normalize'}));
  const result=parsed(tools.at(-1).content);assert.equal(result.matched_count,wantMemory?1:0);if(wantMemory)assert.equal(result.lessons[0].id,known.memory.candidate_id);checks.push({phase:'hermes-read',available:wantMemory});
  return reply(res,b,{content:'当前项目规范化经验查询命中 '+result.matched_count+' 条。只有当前有效的治理经验才参与此查询；旧计数或尚未晋升的草稿不能作为已验证依据。'});
 }
 if(active==='memory-status'){
  if(step===0)return reply(res,b,call('capability_status',{}));
  const result=parsed(tools.at(-1).content),count=result.supermemory.project_notes.eligible_records;assert.equal(count,wantMemory?1:0);checks.push({phase:'memory-readiness',eligibleRecords:count});
  return reply(res,b,{content:'当前项目可召回经验为 '+count+' 条。该数量只表示当前可用性，不能代表经验语义正确、已经解决业务或具备通用迁移能力。'});
 }
 if(active==='memory-revoke'){
  if(step===0)return reply(res,b,call('super_memory',{action:'revoke',key:known.memory.candidate_id}));
  return reply(res,b,{content:'已按工具结果撤销这条项目经验；后续召回应排除它。原始候选与验证历史保留，没有删除其他项目笔记或用户技能文件。'});
 }
 throw Error('unknown controlled phase');
 }catch(e){res.writeHead(500);res.end(JSON.stringify({error:String(e)}));}});
async function start(){service=spawn(hub,['service',String(port)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:bundle}});service.stdout.on('data',x=>logs.push(x));service.stderr.on('data',x=>logs.push(x));await poll(async()=>{try{return(await api('/api/swarm/status')).status===200;}catch{return false;}});}
async function stop(){if(service){const p=service;service=null;p.kill();await Promise.race([new Promise(r=>p.once('exit',r)),sleep(3000)]);}}
async function run(phase){active=phase;step=0;const prompt=phase==='good'||phase==='bad'?'使用skill_tool保存规范化工作流草稿，说明这是尚未激活的隔离候选。':phase==='memory'?'使用hermes_workflow记录规范化经验草稿；本轮仅保存未验证候选，不宣称已经激活。':phase==='inspect'?'只读检查指定技能是否可以读取；找不到就如实说明，不创建或修改文件。':phase==='hermes-read'?'只读查询当前项目规范化经验，按实际命中说明，不保存或修改内容。':phase==='memory-status'?'只读查看当前项目可用记忆记录数量，按实际状态说明，不写入笔记。':phase==='memory-revoke'?'使用super_memory撤销指定项目经验；不要修改业务文件。':'只读查询指定项目经验，找不到就如实说明，不存新笔记。';
 const r=await api('/api/agents/run',{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:phase==='hermes-read'?'plan':'yolo',timeoutSeconds:60,prompt});tasks.push(r.value.taskId);
 const t=await poll(async()=>{const t=(await api('/api/tasks/'+r.value.taskId)).value;return ['completed','failed','cancelled'].includes(t.status)?t:null;},65000);
 proofs.push({phase,taskId:t.id,status:t.status});assert.equal(t.status,'completed',JSON.stringify(t));return t;
}
const candidates=async()=>{const r=await api('/api/evolution/candidates?projectPath='+encodeURIComponent(project));assert.equal(r.status,200);return r.value;};
async function action(c,act,suiteId){const payload={projectPath:project,action:act,expectedRevision:c.revision};if(suiteId)payload.suiteId=suiteId;return api('/api/evolution/candidates/'+c.id+'/actions',payload);}
const sha=buf=>crypto.createHash('sha256').update(buf).digest('hex');
function targetFingerprint(target){const first=crypto.createHash('sha256');for(const name of fs.readdirSync(target).sort()){const bytes=fs.readFileSync(path.join(target,name)),n=Buffer.alloc(8);n.writeBigUInt64LE(BigInt(bytes.length));first.update(name);first.update(Buffer.from([0]));first.update(n);first.update(sha(bytes));}return sha(first.digest());}
(async()=>{let error;try{
 const training=path.join(project,'training.json');fs.writeFileSync(training,'{"source":"training-fixture"}');const trainingBytes=fs.readFileSync(training);
 const userSkill=path.join(project,'.agents/skills/user-kept/SKILL.md');fs.mkdirSync(path.dirname(userSkill),{recursive:true});fs.writeFileSync(userSkill,'---\nname: user-kept\ndescription: user owned\n---\nKeep this file.');const userBytes=fs.readFileSync(userSkill);
 const target=path.join(root,'heldout-target'),oracle=path.join(root,'trusted/oracle.cjs');fs.mkdirSync(target,{recursive:true});fs.mkdirSync(path.dirname(oracle),{recursive:true});
 fs.writeFileSync(path.join(target,'cases.json'),'[[null,[]],[[" A ",7,null,"B"," "],["A","B"]],[[""," z "],["z"]]]');
 fs.writeFileSync(oracle,"const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');const [file,target,out]=process.argv.slice(2);let text=fs.readFileSync(file,'utf8');if(file.endsWith('.json'))text=JSON.parse(text).summary;const match=text.match(/```js\\n([\\s\\S]*?)\\n```/);assert.ok(match,'missing actual proposal');const sandbox={module:{exports:null}};vm.runInNewContext(match[1],sandbox,{timeout:500});for(const [input,expected] of JSON.parse(fs.readFileSync(target+'/cases.json','utf8'))){assert.deepEqual(JSON.parse(JSON.stringify(sandbox.module.exports(input))),expected);}fs.writeFileSync(out+'/oracle-proof.json',JSON.stringify({assertionsPassed:true,cases:3}));");
 const profile=path.join(data,'evolution/holdout-profiles.json');fs.mkdirSync(path.dirname(profile),{recursive:true});fs.writeFileSync(profile,JSON.stringify({schemaVersion:1,suites:[{id:'normalization',timeoutSeconds:10,cases:[{id:'different-target-cases',targetPath:target,targetFingerprint:targetFingerprint(target),oraclePath:oracle,oracleSha256:sha(fs.readFileSync(oracle))}]}]}));
 await new Promise((resolve,reject)=>{mock.once('error',reject);mock.listen(modelPort,'127.0.0.1',resolve);});await start();await run('good');
 let list=await candidates(),good=list.candidates.find(c=>c.id===known.good.candidate_id);assert.ok(good);assert.equal(good.state,'candidate');assert.ok(!list.effectiveActiveIds.includes(good.id));
 assert.equal((await action(good,'promote')).status,409);
 const forged=await api('/api/evolution/candidates/'+good.id+'/actions',{projectPath:project,action:'promote',expectedRevision:good.revision,passed:true});assert.ok([400,422].includes(forged.status));
 wantReadable=false;await run('inspect');let r=await action(good,'validate','normalization');assert.equal(r.status,200,JSON.stringify(r));good=r.value.candidate;assert.equal(good.validation.cases[0].exitCode,0);
 r=await action(good,'promote');assert.equal(r.status,200);good=r.value.candidate;wantReadable=true;await run('inspect');
 const oldPayload=fs.readFileSync(known.good.path);await run('bad');list=await candidates();let bad=list.candidates.find(c=>c.id===known.bad.candidate_id);r=await action(bad,'validate','normalization');assert.equal(r.status,422);bad=r.value.candidate;assert.equal(bad.lastResult,'holdout_check_failed');assert.equal((await action(bad,'promote')).status,409);
 list=await candidates();assert.ok(list.effectiveActiveIds.includes(good.id));assert.ok(oldPayload.equals(fs.readFileSync(known.good.path)));
 fs.writeFileSync(training,'changed training source');assert.ok(!(await candidates()).effectiveActiveIds.includes(good.id));wantReadable=false;await run('inspect');fs.writeFileSync(training,trainingBytes);
 r=await action(good,'revoke');assert.equal(r.status,200);good=r.value.candidate;assert.equal((await action(good,'promote')).status,409);wantReadable=false;await run('inspect');
 await run('memory');wantMemory=false;await run('memory-read');await run('memory-status');await run('hermes-read');list=await candidates();let memory=list.candidates.find(c=>c.id===known.memory.candidate_id);assert.ok(memory);r=await action(memory,'validate','normalization');assert.equal(r.status,200,JSON.stringify(r));memory=r.value.candidate;r=await action(memory,'promote');assert.equal(r.status,200);wantMemory=true;await run('memory-read');await run('memory-status');await run('hermes-read');
 await run('memory-revoke');list=await candidates();assert.equal(list.candidates.find(c=>c.id===memory.id).state,'revoked');wantMemory=false;await run('memory-read');await run('memory-status');await run('hermes-read');
 assert.ok(userBytes.equals(fs.readFileSync(userSkill)));assert.ok(trainingBytes.equals(fs.readFileSync(training)));assert.ok(!(await candidates()).effectiveActiveIds.length);
 fs.writeFileSync(path.join(root,'acceptance.json'),JSON.stringify({ok:true,scope:'受控模型保存宿主测试草稿；真实Hub/Node留出断言、管理接口、跨Run加载及撤销；不认证真实模型迁移',proofs,checks,known,finalCandidates:list,observations,modelRequests,modelResponses},null,2));
 }catch(e){error=e;fs.writeFileSync(path.join(root,'acceptance-failed.json'),JSON.stringify({error:e.stack,proofs,checks,known,observations,modelRequests,modelResponses},null,2));}
 finally{for(const id of tasks)await api('/api/tasks/'+id+'/cancel',{},'POST').catch(()=>{});await stop();mock.closeAllConnections();await new Promise(r=>mock.close(r));fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(logs));}
 if(error){console.error(error.message);process.exitCode=1;}else console.log('candidate governance live acceptance passed');
})().catch(e=>{console.error(e.message);process.exitCode=1;});
