/* Restore the host verifier contract; preserve the original Hub publication. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const root=path.resolve(__dirname,'..'),base=path.join(root,'artifacts/hub-cloud-publish');
const read=p=>JSON.parse(fs.readFileSync(p,'utf8').replace(/^\uFEFF/,''));
const sha=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const identity=read(path.join(base,'case-v1/identity.json'));
const from=path.join(identity.app_data_root,'runs/38c36d5e-42d6-4e6b-8e86-6a936977552a/workspace');
const stage=path.join(base,'case-v2'),project=path.join(stage,'project');assert.ok(!fs.existsSync(stage));fs.mkdirSync(project,{recursive:true});
const copied={};for(const file of ['rule.json','publication.json','share-command.txt']){fs.copyFileSync(path.join(from,file),path.join(project,file));copied[file]=sha(path.join(project,file));}
const target=path.join(project,'rule.json'),script=path.join(root,'scripts/verify_hub_cloud_publication_v2.cjs');
const profile=read(path.join(base,'case-v1/profile.json'));profile.delivery.target=target;
Object.assign(profile.validators[0],{script,script_sha256:sha(script),args:['--workspace','{workspace}','--artifact','{artifact}','--target','{target}']});
const requests=[{id:'cloud-host-contract-recovery-v2',classification:'host_harness_recovery_readback_only',prior_exposure:'Rule and cloud receipt copied unchanged from actual Hub uploader task. No new upload evaluated in this Run.',oracle:'dmghg',
 oracle_command:[process.execPath,script,'--workspace','{workspace}','--artifact','{workspace}/publication.json','--target',target],
 request:{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'guarded',thinkingLevel:'high',timeoutSeconds:600,
 prompt:'继续验收已完成的云发布。工作区三个文件是上一Hub任务的真实产物，云6oooole/xxxxxx/qdvjijbdhafvtgja 已上传并回读成功。上一轮宿主冻结验收器缺少必填 target 输出字段导致门禁失败，宿主已另建修正版配置。本轮禁止任何新上传、禁止修改现有三个交付文件，不用重新生成或逆向。请读取适用规则技能、检查现有文件、声明适用交付计划并实际调用 verify_delivery 复验全部产物，按真实证据更新进度并结束。所有写文件/桌面/LAN/账号注册付费均不需要；若尚有阻塞如实报告。本轮是同一发布的验收恢复，不能声称这是一次新的自主上传，也不能将上一轮终态改成成功。'}}];
for(const [name,value]of Object.entries({'profile.json':profile,'requests.json':requests,'preparation.json':{from,copied,host_uploaded:false,purpose:'fix missing target in trusted host output; never mutate frozen v1 validator or repeat publication'}}))fs.writeFileSync(path.join(stage,name),JSON.stringify(value,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({stage,copied,host_uploaded:false}));
