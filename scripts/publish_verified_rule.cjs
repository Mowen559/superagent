/* Managed capability: fixed provider, verified source only, one creation attempt.
 * Called by perform_delivery_action, never a rule runtime dependency. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const sha=data=>crypto.createHash('sha256').update(data).digest('hex');
const realPath=file=>fs.realpathSync.native(file);
function canonical(file,workspace){const real=realPath(file);assert.ok(real.startsWith(realPath(workspace)+path.sep),'artifact outside workspace');return real}
async function publish({workspace,artifact,expected,target,reuseOnly=false},{fetcher=globalThis.fetch,parser}={}){
 workspace=realPath(workspace);artifact=canonical(artifact,workspace);
 const data=fs.readFileSync(artifact);assert.equal(sha(data),expected,'source changed before publication');
 const rule=JSON.parse(data.toString('utf8'));assert.ok(!Array.isArray(rule)&&rule.title&&rule.find_rule,'not a single rule');
 parser ||= require(path.resolve(__dirname,'../../deepcloud-ui/native/runtime/hiker-rule-parser'));
 const receiptFile=path.join(workspace,'publication.json'),commandFile=path.join(workspace,'share-command.txt');
 const attemptFile=path.join(workspace,'.superagent','cloud-publication-attempt.json');
 // Refuse redirected output paths too, including a pre-existing directory symlink.
 for(const file of [receiptFile,commandFile,attemptFile]){
  if(fs.lstatSync(file,{throwIfNoEntry:false}))canonical(file,workspace);
  if(fs.lstatSync(path.dirname(file),{throwIfNoEntry:false}))assert.ok(realPath(path.dirname(file))===workspace||realPath(path.dirname(file)).startsWith(workspace+path.sep),'output directory outside workspace');
 }
 let receipt;
 if(fs.existsSync(receiptFile)){
  receipt=JSON.parse(fs.readFileSync(receiptFile,'utf8'));assert.equal(receipt.rule_sha256,expected,'existing publication belongs to another source; never create a replacement implicitly');
  assert.ok(/^云6oooole\/xxxxxx\/[A-Za-z0-9_-]+$/.test(receipt.remote_command),'existing receipt has no supported public ID');
 }else{
  assert.ok(!reuseOnly,'existing publication required; creation disabled for this action');
  assert.ok(!fs.existsSync(attemptFile),'previous creation may have committed; preserve state, do not retry POST');
  fs.mkdirSync(path.dirname(attemptFile),{recursive:true});
  fs.writeFileSync(attemptFile,JSON.stringify({provider:'pasteme',source_sha256:expected,state:'creation_attempted',utc:new Date().toISOString()}),{flag:'wx'});
  const content=parser.exportRuleToStandardCommand(rule);
  assert.equal(sha(fs.readFileSync(artifact)),expected,'source changed before POST');
  const response=await fetcher('https://pasteme.tyrantg.com/api/create',{method:'POST',headers:{'Content-Type':'application/json','User-Agent':'DeepCloud-Hub'},body:JSON.stringify({content}),signal:AbortSignal.timeout(20000)});
  assert.ok(response.ok,'cloud create HTTP '+response.status);
  const body=await response.json(),id=body?.data?.path;assert.ok(body?.result_code==='SUCCESS'&&typeof id==='string'&&/^[A-Za-z0-9_-]+$/.test(id),'cloud did not return a valid ID');
  // Never persist or print response auth_code/password/headers. Persist the ID
  // before readback so an interrupted verifier cannot cause another creation.
  receipt={remote_command:'云6oooole/xxxxxx/'+id,provider:'pasteme',code_id:id,web_url:'https://pasteme.tyrantg.com/'+id,rule_sha256:expected,roundtrip_verified:false};
  fs.writeFileSync(receiptFile,JSON.stringify(receipt,null,2)+'\n',{flag:'wx'});
 }
 fs.writeFileSync(commandFile,receipt.remote_command+'\n');
 const baseline=await parser.parseHikerRuleText(parser.exportRuleToStandardCommand(rule));
 const remote=await parser.parseHikerRuleText(receipt.remote_command);assert.ok(remote.success&&remote.isRule,remote.error||'cloud readback failed; existing ID preserved');
 for(const key of ['title','version','url','class_name','class_url','find_rule','search_url','searchFind','detail_find_rule'])assert.deepEqual(remote.ruleData[key],baseline.ruleData[key],'cloud differs: '+key);
 assert.equal(sha(fs.readFileSync(artifact)),expected,'source changed during publication');
 // Restore public provider metadata if an older model rewrote a legacy receipt.
 // This happens only after a real readback; never create a new cloud entry.
 receipt.provider='pasteme';receipt.code_id=receipt.remote_command.split('/').at(-1);
 receipt.web_url='https://pasteme.tyrantg.com/'+receipt.code_id;
 receipt.roundtrip_verified=true;fs.writeFileSync(receiptFile,JSON.stringify(receipt,null,2)+'\n');
 return {passed:true,target,checks:['verified_source_hash','single_creation_attempt','production_cloud_readback','source_fields_equal'],remote_command:receipt.remote_command,source_sha256:expected};
}
module.exports={publish};
if(require.main===module){const args=process.argv.slice(2),opt=k=>args[args.indexOf(k)+1];publish({workspace:opt('--workspace'),artifact:opt('--artifact'),expected:opt('--expected-sha256'),target:opt('--target'),reuseOnly:args.includes('--reuse-only')})
 .then(r=>console.log(JSON.stringify(r))).catch(e=>{console.log(JSON.stringify({passed:false,error:e.message}));process.exitCode=1})}
