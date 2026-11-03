/* Host-side task preparation only. No cloud write implementation or requests. */
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),assert=require('node:assert/strict');
const label=process.argv[2]||'case-v3';assert.match(label,/^[\w-]+$/);
const root=path.resolve(__dirname,'..'),stage=path.join(root,'artifacts/hub-cloud-publish',label);
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const expected='f3e214ead8fa7f89494d4ed3467e246d5e83d0c94813ddc8974a5093c36fdd95';
assert.ok(!fs.existsSync(stage),'Never overwrite a prior evaluation');
const source=path.join(root,'artifacts/hub-delivery/dmghg-independent-js-v1/rule.json');assert.equal(hash(source),expected);
const project=path.join(stage,'project');fs.mkdirSync(project,{recursive:true});fs.copyFileSync(source,path.join(project,'rule.json'));
const verifier=path.join(root,'scripts/verify_hub_cloud_publication_v2.cjs');
const profile={delivery:{version:1,scope:'implementation',target:path.join(project,'rule.json'),artifacts:[
 {id:'rule',path:'rule.json',required:true},{id:'publication',path:'publication.json',required:true,validator:'cloud-roundtrip'},
 {id:'command',path:'share-command.txt',required:true}]},validators:[{id:'cloud-roundtrip',program:process.execPath,
 program_sha256:hash(process.execPath),script:verifier,script_sha256:hash(verifier),
 args:['--workspace','{workspace}','--artifact','{artifact}','--target','{target}'],timeout_seconds:120}]};
const prompt=`请独立完成这一个明确任务：将工作区已有的 rule.json 发布到真实的海阔云剪切板，交付可从云端取回原规则的短口令。用户已明确授权上传这一份规则，不能上传任何模型密钥/用户登录凭据。不需要重新逆向应用或重新生成规则。现有规则已通过本地生产运行时验证，请保持原始字节和 SHA256 ${expected} 不变。
先通过 skill_tool 读取适用的 deepcloud-rule-engineer 技能和发布/导入相关脚本，自行选择可用云服务并执行真实上传。可以复用已有模块，也可以自行编写工作区脚本。不要调用带局域网扫描/直推副作用的 CLI main；禁止桌面、LAN 52020、原应用目录修改、账号注册和付费。每个服务同类失败最多尝试两次，最多三个服务；一旦得到成功发布的 ID，停止创建新副本，只围绕这份 ID 回读、解析和验收。不要用虚构 ID、测试示例 ID、离线 Base64 长口令或本地 HTTP 代替云发布。
交付文件：保持 rule.json；share-command.txt 仅写真实短口令；publication.json 至少包含 remote_command（字符串），并记录 provider、web_url（如果有）、真实上传/回读证据及次数、未验证范围。上传成功响应本身不能视作回读成功，必须真的取回内容。请实际调用 verify_delivery，修复可修复的问题后再结束。冻结验收器会通过生产云口令解析器回读，比较规则关键字段，并导入独立数据库。失败时保留真实错误，不要写成功。范围仅为发布现有规则，不得声称整个逆向与生成过程均由你独立完成。`;
const requests=[{id:'cloud-publication-first-v1',classification:'first_attempt',prior_exposure:'Existing verified rule provided; prior protocol work coached. Only cloud publication is evaluated independently here.',oracle:'dmghg',
 oracle_command:[process.execPath,verifier,'--workspace','{workspace}','--artifact','{workspace}/publication.json','--target',path.join(project,'rule.json')],
 request:{agent:'superagent',project,model:'gemini-3.8-flash-high',workMode:'guarded',thinkingLevel:'high',timeoutSeconds:1500,prompt}}];
for(const [name,value] of Object.entries({'profile.json':profile,'requests.json':requests,'preparation.json':{source,source_sha256:expected,host_uploaded:false,host_wrote_publisher:false,verifier_sha256:hash(verifier),scope:'publication of an existing verified rule'}}))fs.writeFileSync(path.join(stage,name),JSON.stringify(value,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({stage,project,profile:path.join(stage,'profile.json'),requests:path.join(stage,'requests.json'),host_uploaded:false}));
