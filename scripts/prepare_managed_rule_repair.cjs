/* Prepare a separately budgeted supervised repair of a preserved Hub artifact.
 * Provides only the previous rule and observed failure, never target code edits. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict'),{spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..'),label=process.argv[2]||'nasa-managed-repair-v1';
assert.match(label,/^[\w-]+$/);
const previous=path.join(root,'docs/reviews/hub-nasa-managed-attempt-v3.json');
const old=JSON.parse(fs.readFileSync(previous,'utf8')).attempts[0];assert.equal(old.status,'failed');
const source=path.join(old.execution_workspace,'rule.json'),bytes=fs.readFileSync(source),sha=data=>crypto.createHash('sha256').update(data).digest('hex');
assert.equal(sha(bytes),old.artifacts.find(x=>path.basename(x.path)==='rule.json').sha256);
const made=spawnSync(process.execPath,[path.join(__dirname,'prepare_managed_rule_eval.cjs'),label],{encoding:'utf8'});assert.equal(made.status,0,made.stderr);process.stdout.write(made.stdout);
const stage=path.join(root,'artifacts/hub-natural-rule',label),requestsFile=path.join(stage,'requests.json'),frozenFile=path.join(stage,'frozen.json');
const requests=JSON.parse(fs.readFileSync(requestsFile,'utf8')),frozen=JSON.parse(fs.readFileSync(frozenFile,'utf8'));
const feedback='继续修复本工作区上一轮Hub生成的rule.json，完成原定NASA独立JS规则及云短口令交付。生产运行库验收失败，日志出现NASA请求在3500ms超时；直接接口探针有数据不能代替规则运行验收。请自行定位修复，通过本地验收后受控发布，补齐README.md、publication.json、share-command.txt及导入说明；不要更换目标、扫描设备或新建测试云内容。';
requests[0].id='NASA-managed-supervised-repair-'+label;requests[0].classification='known_target_supervised_repair';requests[0].prior_exposure='Previous failed Hub rule supplied unchanged. One short feedback reports the independently observed production request timeout; no implementation or protocol supplied.';
requests[0].request.prompt=feedback;requests[0].request.timeoutSeconds=900;
const seed=path.join(stage,'project/rule.json');fs.writeFileSync(seed,bytes,{flag:'wx'});
frozen.classification=requests[0].classification;frozen.initial_prompt=feedback;frozen.budget={model_turns:20,seconds:900,human_feedback:1};
frozen.prior_attempt={task_id:old.task_id,status:old.status,model_turns:old.model_turns,source_report:previous,rule_sha256:sha(bytes)};
frozen.host_target_rule_provided=true;frozen.host_target_rule_authored=false;frozen.seed={path:seed,sha256:sha(bytes),author:'previous Hub Run; unchanged seed copied by host'};
fs.writeFileSync(requestsFile,JSON.stringify(requests,null,2)+'\n');fs.writeFileSync(frozenFile,JSON.stringify(frozen,null,2)+'\n');
console.log(JSON.stringify({classification:frozen.classification,budget:frozen.budget,seed_sha256:sha(bytes)}));
