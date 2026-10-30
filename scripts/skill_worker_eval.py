"""Replay frontend-shaped requests through an isolated Hub HTTP service; no UI."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
from urllib.request import Request, urlopen


def http_json(url, payload=None, timeout=10):
    request = Request(url, data=json.dumps(payload).encode() if payload is not None else None,
                      headers={"Content-Type": "application/json"})
    with urlopen(request, timeout=timeout) as response:
        return json.load(response)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hub", default="target/debug/hub.exe")
    parser.add_argument("--credentials", required=True, help="Read key locally; never copied into receipts")
    parser.add_argument("--app-dir", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--only", choices=["fixture", "application"])
    args = parser.parse_args()
    hub = Path(args.hub).resolve(strict=True)
    output = Path(args.output).resolve()
    stage = Path(tempfile.mkdtemp(prefix="hub-skill-replay-"))
    workspace = stage / "workspace"
    workspace.mkdir()
    fixture = stage / "external skills" / "skill-execution-fixture"
    (fixture / "scripts").mkdir(parents=True)
    canary = "artifact-" + os.urandom(8).hex()
    (fixture / "SKILL.md").write_text(
        "---\nname: skill-execution-fixture\ndescription: >\n  生成本地验收工件\n  使用本技能脚本并独立校验\n---\n"
        "本技能是可执行链路的合成验收夹具，不是动漫业务规则。使用 scripts/generate.cjs <output> 生成 JSON，"
        "再调用 scripts/validate.cjs <output> 校验。不要自行写结果代替脚本。用 skill_tool 读取脚本。\n", encoding="utf-8")
    (fixture / "scripts/generate.cjs").write_text(
        "const fs=require('fs'),path=require('path');const out=process.argv[2];"
        "fs.mkdirSync(path.dirname(out),{recursive:true});fs.writeFileSync(out,JSON.stringify({source:'skill-script',canary:"
        + json.dumps(canary) + "}));console.log('generated '+out);", encoding="utf-8")
    (fixture / "scripts/validate.cjs").write_text(
        "const r=JSON.parse(require('fs').readFileSync(process.argv[2],'utf8'));"
        "if(r.source!=='skill-script'||r.canary!==" + json.dumps(canary)
        + ")process.exit(2);console.log('VERIFIED_SKILL_ARTIFACT');", encoding="utf-8")
    env = os.environ.copy()
    credentials = json.loads(Path(args.credentials).read_text(encoding="utf-8-sig"))
    key = credentials.get("apiKey") or credentials.get("api_key")
    if not key:
        raise RuntimeError("The selected credential file contains no API key")
    env.update(DEEPCLOUD_API_KEY=key, HUB_APP_DATA_ROOT=str(stage / "data"),
               HUB_SKILLS_ROOT=str(fixture.parent), DISABLE_FASTEMBED="1", HUB_AGENT_MAX_MODEL_TURNS="12")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    service_log = (stage / "service.log").open("wb")
    process = subprocess.Popen([str(hub), "service", str(port)], cwd=stage, env=env,
        stdin=subprocess.DEVNULL, stdout=service_log, stderr=subprocess.STDOUT,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    report = {"model": "gemini-3.8-flash-high", "transport": "POST /api/agents/run + SSE",
              "stage": str(stage), "hub_sha256": hashlib.sha256(hub.read_bytes()).hexdigest(), "cases": []}
    try:
        for _ in range(100):
            if process.poll() is not None:
                raise RuntimeError("Hub HTTP service exited during startup")
            try:
                http_json(base + "/api/health", timeout=1)
                break
            except Exception:
                time.sleep(0.2)
        else:
            raise RuntimeError("Hub HTTP health check timed out")
        prompts = {
            "fixture": "根据skill-execution-fixture生成 scratch/result.json，并使用该技能的校验脚本实际验证；按技能执行，不要只给方案。全程命令行，不使用桌面、不启动GUI，不发布。",
            "application": str(Path(args.app_dir)) + "帮我看看，我安装的这个应用，是否可以转化为海阔世界的一个可用规则。全程通过文件、日志和接口调查，不使用桌面，不启动GUI，不修改安装目录，不读取账户或输出鉴权密钥。不要换成同名网站；不能把静态校验或HTTP 200当成可播放证明。",
        }
        for name, prompt in prompts.items():
            if args.only and name != args.only:
                continue
            response = http_json(base + "/api/agents/run", {"agent": "superagent", "project": str(workspace),
                "prompt": prompt, "model": report["model"], "thinkingLevel": "high",
                "workMode": "guarded", "timeoutSeconds": 240})
            task_id = response["taskId"]
            events = []
            with urlopen(base + f"/api/agents/tasks/{task_id}/events", timeout=260) as stream:
                for raw in stream:
                    line = raw.decode("utf-8").strip()
                    if line.startswith("data:"):
                        events.append(json.loads(line[5:].strip()))
            text = json.dumps(events, ensure_ascii=False)
            event_file = stage / f"{name}.events.json"
            event_file.write_text(json.dumps(events, ensure_ascii=False, indent=2), encoding="utf-8")
            artifact = workspace / "scratch/result.json"
            artifact_passed = artifact.is_file() and json.loads(artifact.read_text()).get("canary") == canary
            statuses = [e.get("status") for e in events if e.get("kind") == "status"]
            calls = [e for e in events if e.get("kind") == "tool-use"]
            commands = [e.get("input", {}).get("CommandLine", e.get("input", {}).get("command", "")) for e in calls]
            case = {"name": name, "task_id": task_id, "event_count": len(events),
                    "status": statuses[-1] if statuses else "unknown",
                    "events_file": str(event_file), "event_kinds": sorted({e.get("kind", "") for e in events}),
                    "skill_tool_used": any(e.get("tool") == "skill_tool" for e in calls),
                    "generator_invoked": any("generate.cjs" in cmd for cmd in commands),
                    "probe_invoked": any("dmghg_probe.py" in cmd for cmd in commands),
                    "validation_receipt": "VERIFIED_SKILL_ARTIFACT" in text,
                    "artifact_verified": artifact_passed if name == "fixture" else None}
            if name == "application":
                evidence_file = workspace / "scratch/dmghg-evidence.json"
                evidence = json.loads(evidence_file.read_text(encoding="utf-8")) if evidence_file.is_file() else {}
                case["application_evidence_verified"] = bool(evidence) and (
                    Path(evidence.get("application", "")).resolve() == Path(args.app_dir).resolve()
                    and evidence.get("dll_sha256") == hashlib.sha256((Path(args.app_dir) / "electron_bridge.dll").read_bytes()).hexdigest()
                    and evidence.get("no_desktop") is True
                    and evidence.get("channels", 0) > 0 and evidence.get("search_items", 0) > 0
                    and evidence.get("episode_count", 0) > 0
                    and evidence.get("decoder", {}).get("decoded_video") is True
                    and evidence.get("mobile_standalone_verified") is False)
                case["application_evidence"] = evidence
            case["workflow_passed"] = case["status"] == "completed" and (name != "fixture" or (
                artifact_passed and case["generator_invoked"] and case["validation_receipt"]))
            if name == "application":
                case["workflow_passed"] &= case["probe_invoked"] and case["application_evidence_verified"]
            report["cases"].append(case)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(case, ensure_ascii=False), flush=True)
    finally:
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True)
        service_log.close()


if __name__ == "__main__":
    main()
