/* Explicit current-page binding; captures/compiles eval source, never executes it. */
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),crypto=require('node:crypto'),assert=require('node:assert/strict'),{spawnSync}=require('node:child_process');
const args=process.argv.slice(2),opt=k=>args.includes(k)?args[args.indexOf(k)+1]:undefined,sha=b=>crypto.createHash('sha256').update(b).digest('hex');
const resources=require('./mobile-parser-resources.cjs');
if(args.includes('--list-parsers')){console.log(JSON.stringify(resources.parserStatus()));process.exit(0);}
if(args.includes('--help')){console.log('Usage: node test-rule-mobile-context.cjs --rule workspace.json --source generated-lazy.js --binding find_rule|searchFind|detail_find_rule [--parser-profile ID | --rhino-jar trusted.jar]. Use --list-parsers for ready resources. Default: public comparison parser. Outer action/captured compile only, not Android/network/playback acceptance.');process.exit(0);}
const report={kind:'current_page_eval_capture',scope:'partial_page_binding_and_captured_compile_only',android_verified:false,network_or_media_executed:false,captured:[]};
try {
 const cwd=fs.realpathSync(process.cwd()),owned=f=>{const real=fs.realpathSync(path.resolve(f)),rel=path.relative(cwd,real);assert.ok(rel&&!rel.startsWith('..'+path.sep)&&!path.isAbsolute(rel),'input outside workspace');assert.ok(fs.statSync(real).size<=4*1024*1024,'input too large');return real;};
 const file=owned(opt('--rule')),sourceFile=owned(opt('--source')),bytes=fs.readFileSync(file),rule=JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/,''));
 const binding=opt('--binding');assert.ok(['find_rule','searchFind','detail_find_rule'].includes(binding),'explicit current-page binding required');assert.equal(typeof rule[binding],'string','binding source missing');
 const jar=fs.realpathSync(path.resolve(opt('--rhino-jar')||resources.resolveParser(opt('--parser-profile')||'public-rhino-1.7.14')));assert.equal(path.extname(jar),'.jar','trusted parser jar required');
 report.rule_sha256=sha(bytes);report.action_sha256=sha(fs.readFileSync(sourceFile));report.parser={file:path.basename(jar),sha256:sha(fs.readFileSync(jar)),profile:opt('--rhino-jar')?'explicit':opt('--parser-profile')||'public-rhino-1.7.14'};report.binding=binding;
 const folder=fs.mkdtempSync(path.join(os.tmpdir(),'mobile-context-')),input=path.join(folder,'outer.js');
 fs.writeFileSync(input,'var MY_RULE = '+JSON.stringify({find_rule:rule[binding],title:rule.title||'',url:rule.url||''})+';var input="hiker://empty";\n'+fs.readFileSync(sourceFile,'utf8'));
 const r=spawnSync('java',['-Xmx192m','-cp',jar,path.join(__dirname,'CaptureMobileEval.java'),input],{encoding:'utf8',windowsHide:true,timeout:20000,maxBuffer:1024*1024});if(r.error)throw r.error;
 assert.equal(r.status,0,'capture process failed: '+r.stderr.slice(0,220));
 for(const line of r.stdout.split(/\r?\n/)){const [kind,passed,number,chars,hash,message]=line.split('\t');if(kind==='EVAL')report.captured.push({passed:passed==='true',line:Number(number),chars:Number(chars),sha256:hash,message:Buffer.from(message||'','base64').toString('utf8')});if(kind==='STOP')report.outer_stop=Buffer.from(passed||'','base64').toString('utf8');}
 report.rule_unchanged=sha(fs.readFileSync(file))===report.rule_sha256;
 report.covered=report.captured.length>0;report.passed=report.covered&&report.rule_unchanged&&report.captured.every(x=>x.passed);
 if(!report.covered)report.limitation='No eval captured; action may need other platform APIs. This does not prove a rule error.';
 if(!report.passed)process.exitCode=1;
}catch(error){report.passed=false;report.error=error.message;report.parser_resources=resources.parserProfiles();process.exitCode=1;}
console.log(JSON.stringify(report));
