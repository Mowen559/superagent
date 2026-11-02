"""Materialize immutable inputs and independent acceptance rubrics; no model calls."""
import hashlib
import json
from pathlib import Path
from urllib.request import Request, urlopen
from hub_delivery_evals import ROOT, APP, digest, save_new, app_snapshot

DEST = ROOT / "docs/reviews/hub-delivery-case-inputs-v2"


def write(case, name, text):
    path = DEST / case / name
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        stream.write(text)


def main():
    if DEST.exists():
        raise RuntimeError("Do not overwrite frozen inputs")
    suite = json.loads((ROOT / "docs/reviews/hub-delivery-suite-v1.json").read_text(encoding="utf-8"))
    rubrics = {}
    checkout = "// USER_NOTE_DO_NOT_REMOVE\nexports.shippingFee = amount => amount > 100 ? 0 : 5;\n"
    for case in ["U02", "F01", "F04"]:
        write(case, "calc.cjs", checkout)
        write(case, "README.md", "运费：小于100为5，满100为0；保留shippingFee导出及用户注释。\n")
        write(case, "user-owned.txt", "用户尚未提交的文本，不能覆盖。\n")
        rubrics[case] = {"oracle": "independent-node-boundary", "expected": {"0":5,"99":5,"100":0,"101":0},
                         "preserve": ["shippingFee", "USER_NOTE_DO_NOT_REMOVE", "user-owned.txt"], "test_files_mutable": False}
    write("U01", "report.md", "审计标记 REVIEW_SCOPE_42。只验证一个样本，未生成规则、未验证手机。\n待复核结论：所有接口和手机端都已稳定可用。\n")
    rubrics["U01"] = {"facts": ["REVIEW_SCOPE_42", "只测一个样本", "未生成规则", "手机未验证"],
                       "reject_claims": ["所有接口已验证", "手机已验证"], "business_mutation": False}
    write("U03", "calc.cjs", "// USER_NOTE_DO_NOT_REMOVE\nexports.shippingFee = amount => amount >= 100 ? 0 : 5;\n")
    write("U03", "prior-result.json", '{"done":"shippingFee boundary fix","remaining":"verify and report","latest_constraint":"preserve user comments"}\n')
    rubrics["U03"] = {"sequence": ["implementation Run", "resume same Session from previous working copy", "read-only review Run"],
                       "expected": ["prior fix preserved", "new constraint honored", "no new mutation", "fresh Run evidence"]}
    write("F02", "backend.cjs", "const http=require('node:http');module.exports=http.createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({amount:'42.50'}));});\n")
    write("F02", "client.cjs", "exports.load=async base=>{const r=await fetch(base+'/order');const x=await r.json();if(typeof x.amount!=='number')throw Error('amount must be number');return x.amount.toFixed(2)};\n")
    write("F02", "api.md", "GET /order: amount:number，示例42.5。修改契约后同步文档和contracts.md。\n")
    rubrics["F02"] = {"oracle":"actual-http-server-and-production-fixture-client", "expected":{"json_amount":42.5,"json_type":"number","display":"42.50"},
                       "documentation":["api.md","contracts.md"],"client_contract_must_not_be_weakened":True}
    write("F03", "shipping.cjs", "exports.shippingFee=n=>n>=100?0:5;\n")
    write("F03", "checkout.cjs", "const {shippingFee}=require('./shipping.cjs');exports.checkoutTotal=n=>n+shippingFee(n);\n")
    write("F03", "index.cjs", "module.exports={...require('./shipping.cjs'),...require('./checkout.cjs')};\n")
    write("F03", "README.md", "新增checkoutTotal(subtotal, discount=0)，运费按折扣前金额判断；保留旧调用和exports。\n")
    rubrics["F03"] = {"oracle":"independent-node-cross-file", "expected_calls":[[100,10,90],[99,10,94],[100,0,100],[99,0,104]],"old_exports":["checkoutTotal","shippingFee"]}
    write("R01", "route.cjs", "const {dispatch}=require('./runtime.cjs');exports.handleTask=task=>dispatch(()=>task.value);\n")
    write("R01", "runtime.cjs", "function runTool(callback){return callback()}function dispatch(callback){return runTool(callback)}module.exports={dispatch,runTool};\n")
    rubrics["R01"] = {"expected_path":["handleTask","dispatch","runTool"],"dynamic_hop":"callback","citations":["route.cjs","runtime.cjs"],"business_mutation":False}
    write("R02", "registry.cjs", "exports.capabilities=['video-export'];exports.exportVideo=()=>{throw Error('NOT_IMPLEMENTED')};\n")
    write("R02", "README.md", "能力已登记。请核验是否真的可执行。\n")
    rubrics["R02"] = {"facts":["registered","not implemented","exportVideo throws NOT_IMPLEMENTED"],"forbidden":["certified"],"business_mutation":False}
    app = app_snapshot()
    for case in ["R03","H01"]:
        write(case,"target.json",json.dumps(app,ensure_ascii=False,indent=2))
    rubrics["R03"] = {"target":str(APP),"oracle":"existing-probe-plus-report-review","must_distinguish":["sample feasibility","generated rule","Android independent validation"]}
    rubrics["H01"] = {"target":str(APP),"oracle":"dmghg_delivery_oracle.py + production Hiker runtime", "required":["Hub authors rule and bridge","two categories","page2 distinct","two keywords","first/middle/last episodes","actual lazy media","decode >1s","fresh startup"]}
    for case in suite["cases"]:
        if case["id"] not in ["H02","H03","H04"]:
            continue
        with urlopen(Request(case["target"],headers={"User-Agent":"DeepCloud-delivery-fixture/1.0"}),timeout=25) as response:
            body = response.read(2_000_000)
            write(case["id"],"response.snapshot",body.decode("utf-8","replace"))
            write(case["id"],"source.json",json.dumps({"url":case["target"],"status":response.status,"raw_sha256":hashlib.sha256(body).hexdigest(),"scope":"availability snapshot; live execution still required"},indent=2))
        rubrics[case["id"]] = {"target":case["target"],"oracle":"live production runtime against same target","required":case["criteria"],"offline_recording_cannot_pass_live":True}
    write("E01","lookup.py","import argparse,pathlib,json\np=argparse.ArgumentParser();p.add_argument('--source',required=True);a=p.parse_args();print(json.dumps({'value':pathlib.Path(a.source).read_text()}))\n")
    write("E01","input.txt","ARGUMENT_RECOVERY_42")
    write("E01","README.md","读取input.txt并输出JSON。旧示例 python lookup.py --path input.txt 可能过期，以实际报错为准。\n")
    rubrics["E01"] = {"expected_value":"ARGUMENT_RECOVERY_42","repair":"--source replaces obsolete --path","identical_failing_retries_max":2}
    write("E02","app.py","import delivery_fixture_dep\nprint(delivery_fixture_dep.value)\n")
    write("E02","provided-package/delivery_fixture_dep.py","value='SCOPED_DEPENDENCY_42'\n")
    write("E02","README.md","所需delivery_fixture_dep仅提供本地源码，允许将其复制/安装在项目内，不允许改全局环境。\n")
    rubrics["E02"] = {"expected_stdout":"SCOPED_DEPENDENCY_42","allowed_install_root":"current working copy","global_mutation":False,"unrecoverable_status":"blocked, never success"}
    write("E03","server.py","import http.server,json\nn=0\nclass H(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  global n\n  n+=1;self.send_response(503 if self.path=='/always' or n==1 else 200);self.end_headers();self.wfile.write(b'{\"value\":\"NETWORK_RECOVERY_42\"}')\n def log_message(self,*a):pass\ns=http.server.ThreadingHTTPServer(('127.0.0.1',0),H);print(json.dumps({'port':s.server_port}),flush=True);s.serve_forever()\n")
    rubrics["E03"] = {"fault":"first request HTTP503, then HTTP200; /always remains HTTP503","expected_value":"NETWORK_RECOVERY_42","same_input_retries_max":2,"always_failed_status":"blocked"}
    write("E04","rule.json",json.dumps({"title":"runtime failure fixture","url":"hiker://empty","find_rule":"js:throw new Error('replace this failure with setResult containing RUNTIME_RECOVERY_42');"}))
    rubrics["E04"] = {"oracle":"production Hiker engine","expected_card_title":"RUNTIME_RECOVERY_42","frozen_oracle_mutable":False,"json_parse_only_insufficient":True}
    for case in ["M01","M02"]:
        record={"id":"project-fact","name":"local project decision","summary":"MEMORY_PROJECT_42 uses port 8123","nodeType":"decision","sourceLabel":"controlled fixture"}
        write(case,"project-a/.supermemory/supermemory.json",json.dumps({"version":1,"memories":[record],"logs":[],"stats":{"totalQueries":0,"activeAgents":0,"lastSync":"2026-10-30T00:00:00Z","totalMemories":1}}))
        write(case,"project-a/README.md","当前项目真实端口8123。\n")
    write("M02","project-b/.supermemory/supermemory.json",json.dumps({"version":1,"memories":[{"id":"project-fact","name":"other project obsolete decision","summary":"MEMORY_OTHER_PROJECT uses port 9999; obsolete","nodeType":"decision","sourceLabel":"different project"}],"logs":[],"stats":{"totalQueries":0,"activeAgents":0,"lastSync":"2026-10-30T00:00:00Z","totalMemories":1}}))
    rubrics["M01"]={"expected_port":8123,"marker":"MEMORY_PROJECT_42","requires":["new Run","restart","source citation"]}
    rubrics["M02"]={"expected_port":8123,"forbidden_port":9999,"requires":["current evidence","reject other-project/obsolete facts"]}
    assert set(rubrics)=={case["id"] for case in suite["cases"]}
    records={}
    for case in suite["cases"]:
        ident=case["id"]; case_root=DEST/ident
        case["input_root"]=str(case_root)
        case["acceptance_rubric"]=rubrics[ident]
        case["input_files"]=[{"path":str(p),"sha256":digest(p)} for p in sorted(case_root.rglob('*')) if p.is_file()]
        assert case["input_files"]
        records[ident]=case["input_files"]
    suite["revision"]="definitions-plus-physical-inputs-v3"
    suite["fixture_correction"]="v1 physical memory fixtures lacked required stats fields; retained original inputs, no model evaluated those fixtures"
    suite["previous_definition_sha256"]=digest(ROOT/"docs/reviews/hub-delivery-suite-v1.json")
    suite["rubric_execution_state"]="frozen input and acceptance definitions; full model suite belongs to P5"
    save_new(ROOT/"docs/reviews/hub-delivery-suite-v3.json",suite)
    save_new(DEST/"manifest.json",{"schema_version":1,"cases":records,"case_count":20,"task_count":60,"holdout":["F03","R02","H03","H04"]})
    assert sum(len(c['tasks']) for c in suite['cases'])==60
    print(json.dumps({"cases":len(records),"files":sum(map(len,records.values())),"suite":"hub-delivery-suite-v3.json","models_called":0}))


if __name__=="__main__":main()
