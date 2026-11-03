const test=require('node:test'),assert=require('node:assert/strict'),path=require('node:path'),fs=require('node:fs'),{spawnSync}=require('node:child_process');
const {mediaUrls,categoryQuery,categoryMatches}=require('./verify_nasa_rule_v4.cjs');
test('full media path with spaces and header suffix keeps source identity',()=>{
 assert.deepEqual(mediaUrls('https://images-assets.nasa.gov/video/Go Forward/Go Forward~orig.mp4;{Referer@https://images.nasa.gov/}'),['https://images-assets.nasa.gov/video/Go Forward/Go Forward~orig.mp4']);
 assert.deepEqual(mediaUrls({data:{urls:['https://example.com/a.mp4']}}),['https://example.com/a.mp4']);
});
test('category source must match the label, not just any NASA video',()=>{
 assert.equal(categoryQuery('火星探测'),'mars');assert.equal(categoryQuery('地球观测'),'earth');
 const expected=[{data:[{title:'Mars landing documentary'}]}];categoryMatches([{title:'Mars landing documentary'}],expected);
 assert.throws(()=>categoryMatches([{title:'Earth from Orbit 2013'}],expected),/category label/);
});
test('real production imported search fallback and newly created DB path pass; broken source rejects',()=>{
 const stage=fs.mkdtempSync(path.resolve(__dirname,'../artifacts/hub-natural-rule/oracle-v4-unit-')),config=path.join(stage,'config.json');fs.writeFileSync(config,JSON.stringify({stage}));
 const r=spawnSync(process.execPath,[path.join(__dirname,'verify_nasa_rule_v4.cjs'),'--config',config,'--self-test'],{encoding:'utf8',timeout:30000,windowsHide:true});
 const result=JSON.parse(r.stdout.trim().split(/\r?\n/).at(-1));assert.equal(r.status,0,JSON.stringify(result));assert.equal(result.passed,true);
});
