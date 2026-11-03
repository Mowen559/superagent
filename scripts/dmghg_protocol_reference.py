"""Development-only stateless native protocol reference. Never a rule runtime bridge."""
import argparse
import hashlib
import importlib.util
import json
import sys
from pathlib import Path

ALLOWED = {'auth.legacy_encrypt_auth_header', 'auth.legacy_encrypt_request_body',
           'auth.legacy_decrypt_response', 'auth.legacy_decrypt_host_payload'}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--app-dir', required=True)
    parser.add_argument('--cases', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--pause-for-analysis', action='store_true')
    args = parser.parse_args()
    out = Path(args.output)
    assert not out.exists(), 'never overwrite evidence'
    cases = json.loads(Path(args.cases).read_text(encoding='utf-8-sig'))
    assert all(case['command'] in ALLOWED for case in cases), 'stateless transforms only'
    probe_path = Path(__file__).resolve().parents[1]/'skills/local-app-inspector/scripts/dmghg_probe.py'
    spec = importlib.util.spec_from_file_location('original_probe', probe_path)
    probe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(probe)
    probe.Bridge.READ_COMMANDS = probe.Bridge.READ_COMMANDS | ALLOWED
    native = probe.Bridge(Path(args.app_dir))
    results = []
    try:
        if args.pause_for_analysis:
            print('REFERENCE_READY', flush=True)
            sys.stdin.readline()
        for case in cases:
            record = dict(case)
            try:
                record['result'] = native.call(case['command'], case.get('payload', {}))
            except Exception as error:
                record['error'] = str(error)
            results.append(record)
    finally:
        native.close()
    out.write_text(json.dumps({'development_only':True, 'rule_runtime_dependency':False,
        'dll_sha256':hashlib.sha256((Path(args.app_dir)/'electron_bridge.dll').read_bytes()).hexdigest(),
        'results':results},ensure_ascii=False,indent=2),encoding='utf-8')
    for record in results:
        print(json.dumps({'command':record['command'], 'error':record.get('error'),
            'result_type':type(record.get('result')).__name__,
            'result_keys':list(record['result']) if isinstance(record.get('result'),dict) else None},ensure_ascii=False))

if __name__ == '__main__':
    main()
