/* Preserve actual Hub rule/public receipt; ask Hub to finish documentation only. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict'),{spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..'),label=process.argv[2]||'nasa-managed-finish-v1';assert.match(label,/^[\w-]+$/);
const prior=JSON.parse(fs.readFileSync(path.join(root,'docs/reviews/hub-nasa-managed-repair-v2.json'),'utf8')).attempts[0];
assert.equal(prior.status,'failed');assert.equal(prior.oracle_passed,true);
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const seeds=['rule.json','publication.json','share-command.txt'].map(name=>({name,bytes:fs.readFileSync(path.join(prior.execution_workspace,name))}));
const publication=JSON.parse(seeds[1].bytes.toString('utf8'));assert.equal(publication.roundtrip_verified,true);assert.equal(publication.rule_sha256,sha(seeds[0].bytes));
assert.ok(!/auth_code|password|api_key/i.test(seeds[1].bytes.toString('utf8')));
const prepared=spawnSync(process.execPath,[path.join(__dirname,'prepare_managed_rule_eval.cjs'),label],{encoding:'utf8'});assert.equal(prepared.status,0,prepared.stderr);process.stdout.write(prepared.stdout);
const stage=path.join(root,'artifacts/hub-natural-rule',label),read=n=>JSON.parse(fs.readFileSync(path.join(stage,n),'utf8')),write=(n,x)=>fs.writeFileSync(path.join(stage,n),JSON.stringify(x,null,2)+'\n');
for(const s of seeds)fs.writeFileSync(path.join(stage,'project',s.name),s.bytes,{flag:'wx'});
const profile=read('profile.json');profile.validators.find(v=>v.id==='cloud-create-once').args.push('--reuse-only');write('profile.json',profile);
const prompt='请只补齐本目录的README.md并完成交付复核。现有rule.json、publication.json和share-command.txt均来自上一轮Hub，已有云ID已回读及导入通过；这三个文件保持原样，禁止新建云条目。README说明海阔世界PC导入、使用和联网前提，未实测平台不要声称通过。用现有验收工具复核后结题，无需重新调查接口或读取内部trace。';
const requests=read('requests.json');requests[0].id='NASA-existing-publication-finish-'+label;requests[0].classification='known_case_delivery_finalization';requests[0].prior_exposure='Unchanged previous Hub rule and successful public receipt supplied; focused documentation completion feedback. Not fresh autonomous generation or upload.';requests[0].request.prompt=prompt;requests[0].request.timeoutSeconds=900;write('requests.json',requests);
const frozen=read('frozen.json');Object.assign(frozen,{classification:requests[0].classification,initial_prompt:prompt,budget:{model_turns:12,seconds:900,human_feedback:1},prior_task:{id:prior.task_id,status:prior.status,oracle_passed:prior.oracle_passed},host_target_rule_provided:true,host_target_rule_authored:false,cloud_policy:'reuse-only; creation disabled even if receipt disappears',seeds:seeds.map(s=>({path:path.join(stage,'project',s.name),sha256:sha(s.bytes),author:'previous Hub Run / its managed action; copied unchanged by host'}))});write('frozen.json',frozen);
console.log(JSON.stringify({classification:frozen.classification,budget:frozen.budget,cloud_policy:frozen.cloud_policy}));
