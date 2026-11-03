const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const {publish}=require('./publish_verified_rule.cjs');
const production=require('../../deepcloud-ui/native/runtime/hiker-rule-parser');
function fixture(){
 const workspace=fs.mkdtempSync(path.resolve(__dirname,'../artifacts/hub-natural-rule/managed-publisher-'));
 const artifact=path.join(workspace,'rule.json');fs.writeFileSync(artifact,JSON.stringify({title:'managed-publication-canary',version:1,url:'hiker://empty',find_rule:'js:setResult([]);'}));
 const options={workspace,artifact,expected:crypto.createHash('sha256').update(fs.readFileSync(artifact)).digest('hex'),target:'test-target'};
 const state={posts:0,remote:null,readbackFailure:false};
 const parser={exportRuleToStandardCommand:production.exportRuleToStandardCommand,
  parseHikerRuleText:async text=>{if(text.startsWith('云6')){if(state.readbackFailure)throw Error('simulated readback outage');return production.parseHikerRuleText(state.remote)}return production.parseHikerRuleText(text)}};
 const fetcher=async(url,request)=>{assert.equal(url,'https://pasteme.tyrantg.com/api/create');assert.equal(request.method,'POST');state.posts++;state.remote=JSON.parse(request.body).content;return {ok:true,json:async()=>({result_code:'SUCCESS',data:{path:'managed-test-id',auth_code:'DO_NOT_SAVE_MANAGEMENT_CANARY'}})}};
 return {options,state,deps:{fetcher,parser}};
}
test('source mismatch never creates pending state or sends a POST',async()=>{const f=fixture();f.options.expected='wrong';await assert.rejects(publish(f.options,f.deps),/source changed/);assert.equal(f.state.posts,0);assert.equal(fs.existsSync(path.join(f.options.workspace,'.superagent')),false)});

test('reuse-only action cannot create even if the old receipt is missing',async()=>{
 const f=fixture();await assert.rejects(publish({...f.options,reuseOnly:true},f.deps),/creation disabled/);assert.equal(f.state.posts,0);assert.equal(fs.existsSync(path.join(f.options.workspace,'.superagent')),false);
});

test('reuse-only action revalidates an existing ID without POST',async()=>{
 const f=fixture();await publish(f.options,f.deps);const before=f.state.posts;assert.equal((await publish({...f.options,reuseOnly:true},f.deps)).passed,true);assert.equal(f.state.posts,before);
});
test('single fixed-provider POST, real local command parser, no management credential in receipts',async()=>{
 const f=fixture(),result=await publish(f.options,f.deps);assert.equal(result.passed,true);assert.equal(f.state.posts,1);
 for(const file of ['publication.json','share-command.txt','.superagent/cloud-publication-attempt.json'])assert.ok(!fs.readFileSync(path.join(f.options.workspace,file),'utf8').includes('DO_NOT_SAVE_MANAGEMENT_CANARY'));
 const again=await publish(f.options,f.deps);assert.equal(again.passed,true);assert.equal(f.state.posts,1,'resume must only read back existing ID');
});
test('readback failure preserves ID; retry is read-only',async()=>{
 const f=fixture();f.state.readbackFailure=true;await assert.rejects(publish(f.options,f.deps),/readback outage/);assert.equal(f.state.posts,1);
 assert.equal(JSON.parse(fs.readFileSync(path.join(f.options.workspace,'publication.json'),'utf8')).remote_command,'云6oooole/xxxxxx/managed-test-id');
 f.state.readbackFailure=false;assert.equal((await publish(f.options,f.deps)).passed,true);assert.equal(f.state.posts,1);
});
test('ambiguous POST timeout blocks creation in the next invocation',async()=>{
 const f=fixture();f.deps.fetcher=async()=>{f.state.posts++;throw Error('ambiguous transport timeout')};await assert.rejects(publish(f.options,f.deps),/timeout/);
 await assert.rejects(publish(f.options,f.deps),/previous creation may have committed/);assert.equal(f.state.posts,1);
});
test('changed rule cannot silently replace an existing cloud ID',async()=>{
 const f=fixture();await publish(f.options,f.deps);fs.writeFileSync(f.options.artifact,JSON.stringify({title:'changed',find_rule:'js:setResult([])'}));f.options.expected=crypto.createHash('sha256').update(fs.readFileSync(f.options.artifact)).digest('hex');
 await assert.rejects(publish(f.options,f.deps),/another source/);assert.equal(f.state.posts,1);
});
test('wrong cloud content fails without creating another copy',async()=>{
 const f=fixture(),fetcher=f.deps.fetcher;f.deps.fetcher=async(...args)=>{const response=await fetcher(...args);f.state.remote=production.exportRuleToStandardCommand({title:'wrong cloud source',find_rule:'js:setResult([])'});return response};
 await assert.rejects(publish(f.options,f.deps),/cloud differs/);await assert.rejects(publish(f.options,f.deps),/cloud differs/);assert.equal(f.state.posts,1);
});
test('redirected control directory is rejected before network or writes',async()=>{
 const f=fixture(),outside=fs.mkdtempSync(path.resolve(__dirname,'../artifacts/hub-natural-rule/publisher-outside-'));
 fs.symlinkSync(outside,path.join(f.options.workspace,'.superagent'),'junction');await assert.rejects(publish(f.options,f.deps),/output directory outside/);assert.equal(f.state.posts,0);assert.deepEqual(fs.readdirSync(outside),[]);
});
test('Windows canonical paths from the Rust runtime are accepted',async()=>{
 const f=fixture();f.options.workspace=path.toNamespacedPath(f.options.workspace);f.options.artifact=path.toNamespacedPath(f.options.artifact);
 assert.equal((await publish(f.options,f.deps)).passed,true);assert.equal(f.state.posts,1);
});
