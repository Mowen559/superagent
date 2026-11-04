/* Read-only current-delivery validator. Never generates or uploads a rule. */
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..'),args=process.argv.slice(2),opt=k=>args[args.indexOf(k)+1];
const target=opt('--target'),workspace=opt('--workspace'),artifact=opt('--artifact');
try {
 assert.ok(target&&workspace&&artifact,'host delivery arguments required');
 const result=spawnSync('C:/Users/刘本录/AppData/Local/Python/pythoncore-3.14-64/python.exe',['-X','utf8',path.join(root,'scripts/verify_dmghg_direct_js.py'),'--workspace',workspace,'--artifact',artifact],{encoding:'utf8',windowsHide:true,timeout:180000,env:{...process.env,PYTHONIOENCODING:'utf-8'}});
 const report=JSON.parse(result.stdout.trim().split(/\r?\n/).at(-1));
 assert.ok(result.status===0&&report.passed,report.error||'standalone runtime failed');
 console.log(JSON.stringify({...report,target}));
} catch(error) {console.log(JSON.stringify({passed:false,target,error:error.message}));process.exitCode=1;}
