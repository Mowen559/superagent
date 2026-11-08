// Actual upstream model trial. Host creates requirements/tests, never implementation.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const {spawn,spawnSync}=require('node:child_process');
const [hubArg,bundleArg,stageArg,settingsArg]=process.argv.slice(2);
if(!settingsArg)throw Error('usage: HUB BUNDLE STAGE EXISTING_SETTINGS');
const hub=path.resolve(hubArg),bundle=path.resolve(bundleArg),root=path.resolve(stageArg),project=path.join(root,'project'),data=path.join(root,'data'),port=39882;
const maxTurns=Number(process.env.HUB_SWARM_EVAL_MAX_TURNS||24);
assert.ok(Number.isInteger(maxTurns)&&maxTurns>=24&&maxTurns<=40,'evaluation turns must be within 24..40');
fs.mkdirSync(project,{recursive:true});fs.mkdirSync(path.join(data,'settings'),{recursive:true});
const config=JSON.parse(fs.readFileSync(settingsArg,'utf8').replace(/^\uFEFF/,''));assert.equal(config.model,'gemini-3.8-flash-high');
// Freeze the explicitly requested model on the locally preflighted OpenAI
// compatibility route. Only this isolated trial copy is changed.
config.provider='openai';config.baseUrl=config.baseUrl.replace(/\/$/,'').replace(/\/v1$/,'')+'/v1';
config.fastModel='gemini-3.8-flash-high';config.accounts=[];config.roleModels={};
fs.writeFileSync(path.join(data,'settings/llm.json'),JSON.stringify(config));
for(const [file,name] of [['normalize-jobs.js','normalizeJobs'],['sort-jobs.js','sortJobs'],['index.js','orderJobs']])fs.writeFileSync(path.join(project,file),`exports.${name} = function (jobs) { throw new Error('TODO'); };\n`);
const tests=`const a=require('node:assert/strict'),{normalizeJobs}=require('./normalize-jobs'),{sortJobs}=require('./sort-jobs'),{orderJobs}=require('./index');
a.deepEqual(normalizeJobs([{id:'b',dependsOn:['a','a']},{id:'a'}]),[{id:'b',dependsOn:['a']},{id:'a',dependsOn:[]}]);
a.deepEqual(sortJobs([{id:'b',dependsOn:['a']},{id:'a',dependsOn:[]},{id:'c',dependsOn:[]}]),['a','b','c']);
a.deepEqual(orderJobs([{id:'b',dependsOn:['a']},{id:'a'}]),['a','b']);
a.throws(()=>orderJobs([{id:'a',dependsOn:['a']}]),/cycle/i);console.log('public checks passed');\n`;
fs.writeFileSync(path.join(project,'check-jobs.cjs'),tests);
const prompt=`请用两位子智能体并行完成这个小型开发任务：实现 normalize-jobs.js 的 normalizeJobs(jobs)，以及 sort-jobs.js 的 sortJobs(jobs)，你负责 index.js 的 orderJobs(jobs) 集成。CommonJS，零第三方依赖。
normalizeJobs：输入为任务对象数组；每项id必须是非空、无空白的字符串且不能重复；dependsOn省略视为空数组，否则必须为同样格式的字符串数组；引用未知id报错；依赖去重保留首次顺序；返回只含id与dependsOn的新对象数组，保持任务输入顺序且不修改输入。
sortJobs接收已规范化数组，返回任务id的拓扑序；每一步从所有当前可执行任务中选原始输入位置最前的一个，发现循环抛出包含cycle的错误。orderJobs先规范化再排序。
请使用隔离工作区委派和显式成果收集，保留并执行现有check-jobs.cjs，父任务在合并后独立检查与验证再交付。不要改动检查脚本。不需要额外功能或发布。`;
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
fs.writeFileSync(path.join(root,'frozen.json'),JSON.stringify({model:config.model,candidate:sha(hub),prompt,publicTestSha256:sha(path.join(project,'check-jobs.cjs')),budgetSeconds:1800,parentMaxTurns:maxTurns,leafMaxTurns:maxTurns,maxWorkers:2,interventions:0,author:'Codex wrote requirements/tests and TODO stubs only; actual implementation must be model/Hub authored'},null,2));
const sleep=ms=>new Promise(r=>setTimeout(r,ms));let service,taskId;const logs=[],observations=[];
async function api(url,body){const r=await fetch(`http://127.0.0.1:${port}${url}`,{method:body?'POST':'GET',headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):undefined,signal:AbortSignal.timeout(6000)});const value=await r.json();observations.push({at:Date.now(),url,status:r.status});if(!r.ok)throw Error('local HTTP '+r.status);return value;}
async function poll(fn,ms){const end=Date.now()+ms;while(Date.now()<end){const r=await fn();if(r)return r;await sleep(1000);}throw Error('bounded poll expired');}
(async()=>{const start=Date.now();let failure;try{
 service=spawn(hub,['service',String(port)],{cwd:project,windowsHide:true,env:{...process.env,HUB_APP_DATA_ROOT:data,HUB_BUNDLE_ROOT:bundle,HUB_AGENT_MAX_MODEL_TURNS:String(maxTurns)}});service.stdout.on('data',x=>logs.push(x));service.stderr.on('data',x=>logs.push(x));
 await poll(async()=>{try{return await api('/api/swarm/status');}catch{return false;}},20000);
 const run=await api('/api/agents/run',{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'yolo',prompt,timeoutSeconds:1800});taskId=run.taskId;fs.writeFileSync(path.join(root,'run.json'),JSON.stringify({taskId,start},null,2));
 const task=await poll(async()=>{const t=await api(`/api/tasks/${taskId}`),children=await api(`/api/swarm/runs/${taskId}/children`);fs.writeFileSync(path.join(root,'latest.json'),JSON.stringify({elapsedSeconds:Math.round((Date.now()-start)/1000),task:t,children},null,2));return ['completed','failed','timeout','cancelled'].includes(t.status)?t:null;},1830000);
 const children=await api(`/api/swarm/runs/${taskId}/children`);assert.equal(task.status,'completed',task.error);assert.equal(children.children.filter(c=>!c.handle.replacesSubagentId).length,2);const replaced=new Set(children.children.filter(c=>c.handle.replacesSubagentId).map(c=>c.handle.replacesSubagentId));const effective=children.children.filter(c=>!replaced.has(c.handle.subagentId));assert.equal(effective.length,2);assert.ok(children.children.every(c=>c.processReaped));assert.ok(effective.every(c=>c.state==='succeeded'&&c.collectionReceipt));for(const c of children.children.filter(c=>c.handle.replacesSubagentId)){const old=children.children.find(o=>o.handle.subagentId===c.handle.replacesSubagentId);assert.ok(old&&['failed','cancelled','timed_out'].includes(old.state));assert.equal(old.handle.role,c.handle.role);assert.equal(old.handle.parentRunId,c.handle.parentRunId);assert.equal(old.handle.toolPolicy,c.handle.toolPolicy);}
 const workspace=children.parent.workspacePath;assert.equal(sha(path.join(workspace,'check-jobs.cjs')),sha(path.join(project,'check-jobs.cjs')));
 const oracle=`const a=require('node:assert/strict'),p=process.argv[1],{orderJobs}=require(p+'/index'),{normalizeJobs}=require(p+'/normalize-jobs');
for(const v of [null,{},[{id:''}],[{id:'a b'}],[{id:'a'},{id:'a'}],[{id:'a',dependsOn:['missing']}],[{id:'a',dependsOn:'x'}]])a.throws(()=>normalizeJobs(v));
a.deepEqual(orderJobs([]),[]);a.deepEqual(orderJobs([{id:'x'},{id:'y'},{id:'z'}]),['x','y','z']);
a.deepEqual(orderJobs([{id:'z',dependsOn:['y']},{id:'a'},{id:'y'}]),['a','y','z']);
a.throws(()=>orderJobs([{id:'x',dependsOn:['y']},{id:'y',dependsOn:['x']}]),/cycle/i);
const input=[{id:'x'},{id:'y',dependsOn:['x','x']}],before=JSON.stringify(input);orderJobs(input);a.equal(JSON.stringify(input),before);console.log('independent oracle passed');`;
 const checked=spawnSync('node',['-e',oracle,workspace],{windowsHide:true,encoding:'utf8',timeout:20000});assert.equal(checked.status,0,checked.stderr);
 fs.writeFileSync(path.join(root,'outcome.json'),JSON.stringify({ok:true,elapsedSeconds:(Date.now()-start)/1000,taskId,children,oracle:checked.stdout,interventions:0,scope:'one actual-model dual-worker task, not general intelligence certification'},null,2));
 }catch(e){failure=e;fs.writeFileSync(path.join(root,'outcome.json'),JSON.stringify({ok:false,error:String(e.message).slice(0,2000),elapsedSeconds:(Date.now()-start)/1000,taskId,interventions:0},null,2));}
 finally{if(taskId)await api(`/api/tasks/${taskId}/cancel`,{}).catch(()=>{});if(service){const child=service;child.kill();await Promise.race([new Promise(r=>child.once('exit',r)),sleep(3000)]);}fs.writeFileSync(path.join(root,'hub.log'),Buffer.concat(logs));fs.writeFileSync(path.join(root,'observations.json'),JSON.stringify(observations,null,2));}
 console.log(failure?'actual model trial failed; see bounded outcome':'actual model dual-worker trial passed');if(failure)process.exitCode=1;
})().catch(e=>{console.error(e.message);process.exitCode=1;});
