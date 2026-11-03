/* Frozen host acceptance for synthetic recovery fixtures. Never publishes. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const args=process.argv.slice(2),opt=k=>args[args.indexOf(k)+1],sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const workspace=path.resolve(opt('--workspace')),kind=opt('--case'),target='recovery-fixture:'+kind;
try {
 const checks=[];
 if(kind==='document'){
  const before=opt('--before'),readme=fs.readFileSync(path.join(workspace,'README.md'));assert.notEqual(sha(readme),before,'README was not revised');assert.ok(readme.length>100);
  for(const [name,expected] of Object.entries(JSON.parse(opt('--protected'))))assert.equal(sha(fs.readFileSync(path.join(workspace,name))),expected,'protected '+name+' changed');
  checks.push('readme_revised','protected_business_files_unchanged');
 } else if(kind==='arguments'){
  const request=JSON.parse(fs.readFileSync(path.join(workspace,'request.json'),'utf8'));assert.equal(request.operation,'ping','invalid_arguments: missing field operation=ping');assert.equal(request.request_id,'keep-this-id');checks.push('actual_request_fields');
 } else if(kind==='dependency'){
  const run=require(path.join(workspace,'main.cjs'));assert.equal(run(3),6);assert.equal(run(-2),-4);checks.push('actual_local_dependency_execution');
 } else if(kind==='network'){
  assert.equal(fs.readFileSync(path.join(workspace,'network.txt'),'utf8'),'existing-network-fixture\n');
  const state=opt('--state'),attempts=fs.existsSync(state)?JSON.parse(fs.readFileSync(state,'utf8')).attempts:0;
  if(args.includes('--audit'))assert.ok(attempts>=2,'transient fixture did not recover through actual validator attempts');
  else {fs.writeFileSync(state,JSON.stringify({attempts:attempts+1}));if(attempts===0)throw Error('transient_network: ECONNRESET simulated first acceptance attempt');}
  checks.push('transient_fixture_recovers_without_source_edit');
 } else if(kind==='business'){
  const clamp=require(path.join(workspace,'clamp.cjs'));for(const [input,expected] of [[-5,0],[0,0],[50,50],[120,100]])assert.equal(clamp(input),expected,'business_validation: clamp output mismatch at '+input);checks.push('actual_function_boundary_cases');
 } else throw Error('unknown fixture');
 console.log(JSON.stringify({passed:true,target,checks}));
}catch(e){console.log(JSON.stringify({passed:false,target,error:e.message,checks:['actual_fixture_acceptance']}));process.exitCode=1;}
