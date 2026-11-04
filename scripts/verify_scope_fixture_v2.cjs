/* Frozen host checks. No network, shell, or publication. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const args=process.argv.slice(2),opt=k=>args[args.indexOf(k)+1],hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const raw=path.resolve(opt('--workspace')),workspace=raw.startsWith('\\\\?\\UNC\\')?'\\\\'+raw.slice(8):raw.startsWith('\\\\?\\')?raw.slice(4):raw;
const kind=opt('--case'),target='scope-fixture:'+kind;
try{
 const checks=[];
 if(kind==='document'){
  const file=path.join(workspace,'README.md'),text=fs.readFileSync(file,'utf8');assert.notEqual(hash(file),opt('--before'));assert.ok(text.length>100);
  for(const [name,sha] of Object.entries(JSON.parse(opt('--protected'))))assert.equal(hash(path.join(workspace,name)),sha,'protected '+name+' changed');
  // Exact quality of the scoped statements is manually reviewed separately.
  checks.push('readme_revised','protected_business_artifacts_unchanged');
 }else if(kind==='implementation'){
  assert.equal(hash(path.join(workspace,'README.md')),opt('--readme-sha'),'contract must remain unchanged');
  assert.equal(hash(opt('--skill-path')),opt('--skill-sha'),'skill must remain unchanged');
  const normalize=require(path.join(workspace,'normalize-labels.cjs'));assert.equal(typeof normalize,'function');
  for(const input of [null,undefined,4,{},'a'])assert.deepEqual(normalize(input),[]);
  const values=[' A ','a',' B',null,42,'','  ','b','C','c','D'];const before=JSON.stringify(values);
  assert.deepEqual(normalize(values),['A','B','C','D']);assert.equal(JSON.stringify(values),before);
  assert.deepEqual(normalize(['É','é','猫',' 猫 ','DOG','dog']),['É','猫','DOG']);
  checks.push('actual_function_outputs','stable_case_insensitive_deduplication','input_unchanged','contract_and_skill_unchanged');
 }else throw Error('unknown case');
 console.log(JSON.stringify({passed:true,target,checks}));
}catch(e){console.log(JSON.stringify({passed:false,target,error:e.message,checks:['scope_acceptance']}));process.exitCode=1;}
