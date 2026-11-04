const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..'),label=process.argv[2]||'hub-adaptive-v1';assert.match(label,/^[\w-]+$/);const stage=path.join(root,'artifacts',label);assert.ok(!fs.existsSync(stage));fs.mkdirSync(stage,{recursive:true});
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex'),binary=path.join(root,'target/debug/hub.exe');
const sourceNames=['src/domains/agent_cognitive/event_projection.rs','src/domains/agent_cognitive/executor.rs','src/domains/agent_cognitive/verification_gate.rs',...['cognitive_runtime','run_plan','run_facts','task_progress','tool_execution_gateway','adaptive_run_tests','delivery_evidence','mod'].map(n=>'src/services/brain/'+n+'.rs')];
const sourceFiles=sourceNames.map(n=>path.join(root,n));assert.ok(fs.statSync(binary).mtimeMs>=Math.max(...sourceFiles.map(p=>fs.statSync(p).mtimeMs)),'build candidate first');
const sourceSnapshot=sourceFiles.map(p=>{const copy=path.join(stage,'frozen/source',path.relative(root,p));fs.mkdirSync(path.dirname(copy),{recursive:true});fs.copyFileSync(p,copy);return {original:p,path:copy,sha256:hash(p)};});
const oracle=path.join(stage,'frozen/verify_adaptive_fixture.cjs');fs.copyFileSync(path.join(root,'scripts/verify_adaptive_fixture.cjs'),oracle);
const harnessFiles=['scripts/hub-adaptive-replay.cjs','scripts/run_adaptive_eval.ps1','scripts/prepare_adaptive_eval.cjs','../deepcloud-ui/src/lib/agent-sdk/client.ts','scripts/audit_adaptive_eval.cjs'].map(n=>path.resolve(root,n));
fs.writeFileSync(path.join(stage,'harness-frozen.json'),JSON.stringify(harnessFiles.map((p,i)=>{const copy=path.join(stage,'frozen/harness',`${i}-${path.basename(p)}`);fs.mkdirSync(path.dirname(copy),{recursive:true});fs.copyFileSync(p,copy);return {path:p,copy,sha256:hash(p)};}),null,2));
for(const kind of ['simple','complex']){
 const dir=path.join(stage,kind),project=path.join(dir,'project');fs.mkdirSync(project,{recursive:true});
 fs.writeFileSync(path.join(project,'USER_DATA.txt'),'existing user data must remain unchanged\n');
 const command=`node --test "${oracle}"`;
 let prompt,turns,seconds;
 if(kind==='simple'){
  fs.writeFileSync(path.join(project,'tier.cjs'),"module.exports=n=>n>8?'bulk':'single';\n");
  fs.writeFileSync(path.join(project,'README.md'),'tier.cjs导出单参数函数。数量至少8为bulk，小于8为single；保留导出接口和USER_DATA.txt。无第三方依赖。验收命令：'+command+'\n');
  prompt='tier.cjs把数量恰好8划进了single；边界应该算bulk。把这个临界值处理妥当，其他行为及导出接口保留。';turns=12;seconds=360;
 }else{
  fs.writeFileSync(path.join(project,'price.cjs'),"// USER_NOTE_KEEP_20261104\nexports.unitCost=(price,quantity)=>price;\n");
  fs.writeFileSync(path.join(project,'cart.cjs'),"const {unitCost}=require('./price.cjs');\nexports.total=items=>items.reduce((sum,item)=>sum+unitCost(item.price,item.quantity),0);\n");
  fs.writeFileSync(path.join(project,'README.md'),'price.cjs导出unitCost(price,quantity)，cart.cjs导出total(items)，条目含price和quantity。目标：数量至少8时单价九折；购物车按单价乘数量汇总；空购物车为0。保留已有导出接口、price.cjs中的用户注释和USER_DATA.txt。无第三方依赖。验收命令：'+command+'\n');
  prompt='先维护一份可核对的实施计划，执行中按证据更新同一份计划；理清price.cjs和cart.cjs两层职责，把批量九折和数量汇总接通，空购物车返回0，最后验收。我的用户注释、现有导出接口和数据文件都保留。';turns=24;seconds=720;
 }
 const steering=kind==='complex'?'补充一个要求：金额要在购物车最后汇总时四舍五入到两位小数，不能逐件提前取整。其他原有要求、用户注释和接口继续保留。':null;
 const definition={id:'ADAPTIVE-'+kind,continuation_required:kind==='complex',classification:kind==='complex'?'planned_requirement_update':'natural_wording_no_delivery_profile',prior_exposure:'Known host-authored Node scaffold previously evaluated in v1; revised complex planning wording, not an unchanged A/B pair or a held-out website. No implementation supplied after launch.',oracle:'dmghg',oracle_command:[process.execPath,oracle,'--case',kind,'--workspace','{workspace}'],steering,
  request:{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'guarded',thinkingLevel:'high',timeoutSeconds:seconds,prompt}};
 fs.writeFileSync(path.join(dir,'requests.json'),JSON.stringify([definition],null,2));
 fs.writeFileSync(path.join(dir,'frozen.json'),JSON.stringify({kind,candidate:{path:binary,sha256:hash(binary)},model_turns:turns,seconds,definition,oracle:{path:oracle,sha256:hash(oracle)},sources:sourceSnapshot,seeds:fs.readdirSync(project).map(n=>({path:path.join(project,n),sha256:hash(path.join(project,n))})),delivery_profile:null,cloud:false},null,2));
}
const walk=(p,depth=0)=>!fs.existsSync(p)||depth>5?[]:fs.readdirSync(p,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(p,e.name),depth+1):[path.join(p,e.name)]);
const skills=walk(path.join(root,'skills')).concat(...['.gemini/config/skills','.agents/skills','.codex/skills'].map(n=>walk(path.join(process.env.USERPROFILE,n),2).filter(p=>path.basename(p)==='SKILL.md')));
fs.writeFileSync(path.join(stage,'skills-frozen.json'),JSON.stringify(skills.map(p=>({path:p,sha256:hash(p)})),null,2));
fs.writeFileSync(path.join(stage,'source-snapshot.json'),JSON.stringify(sourceSnapshot,null,2));console.log(JSON.stringify({stage,candidate:hash(binary),cases:['simple','complex','complex-to-readonly-continuation'],budgets:[12,24,24],continuation_deadline_seconds:300}));
