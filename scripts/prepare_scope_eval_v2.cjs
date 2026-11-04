/* Bounded task-scope replays. Host scaffolds and prior exposure are explicit. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..'),label=process.argv[2]||'hub-scope-v2';assert.match(label,/^[\w-]+$/);
const stage=path.join(root,'artifacts',label);assert.ok(!fs.existsSync(stage),'Never overwrite evidence');
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const binary=path.join(root,'target/debug/hub.exe'),oracleSource=path.join(root,'scripts/verify_scope_fixture_v2.cjs'),oracle=path.join(stage,'frozen/verify_scope_fixture_v2.cjs');
const sources=['src/domains/agent_cognitive/capability_scout.rs',...['delivery_evidence','run_plan','bounded_recovery_tests','worker','run_prelude','cognitive_runtime','task_progress','task_progress_tests','tools/skill_tool'].map(n=>'src/services/brain/'+n+'.rs')].map(n=>path.join(root,n));
assert.ok(fs.statSync(binary).mtimeMs>=Math.max(...sources.map(p=>fs.statSync(p).mtimeMs)),'Build current candidate first');
fs.mkdirSync(path.dirname(oracle),{recursive:true});fs.copyFileSync(oracleSource,oracle);
const sourceSnapshot=sources.map(p=>{const copy=path.join(stage,'frozen/source',path.relative(root,p));fs.mkdirSync(path.dirname(copy),{recursive:true});fs.copyFileSync(p,copy);assert.equal(hash(copy),hash(p));return {original:p,path:copy,sha256:hash(copy)};});
fs.writeFileSync(path.join(stage,'source-snapshot.json'),JSON.stringify({timing:'prelaunch byte copies',files:sourceSnapshot},null,2)+'\n');
const cases=[];
for(const kind of ['document-replay','document-reworded','explicit-implementation']){
 const dir=path.join(stage,kind),project=path.join(dir,'project');fs.mkdirSync(project,{recursive:true});
 let prompt,profile,command;
 if(kind.startsWith('document')){
  const priorDir=path.join(root,'artifacts/hub-recovery-v1/document/project');
  for(const file of ['README.md','rule.json','publication.json','share-command.txt'])fs.copyFileSync(path.join(priorDir,file),path.join(project,file));
  const previous=JSON.parse(fs.readFileSync(path.join(root,'artifacts/hub-recovery-v1/document/requests.json'),'utf8'))[0];
  prompt=kind==='document-replay'?previous.request.prompt:'给接手这套NASA海阔规则的人整理一下README：写清楚网络条件、短口令怎样手动导入，以及本轮到底检查到了哪一层。别把抽样说成全库认证，也别承诺手机或桌面窗口已经测过。保留现有规则与发布凭据文件。';
  const protectedFiles=Object.fromEntries(['rule.json','publication.json','share-command.txt'].map(n=>[n,hash(path.join(project,n))]));
  command=['--case','document','--workspace','{workspace}','--before',hash(path.join(project,'README.md')),'--protected',JSON.stringify(protectedFiles)];
  profile={delivery:{version:1,scope:'implementation',target:'scope-fixture:document',artifacts:[{id:'readme',path:'README.md',required:true}]},completion_policy:'physical_artifacts',validators:[],actions:[]};
 }else{
  const skillDir=path.join(project,'.agents/skills/scoped-list-fixture');fs.mkdirSync(skillDir,{recursive:true});
  fs.writeFileSync(path.join(skillDir,'SKILL.md'),'---\nname: scoped-list-fixture\ndescription: 本地列表清理与稳定去重实现\n---\n本技能用于本地数组整理。按README中的行为契约实现并使用注册交付验收复核。仅使用Node内置能力，不需要联网或安装依赖；说明文件保持原样。技能资源是方法参考，不能扩大本轮目标。\n');
  fs.writeFileSync(path.join(project,'README.md'),'实现normalize-labels.cjs，CommonJS导出单参数函数。输入为数组，忽略非字符串项，清理字符串首尾空白、丢弃空项，按不区分大小写稳定去重，保留首次出现项的大小写。不得改写原数组；非数组输入返回空数组。只用内置能力。\n');
  prompt='请使用 scoped-list-fixture，把README里描述的列表整理函数真正写出来，放在normalize-labels.cjs，完成交付验收后告诉我结果。';
  command=['--case','implementation','--workspace','{workspace}','--readme-sha',hash(path.join(project,'README.md')),'--skill-sha',hash(path.join(skillDir,'SKILL.md')),'--skill-path',path.join(skillDir,'SKILL.md')];
  profile={delivery:{version:1,scope:'implementation',target:'scope-fixture:implementation',artifacts:[{id:'module',path:'normalize-labels.cjs',required:true,validator:'behavior'}]},validators:[{id:'behavior',program:process.execPath,program_sha256:hash(process.execPath),script:oracle,script_sha256:hash(oracle),args:command,timeout_seconds:20}],actions:[]};
 }
 const turns=10;
 fs.writeFileSync(path.join(dir,'profile.json'),JSON.stringify(profile,null,2)+'\n');
 const request={id:'SCOPE-'+kind,classification:kind==='document-replay'?'known_failed_case_replay':kind==='document-reworded'?'new_wording_known_target':'synthetic_explicit_skill_implementation',prior_exposure:'Host-provided scaffolds and current acceptance contract; not a held-out site or zero-shot rule generation. No feedback after launch.',oracle:'dmghg',oracle_command:[process.execPath,oracle,...command],request:{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'guarded',thinkingLevel:'high',timeoutSeconds:300,prompt}};
 fs.writeFileSync(path.join(dir,'requests.json'),JSON.stringify([request],null,2)+'\n');
 const allFiles=p=>fs.readdirSync(p,{withFileTypes:true}).flatMap(e=>e.isDirectory()?allFiles(path.join(p,e.name)):[path.join(p,e.name)]);
 fs.writeFileSync(path.join(dir,'frozen.json'),JSON.stringify({candidate:{path:binary,sha256:hash(binary)},kind,model_turns:turns,seconds:300,prompt,feedback:0,cloud_creation:false,files:[...sources,oracle,path.join(dir,'profile.json'),path.join(dir,'requests.json')].map(p=>({path:p,sha256:hash(p)})),seeds:allFiles(project).map(p=>({path:p,sha256:hash(p)}))},null,2)+'\n');
 cases.push({kind,dir,turns});
}
const skillRoot=path.join(root,'skills'),walk=p=>fs.readdirSync(p,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(p,e.name)):[path.join(p,e.name)]);
fs.writeFileSync(path.join(stage,'production-skills-frozen.json'),JSON.stringify(walk(skillRoot).map(p=>({path:p,sha256:hash(p)})),null,2)+'\n');
const catalogs=[path.resolve(root,'../.agents/skills'),...['.gemini/config/skills','.agents/skills','.codex/skills'].map(n=>path.join(process.env.USERPROFILE,n))];
const findSkills=(p,depth=0)=>!fs.existsSync(p)||depth>3?[]:fs.readdirSync(p,{withFileTypes:true}).flatMap(e=>e.isDirectory()?findSkills(path.join(p,e.name),depth+1):e.name==='SKILL.md'?[path.join(p,e.name)]:[]);
fs.writeFileSync(path.join(stage,'additional-catalogs-frozen.json'),JSON.stringify(catalogs.flatMap(p=>findSkills(p)).map(p=>({path:p,sha256:hash(p)})),null,2)+'\n');
fs.writeFileSync(path.join(stage,'cases.json'),JSON.stringify(cases,null,2)+'\n');console.log(JSON.stringify({stage,cases:cases.map(x=>x.kind),candidate:hash(binary)}));
