/* Host-only post-run audit. No target implementation or changes to evidence. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..'),label=process.argv[2];assert.match(label,/^[\w-]+$/);
const stage=path.join(root,'artifacts',label),read=p=>JSON.parse(fs.readFileSync(p,'utf8').replace(/^\uFEFF/,''));
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const verify=rows=>rows.map(x=>({path:x.path,expected:x.sha256,actual:fs.existsSync(x.path)?hash(x.path):null})).filter(x=>x.expected.toLowerCase()!==x.actual?.toLowerCase());
const cases=[],frozen=[];
for(const kind of ['simple','complex']){
 const dir=path.join(stage,kind),identity=read(path.join(dir,'identity.json')),definition=read(path.join(dir,'frozen.json'));
 frozen.push(...definition.seeds,...definition.sources,...definition.sources.map(x=>({path:x.original,sha256:x.sha256})),definition.oracle,{path:path.join(identity.stage,'hub.exe'),sha256:definition.candidate.sha256});
 const reportPath=path.join(root,'docs/reviews',`${label}-${kind}.json`),report=read(reportPath);
 for(const a of report.attempts){
  assert.ok(['completed','failed','cancelled','timeout','infra_failed'].includes(a.status));
  const traces=a.events.filter(e=>e.kind==='trace-snapshot'),tools=a.events.filter(e=>e.kind==='tool-use');
  const snapshots=traces.map(e=>({path:path.join(a.execution_workspace,e.snapshot_file),data:read(path.join(a.execution_workspace,e.snapshot_file))}));
  const facts=snapshots.flatMap(s=>{const m=s.data.system_prompt?.match(/\[Current Run host facts\]\n([^\n]+)/);if(!m)return [];try{return [{turn:s.data.turn,...JSON.parse(m[1])}]}catch{return []}});
  const provenance=snapshots.map(s=>({turn:s.data.turn,labels:s.data.messages.flatMap(m=>{try{const v=JSON.parse(m.content);return v._run_provenance?[v._run_provenance]:[]}catch{return []}})}));
  const results=a.events.filter(e=>e.kind==='tool-result').map(e=>{let value;try{value=JSON.parse(e.output)}catch{value={}}return {tool:e.tool,value};});
  cases.push({id:a.id,task_id:a.task_id,status:a.status,passed:a.passed,oracle:a.oracle,steering:a.steering||null,session_reused:a.session_reused??null,
   report_path:reportPath,report_sha256:hash(reportPath),execution_workspace:a.execution_workspace,model_turns:traces.length,tool_calls:tools.length,duration_ms:a.duration_ms,
   tools:tools.map(e=>e.tool),plans:results.filter(e=>e.tool==='plan_task').map(e=>({status:e.value.status,error:e.value.error,contract:e.value.task_contract})),
   revisions:results.filter(e=>e.tool==='revise_task_plan').map(e=>e.value),
   progress:results.filter(e=>e.tool==='update_task_progress').map(e=>e.value),
   facts,provenance,snapshots:snapshots.map(s=>({path:s.path,sha256:hash(s.path),steering_visible:a.steering?JSON.stringify(s.data.messages).includes(a.steering.prompt):null})),
   usage:{prompt_tokens:traces.reduce((n,e)=>n+(e.prompt_tokens||0),0),completion_tokens:traces.reduce((n,e)=>n+(e.completion_tokens||0),0),source:'provider-reported trace fields'},
   candidate:definition.candidate.sha256,budget:{model_turns:definition.model_turns,seconds:a.request.timeoutSeconds},
   forbidden_readonly_operations:a.id.endsWith('readonly')?tools.filter(e=>['write_to_file','replace_in_file','multi_replace_file_content','run_command','verify_task_tool','perform_delivery_action'].includes(e.tool)).map(e=>e.tool):[],
   oracle_source_reads:tools.filter(e=>['view_file','read_file','run_command'].includes(e.tool)&&/verify_adaptive_fixture\.cjs/.test(JSON.stringify(e.input))&&!/node\s+(--test\s+)?["']?[^\n]*verify_adaptive_fixture\.cjs/.test(e.input?.CommandLine||'')).length,
   managed_actions:tools.filter(e=>e.tool==='perform_delivery_action').length});
 }
}
const result={date:'2026-11-04',label,planned:3,executed:cases.length,cases,
 totals:{completed:cases.filter(c=>c.status==='completed').length,failed:cases.filter(c=>c.status==='failed').length,model_turns:cases.reduce((n,c)=>n+c.model_turns,0),tool_calls:cases.reduce((n,c)=>n+c.tool_calls,0)},
 source_or_seed_mismatches:verify(frozen),skill_mismatches:verify(read(path.join(stage,'skills-frozen.json'))),harness_mismatches:verify(read(path.join(stage,'harness-frozen.json'))),
 scope:'Bounded synthetic tasks and same-Session continuation; file oracle is not semantic prose or plan acceptance. No intelligence percentage, held-out site, cloud publication, desktop or installed release certification.'};
const out=path.join(stage,'post-run-audit.json');assert.ok(!fs.existsSync(out),'Never overwrite audit');fs.writeFileSync(out,JSON.stringify(result,null,2));
console.log(JSON.stringify({output:out,planned:result.planned,executed:result.executed,totals:result.totals,mismatches:result.source_or_seed_mismatches.length+result.skill_mismatches.length+result.harness_mismatches.length}));
