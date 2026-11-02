"""Independent business oracle; never writes the Hub's rule or bridge implementation."""
import argparse
import ctypes
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import socket
import subprocess
import sys
import time
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
PROBE_PATH = ROOT / "skills/local-app-inspector/scripts/dmghg_probe.py"
RUNTIME_PATH = ROOT.parent / "deepcloud-ui/scripts/hub-rule-runtime.cjs"


def discloses_mobile_verification_boundary(instructions):
    # Evaluate the mobile statement itself, not unrelated "untested" prose.
    mobile = re.compile(r'Android|手机|移动端', re.I)
    negative = re.compile(
        r'(?:未|未经|没有|尚无)[^。！？；;\n]{0,100}(?:验证|实测|测试|认证)'
        r'|待[^。！？；;\n]{0,30}(?:验证|实测|测试|认证)'
        r'|\b(?:unverified|untested|unvalidated|not(?:\s+[a-z]+){0,3}\s+(?:verified|tested|validated|certified))\b', re.I)
    statements = re.split(r'[。！？；;\n]|(?<=[a-z])\.(?:\s|$)', instructions, flags=re.I)
    return any(mobile.search(statement) and negative.search(statement) for statement in statements)


def sha(path):
    return hashlib.file_digest(Path(path).open("rb"), "sha256").hexdigest()


def module():
    spec = importlib.util.spec_from_file_location("trusted_existing_probe", PROBE_PATH)
    probe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(probe)
    return probe


def verification_ports(artifact):
    rule = json.loads(Path(artifact).read_text(encoding='utf-8-sig'))
    if isinstance(rule, list):
        assert len(rule) == 1, 'expected one rule'
        rule = rule[0]
    address = urlsplit(rule.get('url', ''))
    default = address.port if address.hostname in ['127.0.0.1', 'localhost'] else 18002
    default = default or 18002
    for candidate in [18002, 18001, 18000]:
        if candidate == default:
            continue
        try:
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', candidate))
            return default, candidate
        except OSError:
            continue
    raise AssertionError('no free alternate approved loopback engine port')


class DecodeProgressError(AssertionError):
    def __init__(self, attempts):
        self.decoder_attempts = attempts
        last = attempts[-1]
        facts = {}
        for key in ["video-params/w", "video-params/h", "time-pos"]:
            value = str(last.get(key, ""))
            facts[key] = value[:24] if re.fullmatch(r"-?\d+(?:\.\d+)?", value) else "unavailable"
        facts['diagnostic_codes'] = last.get('diagnostic_codes', [])
        for key in ['end_file_reason', 'end_file_error']:
            if isinstance(last.get(key), int):
                facts[key] = last[key]
        super().__init__("one second actual decoding not reached; sample=first result of search 仙逆 / first episode; "
                         f"attempts={len(attempts)}; " + json.dumps(facts, ensure_ascii=False))


def verify_decode_progress(probe, native, variant):
    attempts = []
    for _ in range(3):
        decoded = probe.decode_probe(native, variant, progress_seconds=1.0, diagnostics=True)
        attempts.append(decoded)
        if decoded.get('decoded_video') is True:
            return decoded, attempts
    raise DecodeProgressError(attempts)


def reference(bridge):
    channels = bridge.call("catalog.get_channels")
    category_pages = []
    for category in channels[:2]:
        listing = bridge.call("catalog.get_video_list", {"channel": category["id"], "page": 1})["items"]
        assert listing and all(item.get("cid") == category["id"] for item in listing), "upstream category contract mismatch"
        category_pages.append({"id": category["id"], "name": category["name"], "ids": [str(v["id"]) for v in listing]})
    page1 = bridge.call("catalog.get_video_list", {"page": 1})["items"]
    page2 = bridge.call("catalog.get_video_list", {"page": 2})["items"]
    assert page1 and page2, "upstream lists empty"
    searches = []
    selected = None
    for keyword in ["仙逆", "斗破"]:
        items = bridge.call("catalog.search_video", {"key": keyword, "page": "1"})["items"]
        assert items, "upstream search empty"
        searches.append({"keyword": keyword, "ids": [str(item["id"]) for item in items]})
        if selected is None:
            selected = items[0]
    detail = bridge.call("catalog.get_video_detail", {"id": str(selected["id"])})
    group = next(part for part in detail["parts"] if part.get("part"))
    episodes = group["part"]
    # Episode part entries are actual upstream strings, not inferred numbering.
    return {"channels": category_pages,
        "page1_ids": [str(v["id"]) for v in page1], "page2_ids": [str(v["id"]) for v in page2],
        "pagination_applicable": [v["id"] for v in page1] != [v["id"] for v in page2],
        "searches": searches, "detail_id": str(selected["id"]), "detail_title": detail["name"],
        "episodes": [str(e) for e in episodes], "episode_count": len(episodes)}


def playback_variant(value):
    if isinstance(value, dict):
        if isinstance(value.get("urls"), list) and value["urls"]:
            variant = playback_variant(value["urls"][0])
            headers = value.get("headers")
            if isinstance(headers, list) and headers:
                headers = headers[0]
            if isinstance(headers, dict):
                variant["headers"].update(headers)
            return variant
        for key in ["url", "playUrl", "play_url", "result", "data"]:
            if key in value:
                variant = playback_variant(value[key])
                if isinstance(value.get("headers"), dict):
                    variant["headers"].update(value["headers"])
                return variant
    if isinstance(value, list):
        for item in value:
            try:
                return playback_variant(item)
            except (AssertionError, ValueError):
                pass
    assert isinstance(value, str), "lazy did not produce a playback URL"
    if value.lstrip().startswith(("{", "[")):
        return playback_variant(json.loads(value))
    value = value.replace("#isVideo=true#", "").replace("#noLoading#", "")
    headers = {}
    if ";{" in value:
        value, suffix = value.split(";{", 1)
        for field in suffix.rstrip("}").split("&&"):
            if "@" in field:
                key, val = field.split("@", 1)
                headers[key] = val
    assert value.startswith(("https://", "http://")), "lazy returned a non-network playback value"
    return {"url": value, "headers": headers}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--artifact", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--node", default="node")
    parser.add_argument("--inputs")
    parser.add_argument("--inputs-sha256")
    args = parser.parse_args()
    if args.inputs:
        assert sha(args.inputs) == args.inputs_sha256, "frozen oracle input manifest changed"
        frozen = json.loads(Path(args.inputs).read_text(encoding="utf-8"))
        assert all(sha(record["path"]) == record["sha256"] for record in frozen["files"]), "frozen dependency or runtime changed"
    workspace = Path(args.workspace).resolve(strict=True)
    artifact = Path(args.artifact).resolve(strict=True)
    assert artifact.is_relative_to(workspace), "artifact outside execution workspace"
    app = Path(args.target).resolve(strict=True)
    bridge_script = workspace / "bridge.py"
    readme = workspace / "README.md"
    assert bridge_script.is_file() and readme.is_file(), "missing bridge.py or README.md"
    instructions = readme.read_text(encoding="utf-8-sig")
    assert all(word.lower() in instructions.lower() for word in ["bridge.py", "--app-dir", "--port", "Python", "Windows"]), "README lacks runnable dependencies/startup"
    assert discloses_mobile_verification_boundary(instructions), 'README must disclose the mobile verification boundary in its mobile statement, for example: Android独立运行未验证'
    identities = {str(p): sha(p) for p in [artifact, bridge_script, readme, PROBE_PATH, RUNTIME_PATH, app / "electron_bridge.dll"]}
    probe = module()
    native = probe.Bridge(app)
    process = None
    try:
        expected = reference(native)
        default_port, port = verification_ports(artifact)
        env = {**os.environ, "PYTHONUTF8": "1", "PYTHONIOENCODING": "utf-8", "PYTHONDONTWRITEBYTECODE": "1"}
        snapshot = ROOT/'frozen-source.json'
        if snapshot.is_file():
            env['NODE_PATH'] = json.loads(snapshot.read_text(encoding='utf-8'))['node_modules_reference']
        process = subprocess.Popen([sys.executable, str(bridge_script), "--app-dir", str(app), "--port", str(port)],
            cwd=workspace, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            assert process.poll() is None, "generated bridge exited during startup"
            with socket.socket() as sock:
                if sock.connect_ex(("127.0.0.1", port)) == 0:
                    break
            time.sleep(0.1)
        else:
            raise AssertionError("generated bridge did not listen")
        runtime = subprocess.run([args.node, str(RUNTIME_PATH)], cwd=workspace, env=env,
            input=json.dumps({"rule_path": str(artifact), "target": str(app), "bridge_port": port, "reference": expected}),
            capture_output=True, encoding="utf-8", timeout=100, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        # Runtime logs can contain signed media links; never persist or print them.
        lines = runtime.stdout.strip().splitlines()
        assert lines, f'production runtime returned no structured result (exit {runtime.returncode}); check Node dependencies and the frozen NODE_PATH reference'
        result = json.loads(lines[-1])
        assert runtime.returncode == 0 and result.get("passed") is True, (
            result.get("error", "production runtime failed") +
            f'; configured_bridge_port={port}, rule_default_port={default_port}')
        variant = playback_variant(result["playback"])
        media = probe.media_probe(variant["url"], variant["headers"])
        assert media["bytes_read"] > 0, "empty media bytes"
        decoded, decoder_attempts = verify_decode_progress(probe, native, variant)
        assert all(sha(path) == original for path, original in identities.items()), "inputs changed during verification"
        report = {"passed": True, "target": args.target,
            "checks": result["checks"] + ["bridgeBase_overrides_declared_default_port", "actual_media_bytes",
                "null_output_decode_over_one_second", "inputs_unchanged"],
            "observed": result["observed"], "decoder": decoded, "decoder_attempts": decoder_attempts,
            "mobile_standalone_verified": False,
            "declared_default_port": default_port, "bridge_port": port,
            "target_dll_sha256": identities[str(app / "electron_bridge.dll")], "no_desktop": True}
        print(json.dumps(report, ensure_ascii=False))
    finally:
        native.close()
        if process is not None:
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True)
            else:
                process.terminate()
            process.wait(timeout=10)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    try:
        main()
    except Exception as exc:
        # Only bounded domain errors; never echo signed request/response bodies.
        message = str(exc)
        message = re.sub(r"https?://\S+", "[URL]", message)
        failure = {"passed": False, "checks": [], "error": message[:400]}
        if isinstance(exc, DecodeProgressError):
            failure['decoder_attempts'] = exc.decoder_attempts
        print(json.dumps(failure, ensure_ascii=False))
        raise SystemExit(1)
