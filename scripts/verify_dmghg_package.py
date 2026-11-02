"""独立冷目录交付复核；只复制 Hub 产物，不修改业务源码。"""
import argparse
import ast
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def request(port, path, body=None):
    data = None if body is None else body
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data,
        headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.status, json.loads(response.read())
    except urllib.error.HTTPError as error:
        return error.code, error.read(65536)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--request-file')
    parser.add_argument('--output')
    parser.add_argument('--artifact')
    parser.add_argument('--target')
    parser.add_argument('--bridge-only', action='store_true')
    parser.add_argument('--health', default='/ping')
    parser.add_argument('--readonly', default='/channels')
    parser.add_argument('--rpc', default='/call')
    args = parser.parse_args()
    source = Path(args.workspace).resolve(strict=True)
    root = Path(tempfile.mkdtemp(prefix='hub-package-cold-'))
    cold = root / '新目录交付'
    cold.mkdir()
    receipt = {'passed': False, 'source': str(source), 'cold_directory': str(cold),
        'target': args.target, 'validator_sha256': digest(__file__), 'checks': []}
    process = None
    partial = None
    try:
        receipt['artifacts'] = []
        for name in ['bridge.py', 'rule.json', 'README.md']:
            shutil.copyfile(source/name, cold/name)
            receipt['artifacts'].append({'name': name, 'sha256': digest(cold/name)})
        # AST inspection is dependency evidence, never a replacement for real calls.
        tree = ast.parse((cold/'bridge.py').read_text(encoding='utf-8-sig'))
        imports = sorted({n.names[0].name.split('.')[0] if isinstance(n, ast.Import)
            else (n.module or '').split('.')[0] for n in ast.walk(tree)
            if isinstance(n, (ast.Import, ast.ImportFrom))})
        receipt['imports'] = imports
        receipt['undeclared_nonstdlib_imports'] = [m for m in imports if m and m not in sys.stdlib_module_names]
        assert not receipt['undeclared_nonstdlib_imports'], 'undeclared Python dependencies'
        env = {**os.environ, 'PYTHONUTF8': '1', 'PYTHONIOENCODING': 'utf-8', 'PYTHONDONTWRITEBYTECODE': '1'}
        kwargs = {'cwd': cold, 'env': env, 'creationflags': getattr(subprocess, 'CREATE_NO_WINDOW', 0)}
        target = args.target
        if not args.bridge_only:
            assert args.request_file, '--request-file required for full cold verification'
            definition = json.loads(Path(args.request_file).read_text(encoding='utf-8-sig'))[0]
            # The pinned argv target, never the natural prompt, is the authority.
            command = [v.replace('{workspace}', str(cold)) for v in definition['oracle_command']]
            target = command[command.index('--target') + 1]
            receipt['target'] = target
            completed = subprocess.run(command, capture_output=True, encoding='utf-8', timeout=150, **kwargs)
            oracle = json.loads(completed.stdout.strip().splitlines()[-1])
            receipt['domain_oracle'] = oracle
            assert completed.returncode == 0 and oracle.get('passed') is True, 'cold directory domain verification failed'
            receipt['checks'].append('only_three_delivered_files_cold_runtime_and_decode')
        assert target, '--target required for bridge-only verification'
        guard = Path(__file__).with_name('check_bridge_loopback.py')
        host_checks = []
        for override in [None,'0.0.0.0','192.0.2.1']:
            command = [sys.executable,str(guard),'--bridge',str(cold/'bridge.py'),'--target',target]
            if override is not None:
                command += ['--override',override]
            observed = subprocess.run(command,capture_output=True,encoding='utf-8',timeout=15,**kwargs)
            result = json.loads(observed.stdout.strip().splitlines()[-1])
            host_checks.append(result)
            assert observed.returncode == 0 and result['passed'], 'loopback binding contract failed: '+result.get('reason','unknown')
        receipt['host_checks'] = host_checks
        receipt['checks'].append('default_and_cli_overrides_cannot_bind_non_loopback')
        if args.artifact:
            assert Path(args.artifact).resolve() == source/'bridge.py', 'bridge artifact identity mismatch'
        for candidate in [18002, 18001, 18000]:
            try:
                with socket.socket() as sock:
                    sock.bind(('127.0.0.1', candidate))
                port = candidate
                break
            except OSError:
                continue
        else:
            raise AssertionError('no approved free port')
        process = subprocess.Popen([sys.executable, str(cold/'bridge.py'), '--app-dir', target, '--port', str(port)],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, **kwargs)
        deadline = time.monotonic()+15
        while time.monotonic()<deadline:
            assert process.poll() is None, 'bridge startup failed'
            try:
                if request(port, args.health)[0] == 200:
                    break
            except (OSError, ValueError):
                time.sleep(.1)
        else:
            raise AssertionError('bridge startup deadline')
        # A partial client must not prevent another client receiving a response.
        partial = socket.create_connection(('127.0.0.1', port), timeout=3)
        timeout_started = time.monotonic()
        partial.sendall(f'POST {args.rpc} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 100\r\n\r\n{{'.encode())
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            responses = list(pool.map(lambda _: request(port, args.readonly), range(4)))
        assert all(code == 200 and data for code, data in responses), 'concurrent read requests failed'
        receipt['checks'].append('four_concurrent_reads_with_partial_client')
        # A real incomplete request must eventually be closed by the server.
        # The client's own timeout is a failing observation, not a passing check.
        remaining = 8-(time.monotonic()-timeout_started)
        assert remaining > 0, 'partial HTTP request deadline exhausted during concurrent reads'
        partial.settimeout(remaining)
        try:
            partial.recv(4096)
        except TimeoutError as exc:
            raise AssertionError('partial HTTP request has no server-side timeout within 8 seconds') from exc
        receipt['partial_request_closed_ms'] = round((time.monotonic()-timeout_started)*1000)
        assert time.monotonic()-timeout_started < 8, 'partial HTTP request closed after the total 8-second deadline'
        receipt['checks'].append('incomplete_request_server_timeout_under_eight_seconds')
        partial.close()
        partial = None
        assert request(port, '/__invalid_route__')[0] >= 400, 'unknown route not rejected'
        assert request(port, args.rpc, b'{invalid-json')[0] >= 400, 'invalid JSON not rejected'
        assert request(port, args.rpc, b'{"command":"__invalid_command__","payload":{}}')[0] >= 400, 'unknown RPC not rejected'
        assert request(port, args.rpc, b'{"command":"account.login","payload":{}}')[0] >= 400, 'account command not rejected'
        receipt['checks'].append('unknown_route_bad_json_unknown_rpc_account_rpc_rejected')
        stopped = time.monotonic()
        subprocess.run(['taskkill','/PID',str(process.pid),'/T','/F'], capture_output=True, timeout=10)
        process.wait(timeout=5)
        receipt['shutdown_ms'] = round((time.monotonic()-stopped)*1000)
        with socket.socket() as sock:
            assert sock.connect_ex(('127.0.0.1', port)) != 0, 'bridge port survived shutdown'
        receipt['checks'].append('process_tree_shutdown_releases_port')
        process = None
        for app_dir in [str(root/'不存在的应用')]:
            failed = subprocess.run([sys.executable, str(cold/'bridge.py'), '--app-dir', app_dir, '--port', str(port)],
                capture_output=True, timeout=15, **kwargs)
            assert failed.returncode != 0, 'missing install falsely accepted'
        receipt['checks'].append('missing_install_fails_without_gui')
        assert all(digest(source/a['name']) == a['sha256'] == digest(cold/a['name']) for a in receipt['artifacts']), 'artifact changed during package audit'
        receipt['checks'].append('source_and_copy_hashes_unchanged')
        receipt['passed'] = True
    except Exception as exc:
        receipt['error'] = type(exc).__name__ + ': ' + str(exc)[:200]
    finally:
        if partial is not None:
            partial.close()
        if process is not None:
            subprocess.run(['taskkill','/PID',str(process.pid),'/T','/F'], capture_output=True, timeout=10)
        if args.output:
            with Path(args.output).open('x', encoding='utf-8') as output:
                json.dump(receipt, output, ensure_ascii=False, indent=2)
            print(json.dumps({'passed': receipt['passed'], 'output': args.output, 'checks': receipt['checks']}, ensure_ascii=False))
        else:
            print(json.dumps(receipt, ensure_ascii=False))
    return 0 if receipt['passed'] else 1


if __name__ == '__main__':
    sys.stdout.reconfigure(encoding='utf-8')
    raise SystemExit(main())
