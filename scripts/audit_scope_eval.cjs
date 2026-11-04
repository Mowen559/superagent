const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..'),label=process.argv[2];assert.match(label,/^[\w-]+$/);const stage=path.join(root,'artifacts',label);
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const verifyFiles=files=>files.map(x=>({path:x.path,expected:x.sha256,actual:fs.existsSync(x.path)?hash(x.path):null})).filter(x=>x.expected!==x.actual);
const skillFiles=['production-skills-frozen.json','additional-catalogs-frozen.json'].flatMap(n=>JSON.parse(fs.readFileSync(path.join(stage,n))));
const cases=[];
for(const c of JSON.parse(fs.readFileSync(path.join(stage,'cases.json')))){
 const reportPath=path.join(root,'docs/reviews',`${label}-${c.kind}.json`),report=JSON.parse(fs.readFileSync(reportPath)),a=report.attempts[0],frozen=JSON.parse(fs.readFileSync(path.join(c.dir,'frozen.json')));
 assert.ok(['completed','failed','cancelled','timeout','infra_failed'].includes(a.status),'Run must terminate before auditing');
 const traces=a.events.filter(e=>e.kind==='trace-snapshot'),tools=a.events.filter(e=>e.kind==='tool-use'),verification=a.events.filter(e=>e.kind==='tool-result'&&e.tool==='verify_delivery').map(e=>{try{return JSON.parse(e.output)}catch{return e.output}});
 const first=traces[0],snapshotPath=first&&a.execution_workspace?path.join(a.execution_workspace,first.snapshot_file):null;
 const snapshot=snapshotPath&&fs.existsSync(snapshotPath)?JSON.parse(fs.readFileSync(snapshotPath)):null;
 const identity=JSON.parse(fs.readFileSync(path.join(c.dir,'identity.json'),'utf8').replace(/^\uFEFF/,''));
 cases.push({kind:c.kind,task_id:a.task_id,status:a.status,passed:a.passed,oracle_passed:a.oracle_passed,oracle:a.oracle,
  report_path:reportPath,report_sha256:hash(reportPath),execution_workspace:a.execution_workspace,
  model_turns:traces.length,tool_calls:tools.length,duration_ms:a.duration_ms,
  usage:{prompt_tokens:traces.reduce((n,e)=>n+(e.prompt_tokens||0),0),completion_tokens:traces.reduce((n,e)=>n+(e.completion_tokens||0),0),source:'provider-reported trace-snapshot fields'},
  tools:tools.map(e=>e.tool),commands:tools.filter(e=>e.tool==='run_command').length,plans:tools.filter(e=>e.tool==='plan_task').length,
  skill_reads:tools.filter(e=>e.tool==='skill_tool'&&e.input?.action==='read').map(e=>({name:e.input.skill_name,resource:e.input.resource_path||'SKILL.md'})),
  managed_actions:tools.filter(e=>e.tool==='perform_delivery_action').length,
  observed_raw_publication_references:tools.filter(e=>e.tool==='run_command'&&/paste\.me|pasteme|netcut\.cn|export-command\.cjs|publish_verified_rule/i.test(JSON.stringify(e.input))).length,
  verify_delivery:verification.map(v=>({completion_ready:v.completion_ready,domain_verified:v.domain_verified,exit_code:v.exit_code})),
  runtime_prompt:{available:!!snapshot,sha256:snapshotPath&&fs.existsSync(snapshotPath)?hash(snapshotPath):null,
   candidate_catalog:snapshot?.system_prompt?.includes('当前任务的Skill候选')||false,
   old_forced_mount:snapshot?.system_prompt?.includes('你必须强制严格遵守')||false,
   inline_explicit_skill:snapshot?.system_prompt?.includes('用户完整点名的Skill（正文按当前任务范围使用）')||false,
   physical_strategy:JSON.stringify(snapshot).includes('Read only the evidence needed for the requested files'),
   unconditional_compile_anchor:JSON.stringify(snapshot).includes('Verification Closed-Loop: Run compile/test checks before claiming completion'),
   physical_verification_title:JSON.stringify(snapshot).includes('Verify declared physical files with verify_delivery'),
   default_code_strategy:JSON.stringify(snapshot).includes('make the smallest coherent change, run a quality check')},
  candidate_sha256:frozen.candidate.sha256,private_binary_sha256:hash(path.join(identity.stage,'hub.exe')),
  frozen_mismatches:verifyFiles([...frozen.files,...frozen.seeds,frozen.candidate])});
}
const result={date:'2026-11-04',stage,cases,skill_files_checked:skillFiles.length,skill_mismatches:verifyFiles(skillFiles),
 totals:{attempts:cases.length,completed:cases.filter(x=>x.status==='completed').length,failed:cases.filter(x=>x.status==='failed').length,model_turns:cases.reduce((n,x)=>n+x.model_turns,0),tool_calls:cases.reduce((n,x)=>n+x.tool_calls,0)},
 scope:'Known-target documentation replays and a synthetic explicit Skill implementation. Not held-out website autonomy, Android certification, or intelligence success rate.'};
const output=path.join(stage,'post-run-audit.json');assert.ok(!fs.existsSync(output),'Never overwrite audit');fs.writeFileSync(output,JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({output,totals:result.totals,skill_mismatches:result.skill_mismatches.length,source_mismatches:cases.reduce((n,x)=>n+x.frozen_mismatches.length,0)}));
