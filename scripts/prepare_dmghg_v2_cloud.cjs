// Host setup only. Registers the existing managed publisher; never invokes it.
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const label=process.argv[3]||'dmghg-mobile-v2-20261105';assert.match(label,/^[\w-]+$/);
const root=path.resolve(__dirname,'..'),stage=path.join(root,'artifacts/hub-cloud-publish',label);
const python=process.argv[2];assert.ok(python&&fs.existsSync(python),'absolute Python validator path required');
assert.ok(!fs.existsSync(stage),'preserve previous attempts');
const project=path.join(stage,'project');fs.mkdirSync(project,{recursive:true});
const source=path.join(root,'artifacts/dmghg-android-feedback-20261104/hub-mobile-candidate-v2/rule.json');
const expected='d6466413b28076fe91e0fd1c65c77e2dea4e671a695baf51c30fe503578a011e';
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
assert.equal(sha(source),expected);fs.copyFileSync(source,path.join(project,'rule.json'));
const verifier=path.join(stage,'verify-publication.cjs'),runtime=path.join(root,'scripts/verify_dmghg_direct_js.py');
const publisher=path.join(root,'scripts/publish_verified_rule.cjs');
const config=path.join(stage,'config.json'),target=path.join(project,'rule.json');
fs.writeFileSync(config,JSON.stringify({target,expected,python,python_sha256:sha(python),runtime,runtime_sha256:sha(runtime)},null,2));
fs.writeFileSync(verifier,`// Read-only publisher verifier. No cloud create calls.
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),crypto=require('node:crypto'),a=require('node:assert/strict'),{spawnSync}=require('node:child_process');
const argv=process.argv.slice(2),opt=k=>argv[argv.indexOf(k)+1];
const sha=b=>crypto.createHash('sha256').update(b).digest('hex'),hash=p=>sha(fs.readFileSync(p));
const config=JSON.parse(fs.readFileSync(opt('--config'),'utf8'));
async function main(){
 const workspace=fs.realpathSync.native(opt('--workspace')),artifact=fs.realpathSync.native(opt('--artifact'));
 a.ok(artifact.startsWith(workspace+path.sep),'artifact outside workspace');a.equal(hash(config.target),config.expected,'trusted source changed');
 const ruleFile=path.join(workspace,'rule.json');a.equal(hash(ruleFile),config.expected,'candidate changed');
 if(opt('--phase')==='rule'){
  a.equal(artifact,fs.realpathSync.native(ruleFile));a.equal(hash(config.runtime),config.runtime_sha256);a.equal(hash(config.python),config.python_sha256);
  const result=spawnSync(config.python,['-X','utf8',config.runtime,'--workspace',workspace,'--artifact',artifact],{encoding:'utf8',timeout:150000,windowsHide:true});
  const data=JSON.parse(result.stdout.trim().split('\\n').at(-1));a.equal(result.status,0,JSON.stringify(data));a.equal(data.passed,true);a.equal(data.rule_sha256,config.expected);
  return {...data,target:config.target};
 }
 a.equal(opt('--phase'),'cloud');const receipt=JSON.parse(fs.readFileSync(artifact,'utf8')),command=receipt.remote_command;
 a.equal(receipt.rule_sha256,config.expected);a.match(command,/^云6oooole\\/xxxxxx\\/[A-Za-z0-9_-]+$/);
 a.equal(fs.readFileSync(path.join(workspace,'share-command.txt'),'utf8').trim(),command);
 const ui=path.resolve(${JSON.stringify(root)},'../deepcloud-ui/native/runtime');
 const parser=require(path.join(ui,'hiker-rule-parser')), {HkDatabase}=require(path.join(ui,'hk-db'));
 const expected=JSON.parse(fs.readFileSync(ruleFile,'utf8'));
 const baseline=await parser.parseHikerRuleText(parser.exportRuleToStandardCommand(expected)),remote=await parser.parseHikerRuleText(command);
 a.ok(baseline.success&&baseline.isRule);a.ok(remote.success&&remote.isRule,remote.error||'cloud readback failed');
 const fields=['title','version','url','class_name','class_url','find_rule','search_url','searchFind','detail_find_rule'];
 const field_hashes={};for(const k of fields){a.deepEqual(remote.ruleData[k],baseline.ruleData[k],'cloud differs '+k);field_hashes[k]=sha(String(remote.ruleData[k]??''));}
 const db=new HkDatabase(),dir=fs.mkdtempSync(path.join(os.tmpdir(),'hub-cloud-v2-import-'));
 try{db.init(dir,{seedFoundation:false});a.equal(db.importRuleRows([remote.ruleData],'cloud-roundtrip','replace').count,1);
  const executable=db.getExecutableRuleByTitle(expected.title);a.ok(executable);for(const k of ['find_rule','searchFind','detail_find_rule'])a.equal(executable[k],expected[k]);
 }finally{db.db?.close()}
 a.equal(hash(ruleFile),config.expected);
 return {passed:true,target:config.target,remote_command:command,source_sha256:config.expected,field_hashes,
  checks:['new_short_command_readback','production_parser_field_equality','real_private_database_import'],android_verified:false,uploaded_by_verifier:false};
}
main().then(x=>console.log(JSON.stringify(x))).catch(e=>{console.log(JSON.stringify({passed:false,error:e.message}));process.exitCode=1});
`);
const entry=(id,script,args,timeout_seconds)=>({id,program:process.execPath,program_sha256:sha(process.execPath),script,script_sha256:sha(script),args,timeout_seconds});
const profile={delivery:{version:1,scope:'implementation',target,artifacts:[
 {id:'rule',path:'rule.json',required:true,validator:'rule-runtime'},
 {id:'publication',path:'publication.json',required:true,validator:'cloud-readback'},
 {id:'command',path:'share-command.txt',required:true},
 {id:'readme',path:'README.md',required:true}]},validators:[
 entry('rule-runtime',verifier,['--config',config,'--workspace','{workspace}','--artifact','{artifact}','--phase','rule'],150),
 entry('cloud-readback',verifier,['--config',config,'--workspace','{workspace}','--artifact','{artifact}','--phase','cloud'],60),
 entry('cloud-create-once',publisher,['--workspace','{workspace}','--artifact','{artifact}','--target','{target}','--expected-sha256','{sha256}'],60)
 ],actions:[{id:'publish_cloud',source:'rule',executor:'cloud-create-once'}]};
const prompt='请按照 deepcloud-rule-engineer Skill，把工作区已有的动漫共和国独立JS测试版v2发布成新的海阔云剪切板短口令。用户已明确授权发布这份新版。规则已经生成，不需重新逆向或改动规则，保持rule.json原始字节不变；不要发布旧版，也不要覆盖旧云条目。自行完成本轮验证、受控发布、生产解析回读及交付。输出share-command.txt、publication.json和简短README.md，给出真正可复制的口令。说明这是已有规则的发布；Android手机实测仍未完成，不能宣称实机播放通过。不使用桌面、局域网设备推送或额外本地服务。若得到成功ID，保留该ID，回读失败只复验，不重复创建。';
const requests=[{id:'DMGHG-v2-authorized-managed-cloud-publication',classification:'existing_rule_cloud_publication',prior_exposure:'Known Hub-authored v2; host supplied original context diagnosis. This task evaluates publication only.',request:{agent:'superagent',project,prompt,model:'gemini-3.8-flash-high',workMode:'guarded',thinkingLevel:'high',timeoutSeconds:1500}}];
for(const [name,data] of Object.entries({'profile.json':profile,'requests.json':requests,'preparation.json':{
 source,expected,scope:'publish already generated v2 only',host_uploaded:false,existing_publisher:publisher,
 binary_sha256:sha(path.join(root,'target/debug/hub.exe')),profile_target:target,
 dependencies:[runtime,publisher,config,verifier].map(p=>({path:p,sha256:sha(p)}))
}}))fs.writeFileSync(path.join(stage,name),JSON.stringify(data,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({stage,project,profile:path.join(stage,'profile.json'),requests:path.join(stage,'requests.json'),host_uploaded:false}));
