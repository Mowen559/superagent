const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),crypto=require('node:crypto'),{spawnSync}=require('node:child_process');
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
function parserProfiles(options={}) {
 const repo=options.repoRoot||path.resolve(__dirname,'../../../..');
 const cache=options.cacheRoot||process.env.HIKER_RULE_TOOL_CACHE||path.join(os.homedir(),'.superagent','tools');
 const definitions=[
  {id:'public-rhino-1.7.14',scope:'public comparison parser, not APK or device certification',sha256:'c9290b0d801bf0dbbbc44338e0f769b7650a0c5d04e6bb1aeb85775c0211b003',candidates:[path.join(cache,'rhino/1.7.14/rhino-1.7.14.jar')]},
  {id:'repository-rhino-1.7.13',scope:'legacy repository baseline, not device version',sha256:'baa4f9015e3bc95e3c8f77a82598f0cc98415b727221e77e2557572a46fb177f',candidates:[path.join(repo,'references/hiker-decompiled-android/source/app/libs/rhino-1.7.13.jar'),path.join(repo,'engine-service/hikerView/app/libs/rhino-1.7.13.jar')]}
 ];
 return definitions.map(({candidates,...profile})=>{
  const file=candidates.find(p=>fs.existsSync(p)&&fs.statSync(p).isFile());
  if(!file)return {...profile,resource_ready:false,path:candidates[0],reason:'missing'};
  const actual=sha(file);return {...profile,path:fs.realpathSync(file),actual_sha256:actual,resource_ready:actual===profile.sha256,reason:actual===profile.sha256?null:'fingerprint mismatch'};
 });
}
function resolveParser(id,options={}) {
 const entry=parserProfiles(options).find(p=>p.id===id);
 if(!entry||!entry.resource_ready)throw Error('Parser resource unavailable: '+id+'; use --list-parsers');
 return entry.path;
}
function parserStatus() {
 const r=spawnSync('java',['-version'],{encoding:'utf8',windowsHide:true,timeout:3000});
 const major=Number((r.stderr+' '+r.stdout).match(/version "(\d+)/)?.[1]||0);
 return {java_available:!r.error&&r.status===0&&major>=11,java_major:major,profiles:parserProfiles(),android_verified:false};
}
module.exports={parserProfiles,resolveParser,parserStatus};
