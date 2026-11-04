/* Independent bounded fixture checks. Same assertions under Node --test or CLI. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const args=process.argv.slice(2),opt=k=>args.includes(k)?args[args.indexOf(k)+1]:null;
const raw=path.resolve(opt('--workspace')||process.cwd()),workspace=raw.startsWith('\\\\?\\UNC\\')?'\\\\'+raw.slice(8):raw.startsWith('\\\\?\\')?raw.slice(4):raw;
const kind=opt('--case')||(fs.existsSync(path.join(workspace,'tier.cjs'))?'simple':'complex');
try{
 const checks=[];
 if(kind!=='readonly')assert.equal(fs.readFileSync(path.join(workspace,'USER_DATA.txt'),'utf8'),'existing user data must remain unchanged\n');
 if(kind==='simple'){
  const tier=require(path.join(workspace,'tier.cjs'));for(const [n,want] of [[-1,'single'],[7,'single'],[8,'bulk'],[9,'bulk']])assert.equal(tier(n),want,'tier boundary '+n);
  checks.push('actual_tier_boundaries');
 }else if(kind==='complex'){
  const price=path.join(workspace,'price.cjs'),{unitCost}=require(price),{total}=require(path.join(workspace,'cart.cjs'));
  assert.ok(fs.readFileSync(price,'utf8').includes('// USER_NOTE_KEEP_20261104'),'user note must remain');
  assert.equal(unitCost(10,7),10);assert.equal(unitCost(10,8),9);assert.ok(Math.abs(unitCost(9.99,8)-8.991)<1e-12,'unit cost must not round early');
  assert.equal(total([]),0);assert.equal(total([{price:10,quantity:2}]),20);
  assert.equal(total([{price:9.99,quantity:8}]),71.93,'final cart rounding');
  assert.equal(total([{price:9.99,quantity:8},{price:0.1,quantity:3},{price:2.555,quantity:1}]),74.78,'combined cart rounding');
  checks.push('actual_discount_boundary','actual_quantity_integration','empty_cart','final_rounding_only','user_note_preserved');
 }else if(kind==='readonly'){
  for(const [name,want] of Object.entries(JSON.parse(opt('--expected')))){
   const actual=crypto.createHash('sha256').update(fs.readFileSync(path.join(workspace,name))).digest('hex');assert.equal(actual,want,'readonly '+name);
  }checks.push('prior_workspace_files_unchanged');
 }else throw Error('unknown fixture');
 console.log(JSON.stringify({passed:true,target:'adaptive-fixture:'+kind,checks}));
}catch(e){console.log(JSON.stringify({passed:false,target:'adaptive-fixture:'+kind,error:e.message,checks:['independent_fixture_assertions']}));process.exitCode=1;}
