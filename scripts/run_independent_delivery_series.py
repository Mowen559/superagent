"""按冻结条件顺序执行三次独立Hub任务；失败即保留证据并停止本系列。"""
import argparse
import json
import os
from pathlib import Path
import socket
import shutil
import subprocess
import sys
import time


def read(path):
    return json.loads(Path(path).read_text(encoding='utf-8-sig'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--frozen', required=True)
    parser.add_argument('--prefix', required=True)
    parser.add_argument('--first-port', type=int, required=True)
    args = parser.parse_args()
    shell = shutil.which('pwsh') or shutil.which('powershell')
    assert shell, 'PowerShell runtime unavailable'
    frozen = read(args.frozen)
    root = Path(read(frozen['profile'])['validators'][0]['script']).parents[1]
    provenance = read(root/'frozen-source.json')
    os.environ['NODE_PATH'] = provenance['node_modules_reference']
    ui = root.parent/'deepcloud-ui'
    prefix = Path(args.prefix).resolve()
    state_path = prefix.with_name(prefix.name+'-series.json')
    state = {'passed':False,'required_runs':3,'attempts':[], 'state':'running',
             'frozen':str(Path(args.frozen).resolve()),'manual_authorship_review':'pending'}
    with state_path.open('x',encoding='utf-8') as stream:
        json.dump(state,stream,ensure_ascii=False,indent=2)
    def persist():
        state_path.write_text(json.dumps(state,ensure_ascii=False,indent=2),encoding='utf-8')
    try:
        for i in range(3):
            ident = prefix.with_name(prefix.name+f'-{i+1}-service.json')
            report = prefix.with_name(prefix.name+f'-part{i+1}.json')
            package = prefix.with_name(prefix.name+f'-package{i+1}.json')
            port = args.first_port+i*2
            assert not any(p.exists() for p in [ident,report,package]), 'never overwrite earlier attempts'
            subprocess.run([shell,'-NoProfile','-File',str(root/'scripts/start_delivery_eval_service.ps1'),
                '-Profile',frozen['profile'],'-IdentityOutput',str(ident),'-Port',str(port),
                '-ModelTurnBudget',str(frozen['budget']['model_turns'])],check=True,cwd=root)
            identity = read(ident)
            try:
                deadline = time.monotonic()+45
                while True:
                    try:
                        with socket.create_connection(('127.0.0.1',port),timeout=1):
                            break
                    except OSError:
                        assert time.monotonic()<deadline, 'isolated service did not bind'
                        time.sleep(0.5)
                result = subprocess.run(['node',str(ui/'scripts/hub-delivery-replay.cjs'),
                    '--base',f'http://127.0.0.1:{port}','--identity',str(ident),
                    '--requests',frozen['requests'],'--offset',str(i),'--limit','1','--output',str(report)],cwd=ui)
                attempt = read(report)['attempts'][0]
                state['attempts'].append({'report':str(report),'task_id':attempt['task_id'],
                    'passed':attempt['passed'],'status':attempt['status']})
                persist()
                assert result.returncode == 0 and attempt['passed'], 'independent generation failed; no result relabelled'
                subprocess.run([sys.executable,str(root/'scripts/verify_dmghg_package.py'),
                    '--workspace',attempt['execution_workspace'],'--request-file',frozen['requests'],
                    '--output',str(package)],check=True,cwd=root)
                assert read(package)['passed'], 'cold package failed'
                state['attempts'][-1]['package'] = str(package)
                review_path = prefix.with_name(prefix.name+f'-review{i+1}.json')
                state['state'] = 'awaiting_manual_review'
                state['manual_review_path'] = str(review_path)
                persist()
                review_deadline = time.monotonic()+600
                while not review_path.exists():
                    assert time.monotonic() < review_deadline, 'manual review receipt missing'
                    time.sleep(2)
                review = read(review_path)
                assert review['task_id'] == attempt['task_id'], 'review belongs to another Run'
                assert review['artifacts'] == {Path(a['path']).name:a['sha256'] for a in attempt['artifacts']}, 'review artifact mismatch'
                assert review['passed'] is True, 'manual delivery/authorship review failed'
                state['attempts'][-1]['manual_review'] = str(review_path)
                state['state'] = 'running'
                persist()
            finally:
                # Only the exact PID whose executable is this owned test stage.
                quote = lambda value: "'"+str(value).replace("'","''")+"'"
                expected = str(Path(identity['stage'])/'hub.exe')
                subprocess.run([shell,'-NoProfile','-Command',
                    f"$owned = Get-Process -Id {int(identity['pid'])} -ErrorAction SilentlyContinue; "
                    f"if ($owned -and $owned.Path -eq {quote(expected)}) {{ Stop-Process -Id $owned.Id }}"],check=True)
        state['state'] = 'all_three_runs_and_manual_reviews_passed'
        state['manual_authorship_review'] = 'passed'
        state['passed'] = True
        persist()
        return 0
    except Exception as exc:
        state['state'] = 'failed'
        state['error'] = type(exc).__name__+': '+str(exc)
        persist()
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
