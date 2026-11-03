/* Read-only remote verifier. Never uploads content or creates cloud entries. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..'),ui=path.resolve(root,'../deepcloud-ui');
const {parseHikerRuleText,exportRuleToStandardCommand}=require(path.join(ui,'native/runtime/hiker-rule-parser'));
const {HkDatabase}=require(path.join(ui,'native/runtime/hk-db'));
const args=process.argv.slice(2),opt=x=>args[args.indexOf(x)+1];
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const original=path.join(root,'artifacts/hub-delivery/dmghg-independent-js-v1/rule.json');
const expectedHash='f3e214ead8fa7f89494d4ed3467e246d5e83d0c94813ddc8974a5093c36fdd95';
const fields=['title','version','url','class_name','class_url','find_rule','search_url','searchFind','detail_find_rule'];
function validateCommand(command){
 assert.equal(typeof command,'string','remote_command missing');assert.ok(command.length>5&&command.length<4096,'not a short command');
 assert.ok(!/base64:\/\/|localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]/i.test(command),'offline/local content is not cloud publication');
 assert.ok(/云\d+oooole|https?:\/\//i.test(command),'unsupported cloud command form');
}
async function main(){
 if(args.includes('--self-test')){
  for(const value of ['base64://@test@AAAA','http://127.0.0.1/a','local-file','x'.repeat(5000)])assert.throws(()=>validateCommand(value));
  assert.equal(sha(fs.readFileSync(original)),expectedHash);const rule=JSON.parse(fs.readFileSync(original,'utf8'));
  const result=await parseHikerRuleText(exportRuleToStandardCommand(rule));assert.ok(result.success&&result.isRule,result.error);
  for(const field of fields)assert.deepEqual(result.ruleData[field],rule[field],field+' local roundtrip changed');
  console.log(JSON.stringify({passed:true,checks:['offline_local_and_oversize_rejected','production_local_command_roundtrip'],uploaded:false}));return;
 }
 const workspace=path.resolve(opt('--workspace')),artifact=path.resolve(opt('--artifact'));
 assert.ok(artifact.startsWith(workspace+path.sep),'receipt outside workspace');assert.ok(fs.statSync(artifact).size<65536,'oversize receipt');
 const receipt=JSON.parse(fs.readFileSync(artifact,'utf8')),command=receipt.remote_command;validateCommand(command);
 const commandFile=path.join(workspace,'share-command.txt');assert.equal(fs.readFileSync(commandFile,'utf8').trim(),command.trim(),'command file and receipt differ');
 assert.equal(sha(fs.readFileSync(original)),expectedHash,'trusted original changed');
 assert.equal(sha(fs.readFileSync(path.join(workspace,'rule.json'))),expectedHash,'published task rule was modified');
 const expected=JSON.parse(fs.readFileSync(original,'utf8'));
 const baseline=await parseHikerRuleText(exportRuleToStandardCommand(expected));assert.ok(baseline.success&&baseline.isRule,baseline.error);
 const actual=await parseHikerRuleText(command);assert.ok(actual.success&&actual.isRule,actual.error||'remote command did not recover a rule');
 const fieldHashes={};for(const field of fields){assert.deepEqual(actual.ruleData[field],baseline.ruleData[field],'cloud readback differs: '+field);fieldHashes[field]=sha(String(actual.ruleData[field]??''));}
 const dbDir=fs.mkdtempSync(path.join(root,'artifacts/hub-cloud-publish/import-verification-')),db=new HkDatabase();
 let imported;
 try{db.init(dbDir,{seedFoundation:false});imported=db.importRuleRows([actual.ruleData],'cloud-roundtrip','replace');assert.equal(imported.count,1);
  const executable=db.getExecutableRuleByTitle(expected.title);assert.ok(executable);for(const field of ['find_rule','searchFind','detail_find_rule'])assert.equal(executable[field],expected[field],'database dropped '+field);
 }finally{db.db?.close()}
 console.log(JSON.stringify({passed:true,remote_command:command,original_sha256:expectedHash,field_hashes:fieldHashes,
  checks:['remote_short_command_readback','production_cloud_command_parser','frozen_rule_fields_equal','actual_private_database_import'],
  publisher:'Hub task; verifier did not upload',gui_used:false}));
}
main().catch(e=>{console.log(JSON.stringify({passed:false,error:e.message}));process.exitCode=1});
