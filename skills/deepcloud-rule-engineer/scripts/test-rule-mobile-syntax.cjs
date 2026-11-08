/* Parser-specific syntax evidence only. No rule execution or Android certification. */
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),crypto=require('node:crypto'),assert=require('node:assert/strict'),{spawnSync}=require('node:child_process');
const args=process.argv.slice(2),option=k=>args.includes(k)?args[args.indexOf(k)+1]:undefined;
const resources=require('./mobile-parser-resources.cjs');
if(args.includes('--list-parsers')){console.log(JSON.stringify(resources.parserStatus()));process.exit(0);}
const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
if(args.includes('--help')){console.log('Usage: node test-rule-mobile-syntax.cjs [--rule] <workspace-rule.json> [--parser-profile ID | --rhino-jar trusted.jar] [--source generated-lazy.js]. Use --list-parsers for resource paths/status. Default: public comparison profile, never device certification. Requires Java 11+. Compile only.');process.exit(0);}
const report={kind:'mobile_parser_syntax',scope:'compile_only_no_execution_or_Android_device_acceptance',android_verified:false,language_version:200,checks:[]};
try {
 const cwd=fs.realpathSync(process.cwd());
 const owned=f=>{const real=fs.realpathSync(path.resolve(f)),rel=path.relative(cwd,real);assert.ok(rel&&!rel.startsWith('..'+path.sep)&&!path.isAbsolute(rel),'input outside workspace');assert.ok(fs.statSync(real).size<=4*1024*1024,'input too large');return real;};
 const profile=option('--parser-profile')||'public-rhino-1.7.14';
 const jarInput=option('--rhino-jar')||resources.resolveParser(profile);
 assert.ok(fs.existsSync(jarInput),'Parser unavailable: provide --rhino-jar <actual-parser.jar>');
 const jar=fs.realpathSync(path.resolve(jarInput));assert.equal(path.extname(jar),'.jar','parser jar required');
 report.parser={sha256:sha(fs.readFileSync(jar)),file:path.basename(jar),profile:option('--rhino-jar')?'explicit':profile,source:option('--rhino-jar')?'explicit':profile==='repository-rhino-1.7.13'?'repository_baseline_not_device_version':'registered_public_comparison_not_device_version'};
 const ruleInput=option('--rule')||(!args[0]?.startsWith('--')?args[0]:undefined);assert.ok(ruleInput,'Provide --rule <workspace-rule.json> or a first positional rule file');
 const ruleFile=owned(ruleInput),bytes=fs.readFileSync(ruleFile),rule=JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/,''));report.rule_sha256=sha(bytes);
 const inputs=[];for(const key of ['preRule','find_rule','searchFind','detail_find_rule'])if(typeof rule[key]==='string'&&rule[key].trim())inputs.push({name:key,source:rule[key].replace(/^js:/,'')});
 if(option('--source')){const f=owned(option('--source'));inputs.push({name:'actual_generated_source',source:fs.readFileSync(f,'utf8')});}
 assert.ok(inputs.length,'no script entries');
 const temp=fs.mkdtempSync(path.join(os.tmpdir(),'hiker-mobile-syntax-'));
 const files=inputs.map((x,i)=>{const f=path.join(temp,i+'.js');fs.writeFileSync(f,x.source);return f;});
 const result=spawnSync('java',['-cp',jar,path.join(__dirname,'MobileSyntaxProbe.java'),...files],{encoding:'utf8',windowsHide:true,timeout:45000,maxBuffer:1024*1024});
 if(result.error)throw result.error;
 const lines=result.stdout.trim().split(/\r?\n/);assert.equal(lines.length,inputs.length,'parser failed before all checks: '+result.stderr.slice(0,250));
 report.checks=inputs.map((x,i)=>{const [status,line,...message]=lines[i].split('\t');return {entry:x.name,passed:status==='PASS',line:Number(line),message:message.join(' ')};});
 report.rule_unchanged=sha(fs.readFileSync(ruleFile))===report.rule_sha256;
 report.passed=result.status===0&&report.rule_unchanged&&report.checks.every(x=>x.passed);
 if(!report.passed)process.exitCode=1;
}catch(error){report.passed=false;report.error=error.message;report.parser_resources=resources.parserProfiles();process.exitCode=1;}
console.log(JSON.stringify(report));
