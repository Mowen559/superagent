"""Exercise the real Hub worker in an isolated, genuinely indexed Node project.

Set SUPERAGENT_EVAL_API_KEY in the launcher environment; never in a file.
No production project, account configuration, or installed binary is modified.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
import re


def run(args, cwd, env, timeout=180):
    started = time.monotonic()
    process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    try:
        out, err = process.communicate(timeout=timeout)
        return {"exit_code": process.returncode, "stdout": out.decode("utf-8", "replace"),
                "stderr": err.decode("utf-8", "replace"),
                "duration_ms": round((time.monotonic() - started) * 1000)}
    except subprocess.TimeoutExpired:
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True)
        else:
            process.kill()
        out, err = process.communicate()
        return {"exit_code": None, "timed_out": True,
                "stdout": out.decode("utf-8", "replace"), "stderr": err.decode("utf-8", "replace")}


def parse_lines(stdout):
    messages = []
    for line in stdout.splitlines():
        try:
            messages.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return messages


def graph_prelude_evidence(messages, project, launch):
    """Count runtime graph use only from the matching Run's raw input snapshot.

    Assistant prose and another project's snapshots cannot satisfy this check.
    """
    evidence = []
    for message in messages:
        if message.get("type") != "trace_snapshot":
            continue
        path = (launch / message.get("snapshot_file", "")).resolve()
        if not path.is_relative_to((launch / ".superagent" / "traces").resolve()) or not path.is_file() or path.stat().st_size > 3_000_000:
            continue
        raw = path.read_bytes()
        snapshot = json.loads(raw)
        prompt = snapshot.get("system_prompt", "")
        block = re.search(r'<knowledge-evidence source="code-graph">(.*?)</knowledge-evidence>', prompt, re.S)
        if (snapshot.get("session_id") == message.get("session_id")
                and f"- Current Directory: {project}" in prompt and block
                and "symbol=checkoutTotal" in block[1] and "symbol=shippingFee" in block[1]):
            evidence.append({"turn": snapshot["turn"], "snapshot_sha256": hashlib.sha256(raw).hexdigest(),
                             "workspace_matches": True, "source": "runtime_code_graph",
                             "graph_excerpt": block[1].strip()})
    return evidence


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--hub", default="target/debug/hub.exe")
    parser.add_argument("--model", default="gemini-3.8-flash-high")
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--only", nargs="+", choices=["reverse", "forward", "requirement", "natural"])
    parser.add_argument("--review", help="Offline review of runtime graph evidence in an existing natural-case report")
    parser.add_argument("--output", default="docs/reviews/2026-10-30-hub-worker-eval.json")
    args = parser.parse_args()
    launch = Path.cwd()
    if args.review:
        if args.live:
            raise RuntimeError("Review is offline; do not combine with --live")
        report = json.loads(Path(args.review).read_text(encoding="utf-8-sig"))
        report["evaluation_rule_version"] = 2
        report["review_note"] = "Graph use includes a matching Run's runtime retrieval, not only explicit model tool calls. Original report remains unchanged."
        for case in report["cases"]:
            if case["case"] != "natural":
                raise RuntimeError("Offline graph review only supports the natural investigation case")
            evidence = graph_prelude_evidence(case["messages"], Path(report["fixture"]) / "project", launch)
            final_text = " ".join(m.get("result", "") for m in case["messages"] if m.get("type") == "status" and m.get("status") == "completed")
            case["original_tool_call_only_passed"] = case["passed"]
            case["runtime_graph_evidence"] = evidence
            case["passed"] = bool(evidence) and report["offline_readiness_passed"] and case["source_unchanged"] and case["acceptance_unchanged"] and case["launcher_checklist_unchanged"] and "HUB_PROJECT_MEMORY_42" in final_text
        Path(args.output).write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print("Offline runtime evidence review:", all(case["passed"] for case in report["cases"]))
        raise SystemExit(0 if all(case["passed"] for case in report["cases"]) else 1)
    hub = (launch / args.hub).resolve()
    fixture = Path(tempfile.mkdtemp(prefix="hub-native-eval-"))
    project = fixture / "project"
    project.mkdir()
    # Only fixture data is written. The API key remains in the child environment.
    (project / "README.md").write_text("Node CommonJS checkout. shippingFee returns 5 below 100 and 0 at or above 100. checkoutTotal calls shippingFee. Preserve function names. Test: node --test acceptance.test.cjs\n", encoding="utf-8")
    (project / "package.json").write_text('{"name":"hub-native-eval","version":"1.0.0"}', encoding="utf-8")
    (project / "calc.cjs").write_text("function shippingFee(subtotal) { return subtotal > 100 ? 0 : 5; }\nfunction checkoutTotal(subtotal) { return subtotal + shippingFee(subtotal); }\nmodule.exports = { shippingFee, checkoutTotal };\n", encoding="utf-8")
    (project / "acceptance.test.cjs").write_text("const test=require('node:test'); const assert=require('node:assert/strict'); const {shippingFee,checkoutTotal}=require('./calc.cjs'); test('fee boundaries',()=>{assert.equal(shippingFee(0),5);assert.equal(shippingFee(99),5);assert.equal(shippingFee(100),0);assert.equal(shippingFee(101),0);assert.equal(checkoutTotal(100),100)});\n", encoding="utf-8")
    memory_dir = project / ".supermemory"
    memory_dir.mkdir()
    memory = {"version": 1, "memories": [{"id":"checkout-memory", "name":"checkout shippingFee boundary memory", "summary":"Checkout audit marker is HUB_PROJECT_MEMORY_42. Preserve CommonJS exports and the >=100 free-shipping contract.", "nodeType":"decision", "sourceLabel":"eval-fixture"}], "logs":[], "stats":{"totalQueries":0,"activeAgents":0,"lastSync":"2026-10-30T00:00:00Z","totalMemories":1}}
    (memory_dir / "supermemory.json").write_text(json.dumps(memory, ensure_ascii=False), encoding="utf-8")
    env = os.environ.copy()
    for name in ["HUB_CODEBASE_MEMORY_DB_PATH", "HUB_SHADOW_GIT_ROOT", "HUB_KNOWLEDGE_GRAPH_PATH", "HUB_AGENT_SESSION_ID"]:
        env.pop(name, None)
    env.update(HUB_APP_DATA_ROOT=str(fixture / "app-data"),
               HUB_WORKSPACE_ROOT=str(project), HUB_PROJECT_ROOT=str(project),
               HUB_CODEBASE_MEMORY_CACHE_ROOT=str(fixture / "graphs"),
               HUB_AGENT_MAX_MODEL_TURNS="12", DISABLE_FASTEMBED="1")
    indexer = shutil.which("codebase-memory-mcp")
    report = {"schema_version":1,"scope":"real_hub_superagent_worker; isolated Node fixture; no full product certification",
              "model_requested":args.model,"fixture":str(fixture),"launch_cwd":str(launch), "cases":[]}
    if indexer:
        env["HUB_CODEBASE_MEMORY_BIN"] = indexer
        report["indexing"] = run([str(hub), "index", "--project", str(project)], launch, env, 150)
    else:
        report["indexing"] = {"status":"indexer_missing"}
    # Discover the actual DB emitted by the real indexer; no seeded graph.
    databases = list((fixture / "graphs").glob("*.db"))
    indexed = None
    for path in databases:
        try:
            with sqlite3.connect(str(path)) as db:
                if db.execute("select count(*) from nodes").fetchone()[0] > 0:
                    indexed = path
                    break
        except sqlite3.Error:
            pass
    if indexed:
        env["HUB_CODEBASE_MEMORY_DB_PATH"] = str(indexed)
        report["graph_database"] = str(indexed)
    report["doctor"] = run([str(hub), "doctor", "--project", str(project)], launch, env, 30)
    report["initial_check"] = run(["node", "--test", "acceptance.test.cjs"], project, env, 10)
    try:
        readiness = json.loads(report["doctor"]["stdout"])
        report["offline_readiness_passed"] = bool(indexed) and readiness["graph"]["ready"] is True and report["initial_check"]["exit_code"] == 1
    except (json.JSONDecodeError, KeyError):
        report["offline_readiness_passed"] = False
    if args.live:
        key = os.environ.get("SUPERAGENT_EVAL_API_KEY")
        if not key:
            raise RuntimeError("SUPERAGENT_EVAL_API_KEY is required for live testing")
        env["DEEPCLOUD_API_KEY"] = key
        env.pop("SUPERAGENT_EVAL_API_KEY", None)
        prompts = [
            ("reverse", "只做调查：请检查当前工作区 checkout/shippingFee，调用 capability_status、project_index(inspect_overview)、search_graph(trace_path,query=checkoutTotal) 和 super_memory(search,query=checkout shippingFee) 取得真实结果，再读取 calc.cjs，解释调用链与边界错误，并报告项目记忆中的 audit marker。不得修改业务源码、不得运行子智能体，不能把缺失图谱说成已查询成功。"),
            ("forward", "修复当前工作区 calc.cjs：shippingFee 在 subtotal 等于100时也应免运费。先读当前文件与项目约定，再用最小修改保留 CommonJS 导出、checkoutTotal及其他代码；实际运行 node --test acceptance.test.cjs 并简洁报告结果。不得改验收测试，不得调用子智能体，不得后台运行命令。"),
            ("requirement", "只做规划：调用 capability_status 确认图谱，再调用 requirement_engine(Action=analyze) 和 requirement_engine(Action=plan)，需求为‘为当前 checkout 新增可配置免运费阈值，保持默认100与CommonJS兼容，不增加依赖’。报告实际生成的 .deepcloud 产物路径，明确尚未实施。不要直接修改业务代码，不调用子智能体。"),
            ("natural", "只做调查：分析当前项目 checkoutTotal 到 shippingFee 的调用关系，结合项目既有记忆说明100这个边界的业务契约，并给出记忆中记录的审计标记。引用真实源码位置并指出当前实现的问题，简洁报告即可。不要修改业务源码，不要启动子智能体。"),
        ]
        selected_prompts = [(name, prompt) for name, prompt in prompts if name in (args.only or ["reverse", "forward", "requirement"])]
        for name, prompt in selected_prompts:
            env["HUB_AGENT_SESSION_ID"] = f"eval-{name}"
            before = hashlib.sha256((project / "calc.cjs").read_bytes()).hexdigest()
            test_before = hashlib.sha256((project / "acceptance.test.cjs").read_bytes()).hexdigest()
            launcher_todo = launch / ".superagent_todo.md"
            launcher_todo_before = launcher_todo.read_bytes() if launcher_todo.exists() else None
            result = run([str(hub), "superagent-worker", "--project", str(project), "--model", args.model,
                          "--tool-policy", "guarded", "--prompt", prompt], launch, env, 240)
            result.update(case=name, source_unchanged=before == hashlib.sha256((project / "calc.cjs").read_bytes()).hexdigest(),
                          acceptance_unchanged=test_before == hashlib.sha256((project / "acceptance.test.cjs").read_bytes()).hexdigest(),
                          messages=parse_lines(result.get("stdout", "")))
            result["launcher_checklist_unchanged"] = launcher_todo_before == (launcher_todo.read_bytes() if launcher_todo.exists() else None)
            if name == "forward":
                result["independent_check"] = run(["node", "--test", "acceptance.test.cjs"], project, env, 10)
                result["final_source"] = (project / "calc.cjs").read_text(encoding="utf-8")
            if name == "requirement":
                result["artifacts"] = {p.name:p.read_text(encoding="utf-8") for p in (project / ".deepcloud").glob("*") if p.is_file()}
            results = [m for m in result["messages"] if m.get("type") == "tool_result"]
            aliases = {"read_file":"view_file", "write_file":"write_to_file", "edit_file":"replace_in_file", "bash":"run_command"}
            canonical = lambda name: aliases.get(name, name)
            called = {canonical(m.get("tool")) for m in result["messages"] if m.get("type") == "tool_call"}
            completed = any(m.get("type") == "status" and m.get("status") == "completed" for m in result["messages"])
            def good_receipt(tool):
                return any(canonical(m.get("tool")) == tool and isinstance(m.get("result"), dict)
                           and not m["result"].get("error")
                           and m["result"].get("exit_code", 0) == 0 for m in results)
            if name in {"reverse", "natural"}:
                final_text = " ".join(m.get("result", "") for m in result["messages"] if m.get("type") == "status")
                graph_call = any(tool in called and good_receipt(tool) for tool in ["project_index", "search_graph", "repo_map_tool"])
                result["runtime_graph_evidence"] = graph_prelude_evidence(result["messages"], project, launch)
                if name == "natural":
                    graph_call = graph_call or bool(result["runtime_graph_evidence"])
                required_calls = name == "natural" or ({"capability_status", "project_index", "search_graph", "super_memory", "view_file"} <= called and all(good_receipt(tool) for tool in ["project_index", "search_graph", "super_memory"]))
                result["passed"] = bool(indexed) and completed and result["source_unchanged"] and result["acceptance_unchanged"] and graph_call and required_calls and "HUB_PROJECT_MEMORY_42" in final_text
            elif name == "forward":
                result["passed"] = completed and not result["source_unchanged"] and result["acceptance_unchanged"] and good_receipt("run_command") and result["independent_check"]["exit_code"] == 0
                result["rtk_observed"] = any(isinstance(m.get("result"), dict) and "rtk_metrics" in m["result"] for m in results)
            else:
                artifacts = result["artifacts"]
                try:
                    plan = json.loads(artifacts.get("execution_plan.json", ""))
                    valid_plan = isinstance(plan, list) and len(plan) > 0 and all(isinstance(item, dict) and item for item in plan)
                except json.JSONDecodeError:
                    valid_plan = False
                actions = {m["result"].get("action") for m in results if m.get("tool") == "requirement_engine" and isinstance(m.get("result"), dict) and m["result"].get("status") == "generated"}
                result["passed"] = completed and result["source_unchanged"] and result["acceptance_unchanged"] and actions == {"analyze", "plan"} and bool(artifacts.get("requirement_impact.md")) and valid_plan
            result["called_tools"] = sorted(called)
            result["passed"] = result["passed"] and result["launcher_checklist_unchanged"]
            report["cases"].append(result)
            print(f"[HubWorkerEval] {name}: passed={result['passed']}; process={result['exit_code']}; tools={result['called_tools']}", flush=True)
    output = launch / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"Report: {output}", flush=True)
    if args.live and (len(report["cases"]) != len(selected_prompts) or not all(case.get("passed") for case in report["cases"])):
        raise SystemExit(1)
    if not args.live and not report["offline_readiness_passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
