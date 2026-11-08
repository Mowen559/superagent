/* Actual PC importer/runtime. No website-specific API, fixture, or acceptance answer. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const args=process.argv.slice(2),opt=k=>args.includes(k)?args[args.indexOf(k)+1]:undefined;
if(args.includes('--help')){console.log('Usage: node test-rule-live.cjs --rule <workspace.json> --entry home|search|detail|action [--page N] [--keyword text] [--url generated-url] [--request workspace-request.json]. Request accepts page/keyword/targetUrl/item/input/actionCode/params. Execution only; not media decoding or user acceptance.');process.exit(0);}
const cwd=fs.realpathSync.native(process.cwd()),sha=b=>crypto.createHash('sha256').update(b).digest('hex');
function owned(p){const real=fs.realpathSync.native(path.resolve(p));assert.ok(real.startsWith(cwd+path.sep),'input outside current workspace');return real;}
let engine,db,file,before;
const report={kind:'production_rule_entry',scope:'entry_execution_only_not_GUI_media_decode_or_user_acceptance',gui_used:false,android_verified:false};
async function main(){
 const input=opt('--request')?JSON.parse(fs.readFileSync(owned(opt('--request')),'utf8')):{};
 file=owned(opt('--rule')||input.rulePath);assert.ok(fs.statSync(file).size<=4*1024*1024,'rule too large');const bytes=fs.readFileSync(file);before=sha(bytes);
 const native=require('./pc-runtime-resources.cjs').resolvePcRuntime();
 engine=require(path.join(native,'hk-engine'));db=require(path.join(native,'hk-db'));const parser=require(path.join(native,'hiker-rule-parser'));
 const cache=path.join(cwd,'.superagent');fs.mkdirSync(cache,{recursive:true});const stage=fs.mkdtempSync(path.join(cache,'live-rule-'));
 db.init(stage,{seedFoundation:false});engine.init(stage);
 const parsed=await parser.parseHikerRuleText(bytes.toString('utf8'));assert.ok(parsed.success&&parsed.isRule,parsed.error||'not an importable rule');assert.equal(db.importRuleRows([parsed.ruleData],'production-live-check','replace').count,1);
 const rule=db.getExecutableRuleByTitle(parsed.ruleData.title);assert.ok(rule,'import lost rule');const entry=opt('--entry')||input.entry||'home';assert.ok(['home','search','detail','action'].includes(entry),'unknown entry');
 const options={page:1,params:{},...input,entry,rulePackage:rule};delete options.rulePath;delete options.workspace;
 if(opt('--keyword')!==undefined)options.keyword=opt('--keyword');if(opt('--url')!==undefined)options.targetUrl=opt('--url');if(opt('--page')!==undefined)options.page=Number(opt('--page'));
 if(options.page!==undefined)assert.ok(Number.isInteger(options.page)&&options.page>0,'invalid page');
 if(entry==='action'&&!options.actionCode){const action=String(options.targetUrl||'').match(/@(lazyRule|rule)=/);assert.ok(action,'action requires actionCode or a generated action URL');const original=options.targetUrl;options.targetUrl=original.slice(0,action.index);options.input=options.targetUrl;options.actionCode=original.slice(action.index+action[0].length);}
 const result=await engine.runRule(options);assert.equal(sha(fs.readFileSync(file)),before,'rule changed during execution');
 Object.assign(report,{success:result.success===true,entry,title:rule.title,rule_sha256:before,data:result.data,error:result.error,private_test_root:stage});
}
const deadline=new Promise((_,reject)=>{const timer=setTimeout(()=>reject(Error('production entry timed out')),50000);timer.unref();});
Promise.race([main(),deadline]).catch(error=>{report.success=false;report.error=error.message;}).finally(async()=>{await engine?.terminateAllWorkers().catch(()=>{});db?.db?.close();if(file&&before)report.rule_unchanged=sha(fs.readFileSync(file))===before;process.stdout.write(JSON.stringify(report)+'\n',()=>process.exit(report.success?0:1));});
