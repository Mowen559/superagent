"""Frozen, headless Hub delivery evaluations. Credentials are never report data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import time
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parent
MODEL = "gemini-3.8-flash-high"
APP = Path(r"D:\太极\动漫共和国概念版")
HOLDOUT = {"F03", "R02", "H03", "H04"}


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def save_new(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(data, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT).decode("utf-8", "replace").strip()


def file_record(path):
    path = Path(path).resolve()
    return {"path": str(path), "sha256": digest(path), "bytes": path.stat().st_size}


def app_snapshot():
    # Only executable resources, never account/session/cache databases.
    paths = [p for p in APP.iterdir() if p.is_file() and p.suffix.lower() in {".dll", ".exe"}]
    lua = APP / "resources" / "lua"
    if lua.is_dir():
        paths.extend(p for p in lua.rglob("*") if p.is_file() and p.suffix.lower() == ".lua")
    return {"root": str(APP), "resource_files": [file_record(p) for p in sorted(paths)],
            "account_state_read": False, "installed_directory_mutation_allowed": False}


def baseline(output):
    paths = []
    for group in ("src", "skills"):
        paths.extend(p for p in (ROOT / group).rglob("*")
                     if p.is_file() and p.suffix in {".rs", ".md", ".py", ".cjs", ".js", ".json"}
                     and "__pycache__" not in p.parts)
    paths.extend(ROOT / p for p in ("Cargo.toml", "Cargo.lock", "scripts/skill_worker_eval.py", "scripts/hub_worker_evals.py"))
    paths.extend(REPO / p for p in ("deepcloud-ui/src/lib/agent-sdk/client.ts",
        "deepcloud-ui/src/lib/agent-sdk/types.ts", "deepcloud-ui/native/gateway/local-gateway.js",
        "deepcloud-ui/native/hk-engine.js"))
    binaries = [file_record(p) for p in (ROOT / "target/debug/hub.exe", ROOT / "target/debug/superagent.exe") if p.is_file()]
    changes = git("status", "--porcelain=v1", "-uall")
    diff = subprocess.check_output(["git", "diff", "--binary"], cwd=ROOT)
    report = {"schema_version": 1, "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "root": str(ROOT), "head": git("rev-parse", "HEAD"), "branch": git("branch", "--show-current"),
        "git_status": changes, "preexisting_diff_sha256": hashlib.sha256(diff).hexdigest(),
        "files": [file_record(p) for p in sorted(set(paths)) if p.is_file()], "binaries": binaries,
        "application": app_snapshot(), "model": MODEL, "credential_policy": "reference-existing-local-config-only",
        "desktop_automation": False, "authorized_phases": ["P0", "P1", "P2"]}
    save_new(output, report)
    print(json.dumps({"baseline": str(output), "files": len(report["files"]), "binaries": len(binaries), "head": report["head"]}))


def suite(output):
    # These are task definitions, not implementations or claims of completed tests.
    definitions = [
      ("U01", "intent", ["只复核已有应用调查报告，核对结论边界。", "帮我审查报告是否夸大，只分析。", "查看已有证据，指出报告哪些话没有依据，不改代码。"], "review-report", ["citations_match", "no_business_mutation"]),
      ("U02", "intent", ["先调查计费错误再修复并验证。", "查清边界问题后直接改好，运行测试。", "定位问题、落实修复，给出实际测试结果。"], "checkout-fixture", ["boundary_oracle", "source_changed", "tests_unchanged"]),
      ("U03", "intent", ["继续刚才任务，新增限制是保留用户注释。", "接着处理未完成的修改，不要动已有备注。", "在上轮成果上完成剩余项，并保留原注释。"], "continuation-fixture", ["prior_artifact_preserved", "latest_constraint", "remaining_work_done"]),
      ("F01", "forward", ["满100免运费，修复当前实现并测试。", "金额正好100时仍收费，请改好。", "让100及以上运费为0，其他行为保持并验证。"], "checkout-fixture", ["boundary_oracle", "exports_preserved", "tests_unchanged"]),
      ("F02", "forward", ["修复订单接口金额字段类型与前端不一致的问题。", "对齐这里前后端金额契约，实际验证请求。", "接口金额返回类型有误，请统一契约并补文档。"], "contract-fixture", ["production_request", "type_contract", "documentation", "ledger"]),
      ("F03", "forward", ["为订单增加折扣总价并保留既有运费语义。", "实现跨模块优惠计算，保留旧接口。", "在这个小工程接入折扣能力，验证新旧行为。"], "cross-file-fixture", ["new_behavior", "old_behavior", "dependency_coverage"]),
      ("F04", "forward", ["修复计算错误，保留我尚未提交的改动。", "改好边界逻辑，不要覆盖现有备注和本地变更。", "完成当前修复并测试，原有未提交内容要留住。"], "dirty-workspace-fixture", ["boundary_oracle", "user_diff_preserved"]),
      ("R01", "reverse", ["解释请求如何进入工具执行，给出代码依据。", "追踪任务入口到实际工具调用的链路。", "帮我理解这里请求到执行的完整路径。"], "call-path-fixture", ["actual_path", "source_citations", "no_business_mutation"]),
      ("R02", "reverse", ["检查这个模块是真的可用还是只有声明。", "判断该能力是否实现，找出实际断点。", "审查现有功能的可执行性，不要只看注册名称。"], "declared-only-fixture", ["stub_detected", "availability_distinguished", "no_business_mutation"]),
      ("R03", "reverse", [str(APP) + "帮我看看这个应用是否能转换海阔规则。", "调查这个安装应用的规则转换可行性：" + str(APP), "分析" + str(APP) + "的数据与播放接口，说明转换边界。"], str(APP), ["target_identity", "probe_receipt", "bounded_claims", "mobile_unverified"]),
      ("H01", "rule", [str(APP) + "根据合适的skill生成实际可用的海阔规则，自己编写并验证。", "把这个安装应用转成可运行规则，交付文件和依赖：" + str(APP), "使用技能完成" + str(APP) + "的规则转换，跑通列表、搜索、详情及播放。"], str(APP), ["hub_authored", "same_target", "production_runtime", "data_coverage", "decode", "fresh_run"]),
      ("H02", "rule", ["把这个公开REST媒体目标做成海阔规则并实际验证。", "根据技能为指定公开API写可用媒体规则。", "转换这个REST数据源，交付可运行规则及证据。"], "https://archive.org/metadata/BigBuckBunny_328", ["same_target", "live_data", "production_runtime", "decode"]),
      ("H03", "rule", ["为这个HTML图书站生成海阔浏览规则，验证分页和详情。", "把指定图书网站转换成可用规则，独立分析结构。", "根据技能实现这个HTML目标的列表、翻页、搜索和详情。"], "https://books.toscrape.com/", ["same_target", "html_structure", "pagination_distinct", "production_runtime"]),
      ("H04", "rule", ["把这个公开测试视频目录做成带lazy解析的海阔规则。", "为指定测试流目标编写延迟解析规则并验证播放。", "生成这个媒体目标的规则，实际验证lazy得到的流。"], "https://test-streams.mux.dev/", ["same_target", "lazy_runtime", "headers_and_expiry", "decode"]),
      ("E01", "recovery", ["完成任务，首次工具参数错误时自行修正。", "遇到调用参数错误请根据返回信息恢复。", "把这次执行做完，读清工具报错后调整。"], "bad-argument-fixture", ["error_observed", "changed_arguments", "bounded_retries"]),
      ("E02", "recovery", ["按技能生成产物，缺依赖时在项目内恢复。", "完成规则检查，所需依赖仅允许装进临时目录。", "处理缺运行时问题后继续，不能改全局环境。"], "missing-dependency-fixture", ["scoped_recovery_or_blocked", "no_global_mutation", "truthful_status"]),
      ("E03", "recovery", ["接口可能暂时失败，完成数据任务并有界重试。", "处理这次网络波动，恢复后继续验证。", "完成请求，持续不可用时明确报告未完成。"], "transient-network-fixture", ["retry_budget", "eventual_success_or_blocked", "truthful_status"]),
      ("E04", "recovery", ["修复可解析但运行失败的规则，保持验收标准。", "根据业务运行错误改好规则，不要修改测试。", "让这个规则真正跑通，禁止降低验证要求。"], "runtime-failure-fixture", ["runtime_pass", "validator_unchanged", "bounded_retries"]),
      ("M01", "memory", ["在新任务里使用这个项目已经确认的配置事实。", "重启后继续，先核对本项目已有决策。", "调用之前保存的项目事实完成调查并注明来源。"], "restart-memory-fixture", ["valid_project_fact", "provenance", "restart_persistence"]),
      ("M02", "memory", ["核实当前配置，不要照搬其他项目或过期记录。", "已有记忆互相冲突，请依据当前事实判断。", "处理错误记忆并使用最新项目证据。"], "conflicting-memory-fixture", ["cross_project_rejected", "stale_rejected", "current_evidence"]),
    ]
    critical = {"U02-1", "U03-1", "F01-1", "F02-1", "F04-1", "R03-1", "H01-1", "H02-1", "H03-1", "H04-1"}
    cases = []
    for ident, category, prompts, target, criteria in definitions:
        cases.append({"id": ident, "category": category, "target": target, "held_out": ident in HOLDOUT,
            "criteria": criteria, "prior_exposure": ident in {"F01", "R03", "H01"},
            "tasks": [{"id": f"{ident}-{i+1}", "prompt": p, "critical": f"{ident}-{i+1}" in critical} for i, p in enumerate(prompts)]})
    probes = []
    for target in sorted({c["target"] for c in cases if c["target"].startswith("https:")}):
        try:
            with urlopen(Request(target, headers={"User-Agent": "DeepCloud-delivery-eval/1.0"}), timeout=25) as response:
                content = response.read(2_000_000)
                probes.append({"url": target, "status": response.status, "sample_sha256": hashlib.sha256(content).hexdigest(), "sample_bytes": len(content)})
        except Exception as exc:
            probes.append({"url": target, "status": "blocked", "error_type": type(exc).__name__})
    report = {"schema_version": 1, "model": MODEL, "cases": cases, "target_availability": probes,
        "case_count": len(cases), "task_count": sum(len(c["tasks"]) for c in cases), "critical_task_ids": sorted(critical),
        "holdout_case_ids": sorted(HOLDOUT), "task_state": "frozen_definitions_not_executed",
        "oracle_policy": "immutable-per-attempt; host changes => assisted; original outcomes retained",
        "budgets_provisional_until_baseline": {"investigation": {"turns": 12, "seconds": 300}, "implementation": {"turns": 32, "seconds": 900}}}
    assert report["case_count"] == 20 and report["task_count"] == 60 and len(critical) == 10
    assert all(c["held_out"] == (c["id"] in HOLDOUT) for c in cases)
    save_new(output, report)
    print(json.dumps({"suite": str(output), "cases": 20, "tasks": 60, "holdout": sorted(HOLDOUT), "availability": probes}))


def prepare_rule(output, model_turn_budget=32, timeout_seconds=900):
    output = Path(output).resolve()
    stage = Path(tempfile.mkdtemp(prefix="hub-dmghg-delivery-"))
    node = Path(shutil.which("node")).resolve(strict=True)
    dependencies = [Path(__file__).resolve(), ROOT / "scripts/dmghg_delivery_oracle.py",
        ROOT / "scripts/verify_dmghg_package.py",
        Path(sys.executable).resolve(), node,
        REPO / "deepcloud-ui/package.json", REPO / "deepcloud-ui/package-lock.json",
        ROOT / "skills/local-app-inspector/scripts/dmghg_probe.py",
        ROOT / "skills/local-app-inspector/SKILL.md", ROOT / "skills/deepcloud-rule-engineer/SKILL.md",
        REPO / "deepcloud-ui/scripts/hub-rule-runtime.cjs", REPO / "deepcloud-ui/scripts/hub-delivery-replay.cjs",
        REPO / "deepcloud-ui/scripts/hiker-standard-template.test.cjs",
        REPO / "deepcloud-ui/src/lib/agent-sdk/client.ts"]
    for skill in ['local-app-inspector', 'deepcloud-rule-engineer']:
        dependencies.extend(p.resolve() for p in (ROOT/'skills'/skill).rglob('*')
            if p.is_file() and p.suffix in {'.md','.py','.js','.cjs','.json'} and '__pycache__' not in p.parts)
    for folder in [REPO / "deepcloud-ui/native/runtime", REPO / "deepcloud-ui/native/hiker-js-core"]:
        dependencies.extend(p.resolve() for p in folder.rglob("*.js") if p.is_file())
    dependencies.extend(p for p in (ROOT / "src").rglob("*.rs") if p.is_file())
    dependencies.extend(ROOT / name for name in ["Cargo.toml", "Cargo.lock", "target/debug/hub.exe"])
    dependencies.extend(Path(r["path"]) for r in app_snapshot()["resource_files"])
    snapshot = ROOT/'frozen-source.json'
    provenance = json.loads(snapshot.read_text(encoding='utf-8')) if snapshot.is_file() else None
    if provenance:
        dependencies.append(snapshot)
        dependencies.extend(Path(f['path']) for f in provenance['files'])
    inputs_path = output.with_name(output.stem + "-inputs.json")
    save_new(inputs_path, {"schema_version": 1, "files": [file_record(p) for p in sorted(set(dependencies))],
        "source_head": provenance['source_head'] if provenance else git("rev-parse", "HEAD"),
        "source_diff_sha256": provenance['source_diff_sha256'] if provenance else hashlib.sha256(subprocess.check_output(["git", "diff", "--binary"], cwd=ROOT)).hexdigest(),
        "target": str(APP), "model": MODEL, "known_target_reuse": "existing protocol/probe Skill; not an unseen-task certification"})
    oracle = str(ROOT / "scripts/dmghg_delivery_oracle.py")
    oracle_args = ["--workspace", "{workspace}", "--artifact", "{artifact}", "--target", str(APP),
        "--node", str(node), "--inputs", str(inputs_path), "--inputs-sha256", digest(inputs_path)]
    profile_path = output.with_name(output.stem + "-profile.json")
    bridge_oracle = str(ROOT / "scripts/verify_dmghg_package.py")
    save_new(profile_path, {"delivery": {"version": 1, "scope": "implementation", "target": str(APP), "artifacts": [
        {"id": "rule", "path": "rule.json", "required": True, "validator": "dmghg-production-runtime"},
        {"id": "bridge", "path": "bridge.py", "required": True, "validator": "dmghg-bridge-contract"},
        {"id": "readme", "path": "README.md", "required": True}]},
        "validators": [{"id": "dmghg-production-runtime", "program": sys.executable, "program_sha256": digest(sys.executable),
            "script": oracle, "script_sha256": digest(oracle), "args": oracle_args, "timeout_seconds": 125},
            {"id": "dmghg-bridge-contract", "program": sys.executable, "program_sha256": digest(sys.executable),
             "script": bridge_oracle, "script_sha256": digest(bridge_oracle),
             "args": ["--workspace", "{workspace}", "--artifact", "{artifact}", "--target", str(APP), "--bridge-only"],
             "timeout_seconds": 45}]})
    requests_path = output.with_name(output.stem + "-requests.json")
    definitions = []
    for index in range(1, 4):
        project = stage / f"project-{index}"
        project.mkdir()
        definitions.append({"id": f"H01-independent-{index}", "oracle": "dmghg", "classification": "first_attempt", "prior_exposure": True,
            "request": {"agent": "superagent", "project": str(project), "model": MODEL, "workMode": "guarded",
                "thinkingLevel": "high", "timeoutSeconds": timeout_seconds,
                "prompt": str(APP) + " 根据合适的skill生成实际可用的海阔规则，亲自编写必要的本地适配并验证，不能只做调查。交付 rule.json、bridge.py、README.md，包含依赖、启动及导入方法。完全不使用桌面，不启动GUI，不改安装目录、不读取原账号，不发布或推送设备。"},
            "oracle_command": [sys.executable, oracle, "--workspace", "{workspace}", "--artifact", "{workspace}/rule.json", *oracle_args[4:]]})
    save_new(requests_path, definitions)
    save_new(output, {"schema_version": 1, "stage": str(stage), "profile": str(profile_path), "profile_sha256": digest(profile_path),
        "inputs": str(inputs_path), "inputs_sha256": digest(inputs_path), "requests": str(requests_path), "requests_sha256": digest(requests_path),
        "model": MODEL, "budget": {"model_turns": model_turn_budget, "timeout_seconds": timeout_seconds},
        "no_task_specific_host_implementation_after_start": True})
    print(json.dumps({"prepared": str(output), "profile": str(profile_path), "requests": str(requests_path)}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["baseline", "suite", "prepare-rule"])
    parser.add_argument("--output", required=True)
    parser.add_argument("--model-turn-budget", type=int, default=32)
    parser.add_argument("--timeout-seconds", type=int, default=900)
    args = parser.parse_args()
    if args.action == 'prepare-rule':
        assert 1 <= args.model_turn_budget <= 40 and 1 <= args.timeout_seconds <= 1800, 'invalid frozen budget: runtime ceiling is 40'
        prepare_rule(args.output, args.model_turn_budget, args.timeout_seconds)
    else:
        {"baseline": baseline, "suite": suite}[args.action](args.output)


if __name__ == "__main__":
    main()
