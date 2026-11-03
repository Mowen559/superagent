/* Separate, bounded synthetic cases; host scaffolds and assistance are disclosed. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..'),label=process.argv[2]||'recovery-v1';assert.match(label,/^[\w-]+$/);const stage=path.join(root,'artifacts',label);assert.ok(!fs.existsSync(stage));
const sha=b=>crypto.createHash('sha256').update(b).digest('hex'),hash=p=>sha(fs.readFileSync(p));
const sources=['tool_recovery.rs','tool_execution_gateway.rs','bounded_recovery_tests.rs','cognitive_runtime.rs','task_progress.rs','run_plan.rs','delivery_evidence.rs'].map(n=>path.join(root,'src/services/brain',n));
const binary=path.join(root,'target/debug/hub.exe'),oracle=path.join(root,'scripts/verify_recovery_fixture.cjs');assert.ok(fs.statSync(binary).mtimeMs>=Math.max(...sources.map(p=>fs.statSync(p).mtimeMs)),'Build latest candidate first');
const specs=[
 ['document',10,{ },'请修订本目录README，使它准确说明现有独立JS规则的联网前提、PC无界面代表样本验收范围及手动导入方式；Android和PC图形界面未测，不要保证全部视频都有4K。其余三个交付文件保留原样，完成文件复核后给出结果。'],
 ['arguments',12,{'request.json':'{"operaton":"ping","request_id":"keep-this-id"}\n'},'请先按原样调用现有交付验收，依据具体错误修复request.json的请求字段，再复核交付。保留请求ID。'],
 ['dependency',12,{'main.cjs':"const double=require('./helper.cjs'); module.exports=value=>double(value);\n",'README.md':'本地模块约定：main.cjs导出函数，输入3得到6，输入-2得到-4。缺失的模块应在项目目录实现，不安装第三方包。\n'},'请先用现有交付验收复现本目录运行失败，再依据README补齐缺失的本地依赖并通过复验；不安装外部依赖。'],
 ['network',10,{'network.txt':'existing-network-fixture\n'},'只做复核：验证本目录现有network.txt是否通过注册的交付验收，保持文件不修改，给出实际结果。'],
 ['business',12,{'clamp.cjs':'module.exports=value=>Math.max(0,value);\n','README.md':'clamp应把数值限制在闭区间[0,100]内，导出单参数函数。\n'},'请先用现有交付验收复现clamp.cjs的边界错误，修复后复核实际函数行为并交付。']
];
fs.mkdirSync(stage,{recursive:true});const cases=[];
for(const [kind,turns,seeds,prompt] of specs){
 const dir=path.join(stage,kind),project=path.join(dir,'project');fs.mkdirSync(project,{recursive:true});
 for(const [n,text] of Object.entries(seeds))fs.writeFileSync(path.join(project,n),text);
 const protectedFiles={};let before;
 if(kind==='document'){
  const prior=JSON.parse(fs.readFileSync(path.join(root,'docs/reviews/hub-nasa-managed-finish-v1.json'),'utf8')).attempts[0];
  for(const n of ['README.md','rule.json','publication.json','share-command.txt']){const bytes=fs.readFileSync(path.join(prior.execution_workspace,n));fs.writeFileSync(path.join(project,n),bytes);if(n==='README.md')before=sha(bytes);else protectedFiles[n]=sha(bytes);}
 }
 const file=({document:'README.md',arguments:'request.json',dependency:'main.cjs',network:'network.txt',business:'clamp.cjs'})[kind];
 const command=['--case',kind,'--workspace','{workspace}'];if(kind==='network')command.push('--state',path.join(dir,'network-state.json'));if(kind==='document')command.push('--before',before,'--protected',JSON.stringify(protectedFiles));
 const artifacts=[{id:'output',path:file,required:true,...(kind==='document'?{}:{validator:'fixture'})}];if(kind==='dependency')artifacts.push({id:'helper',path:'helper.cjs',required:true});
 const profile={delivery:{version:1,scope:'implementation',target:'recovery-fixture:'+kind,artifacts},completion_policy:kind==='document'?'physical_artifacts':'quality_checks',validators:kind==='document'?[]:[{id:'fixture',program:process.execPath,program_sha256:hash(process.execPath),script:oracle,script_sha256:hash(oracle),args:command,timeout_seconds:20}],actions:[]};
 fs.writeFileSync(path.join(dir,'profile.json'),JSON.stringify(profile,null,2)+'\n');
 const request={id:'RECOVERY-'+kind,classification:kind==='document'?'known_case_documentation_replay':'synthetic_recovery_fixture',prior_exposure:'Host-authored scaffold and acceptance contract; not held-out real-site autonomy. No feedback after launch.',oracle:'dmghg',oracle_command:[process.execPath,oracle,...command,'--audit'],request:{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'guarded',thinkingLevel:'high',timeoutSeconds:300,prompt}};
 fs.writeFileSync(path.join(dir,'requests.json'),JSON.stringify([request],null,2)+'\n');
 fs.writeFileSync(path.join(dir,'frozen.json'),JSON.stringify({candidate:{path:binary,sha256:hash(binary)},classification:request.classification,case:kind,budget:{model_turns:turns,seconds:300,followup_feedback:0},initial_prompt:prompt,host_scaffold:true,cloud_creation_authorized:false,files:[...sources,oracle].map(p=>({path:p,sha256:hash(p)})),seeds:fs.readdirSync(project).map(n=>({path:path.join(project,n),sha256:hash(path.join(project,n))}))},null,2)+'\n');
 cases.push({kind,turns,dir});
}
fs.writeFileSync(path.join(stage,'cases.json'),JSON.stringify(cases,null,2)+'\n');console.log(JSON.stringify({stage,cases:cases.map(x=>({kind:x.kind,turns:x.turns})),candidate:hash(binary)}));
