/* Prepare an empty project and frozen host checks; never publishes a rule. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
throw new Error('Retired pilot oracle: see hub-nasa-natural-final-review-v2.json. Preserve existing frozen profiles; correct and regression-test the oracle before preparing another model benchmark.');
const label=process.argv[2]||'nasa-v2';assert.match(label,/^[\w-]+$/);
const root=path.resolve(__dirname,'..'),stage=path.join(root,'artifacts/hub-natural-rule',label),project=path.join(stage,'project');
assert.ok(!fs.existsSync(stage),'Do not overwrite earlier tests');fs.mkdirSync(project,{recursive:true});
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const python='C:/Users/刘本录/AppData/Local/Python/pythoncore-3.14-64/python.exe',decoderDir='D:/太极/动漫共和国概念版';
const config={stage,python,decoder_dir:decoderDir,decoder_role:'Host-only libmpv for windowless media tests. No installed application API used and no rule runtime dependency.'};
fs.writeFileSync(path.join(stage,'oracle-config.json'),JSON.stringify(config,null,2));
const verifier=path.join(root,'scripts/verify_nasa_natural_rule.cjs'),configFile=path.join(stage,'oracle-config.json');
const profile={delivery:{version:1,scope:'implementation',target:'https://images.nasa.gov/',artifacts:[
 {id:'rule',path:'rule.json',required:true,validator:'nasa-runtime'},{id:'readme',path:'README.md',required:true},
 {id:'publication',path:'publication.json',required:true,validator:'nasa-cloud'},{id:'command',path:'share-command.txt',required:true}]},
 validators:['runtime','cloud'].map(phase=>({id:'nasa-'+phase,program:process.execPath,program_sha256:sha(process.execPath),script:verifier,script_sha256:sha(verifier),
 args:['--config',configFile,'--workspace','{workspace}','--artifact','{artifact}','--phase',phase],timeout_seconds:150}))};
const prompt='请把 https://images.nasa.gov/ 做成海阔世界PC可直接使用的独立JS视频规则，支持分类浏览、翻页、搜索、详情和实际播放，不依赖Python或本机桥接服务。请自行使用已有Skill调查并实现，最后发布云剪切板短口令并附导入说明。允许本规则云发布，不扫描或推送设备。交付rule.json、README.md、share-command.txt和publication.json，后者用remote_command保存真实短口令。';
const requests=[{id:'NASA-natural-first-v1',classification:'first_attempt_new_public_api_target',prior_exposure:'No target-specific protocol or business implementation provided by host; target absent in repository Skill/request literal precheck. Pretrained model knowledge not controlled.',oracle:'dmghg',
 oracle_command:[process.execPath,verifier,'--config',configFile,'--workspace','{workspace}','--artifact','{workspace}/publication.json','--phase','cloud'],
 request:{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'guarded',thinkingLevel:'high',timeoutSeconds:1500,prompt}}];
const pinned=[verifier,path.join(root,'scripts/verify_public_media.py'),path.join(root,'skills/local-app-inspector/scripts/dmghg_probe.py'),path.join(decoderDir,'libmpv-2.dll'),python,process.execPath,
 path.resolve(root,'../deepcloud-ui/native/runtime/hiker-rule-parser.js'),path.resolve(root,'../deepcloud-ui/native/runtime/hk-engine.js'),path.resolve(root,'../deepcloud-ui/scripts/hub-delivery-replay.cjs')];
function walk(dir){return fs.readdirSync(dir,{withFileTypes:true}).flatMap(d=>d.isDirectory()?walk(path.join(dir,d.name)):[path.join(dir,d.name)])}
const skillFiles=walk(path.join(root,'skills')).filter(p=>/\.(md|cjs|js|py|json)$/i.test(p));
for(const [name,value] of Object.entries({'profile.json':profile,'requests.json':requests,'frozen.json':{target:'https://images.nasa.gov/',initial_prompt:prompt,initial_project_empty:fs.readdirSync(project).length===0,
 budget:{initial_model_turns:40,initial_seconds:1500,max_human_feedback:2,max_correction_runs:2,correction_turns:20,correction_seconds:900,total_model_turns:80},
 boundaries:['No host protocol, selectors or business implementation','At most two brief factual error feedbacks','No host publishing','No repeated upload after successful ID','Oracle source inspection disqualifies strict first autonomy','No GUI/LAN/Android certification'],
 program_and_host_files:pinned.map(p=>({path:p,sha256:sha(p)})),skills:skillFiles.map(p=>({path:p,sha256:sha(p)})),candidate:{path:path.join(root,'target/debug/hub.exe'),sha256:sha(path.join(root,'target/debug/hub.exe'))},host_uploaded:false}}))
 fs.writeFileSync(path.join(stage,name),JSON.stringify(value,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({stage,profile:path.join(stage,'profile.json'),requests:path.join(stage,'requests.json'),files_pinned:pinned.length+skillFiles.length,host_uploaded:false}));
