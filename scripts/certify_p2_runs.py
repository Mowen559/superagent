"""核对P2三个独立Run的完整分母、冻结身份及冷目录结果。"""
import argparse
import hashlib
import json
from pathlib import Path
import re


def read(path):
    return json.loads(Path(path).read_text(encoding='utf-8-sig'))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def certify(frozen_path, reports, packages):
    frozen = read(frozen_path)
    for key in ['profile', 'inputs', 'requests']:
        assert sha(frozen[key]) == frozen[key+'_sha256'], key+' manifest changed'
    inputs = read(frozen['inputs'])
    assert all(sha(f['path']) == f['sha256'] for f in inputs['files']), 'frozen dependency changed'
    expected = read(frozen['requests'])
    assert len(expected) == 3, 'required denominator is three'
    definitions = {d['id']: d for d in expected}
    attempts = [(read(report), a) for report in reports for a in read(report)['attempts']]
    assert len(attempts) == 3 and {a['id'] for _,a in attempts} == set(definitions), 'missing, duplicate or extra Run'
    assert len({a['task_id'] for _,a in attempts}) == 3, 'Run reused'
    cold = [read(p) for p in packages]
    assert len(cold) == 3 and len({p['source'] for p in cold}) == 3, 'required three distinct cold package receipts'
    binary = next(f for f in inputs['files'] if f['path'].replace('\\','/').endswith('/target/debug/hub.exe'))['sha256']
    sessions = []
    data_roots = set()
    settings_hashes = set()
    results = []
    for report, attempt in attempts:
        definition = definitions[attempt['id']]
        assert report['request_file_sha256'] == frozen['requests_sha256'], 'task definitions changed'
        assert report['runtime']['verified_binary_sha256'] == binary, 'wrong runtime binary'
        assert report['runtime']['profile'] == frozen['profile'], 'wrong runtime profile'
        assert report['runtime']['model_turn_budget'] == frozen['budget']['model_turns'], 'model budget mismatch'
        assert attempt['request']['timeoutSeconds'] == frozen['budget']['timeout_seconds'], 'time budget mismatch'
        assert report['runtime']['evolution_initial_exists'] is False, 'independent Run inherited prior evolution state'
        data_roots.add(report['runtime']['app_data_root'])
        settings_hashes.add(report['runtime']['settings_sha256'])
        assert definition['request'] == attempt['request'], 'request modified'
        assert not attempt['request'].get('resumeSessionId'), 'independent Run resumed prior Session'
        assert attempt['classification'] == 'first_attempt' and attempt['passed'] is True, 'failed or coached Run'
        assert attempt['status'] == 'completed' and attempt['oracle_passed'] is True, 'missing completed domain result'
        assert not list(Path(attempt['project']).iterdir()), 'original empty project modified'
        snapshots = [e for e in attempt['events'] if e['kind']=='trace-snapshot']
        sessions.append(snapshots[0]['session_id'])
        snapshot_path = (Path(attempt['execution_workspace'])/snapshots[0]['snapshot_file']).resolve(strict=True)
        assert snapshot_path.is_relative_to(Path(attempt['execution_workspace']).resolve()), 'trace outside workspace'
        actual_budgets = {int(total) for _,total in re.findall(
            r'Remaining model turns including this request: (\d+) of (\d+)',snapshot_path.read_text(encoding='utf-8'))}
        assert actual_budgets == {frozen['budget']['model_turns']}, 'worker effective budget differs from declared budget'
        # Fresh Runs must not copy previous Runs' implementation files.
        other_runs = {run for e in attempt['events'] if e['kind']=='tool-use'
            for run in re.findall(r'runs[\\/]+([0-9a-f-]{36})',str(e.get('input','')),re.I)
            if run != attempt['task_id']}
        assert not other_runs, 'another Run implementation referenced: '+str(other_runs)
        package = next(p for p in cold if p['source']==attempt['execution_workspace'])
        assert package['passed'] is True and package['domain_oracle']['passed'] is True, 'cold package failed'
        hashes = {Path(a['path']).name:a['sha256'] for a in attempt['artifacts']}
        assert hashes == {a['name']:a['sha256'] for a in package['artifacts']}, 'cold package is a different artifact'
        assert all(sha(Path(attempt['execution_workspace'])/name)==value for name,value in hashes.items()), 'artifact changed'
        assert 'incomplete_request_server_timeout_under_eight_seconds' in package['checks'], 'missing server timeout evidence'
        required = {'category:20','category:21','search:仙逆','search:斗破','lazy_production_runtime',
            'detail_title_and_first_middle_last_episodes','actual_media_bytes','null_output_decode_over_one_second',
            'bridgeBase_overrides_declared_default_port'}
        for domain in [attempt['oracle'], package['domain_oracle']]:
            assert required <= set(domain['checks']), 'incomplete business coverage'
            assert domain['bridge_port'] in [18000,18001,18002], 'unapproved bridge port'
            assert domain['bridge_port'] != domain['declared_default_port'], 'dynamic base was not exercised'
            assert float(domain['decoder']['time-pos']) > 1, 'insufficient decoding progress'
            decoder_attempts = domain['decoder_attempts']
            assert 1 <= len(decoder_attempts) <= 3, 'invalid decoder attempt count'
            assert decoder_attempts[-1] == domain['decoder'], 'final decoder evidence mismatch'
            assert all(a.get('decoded_video') is False for a in decoder_attempts[:-1]), 'retry continued after success'
            assert decoder_attempts[-1]['decoded_video'] is True, 'no successful real decode'
        results.append({'id':attempt['id'],'task_id':attempt['task_id'],'workspace':attempt['execution_workspace'],
            'artifacts':hashes,'duration_ms':attempt['duration_ms'],'model_turns':attempt['model_turns'],
            'checks':attempt['oracle']['checks'],'cold_checks':package['checks']})
    assert len(set(sessions)) == 3, 'Session reused across independent Runs'
    assert len(data_roots) == 3, 'evolution or global memory shared across independent Runs'
    assert len(settings_hashes) == 1, 'model settings changed between independent Runs'
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--frozen',required=True)
    parser.add_argument('--reports',nargs='+',required=True)
    parser.add_argument('--packages',nargs='+',required=True)
    parser.add_argument('--reviews',nargs=3,required=True)
    parser.add_argument('--output',required=True)
    args = parser.parse_args()
    result = {'passed':False,'required_runs':3,'known_target_prior_exposure':True,
        'unseen_target_certified':False,'certifier_sha256':sha(__file__),
        'frozen_sha256':sha(args.frozen),'reports':[{'path':p,'sha256':sha(p)} for p in args.reports],
        'packages':[{'path':p,'sha256':sha(p)} for p in args.packages]}
    try:
        manual = [read(p) for p in args.reviews]
        attempts = [a for p in args.reports for a in read(p)['attempts']]
        assert len(manual) == len(attempts) == 3, 'missing manual reviews'
        for attempt in attempts:
            review = next(r for r in manual if r['task_id'] == attempt['task_id'])
            assert review['passed'] is True, 'manual delivery review failed'
            assert review['artifacts'] == {Path(a['path']).name:a['sha256'] for a in attempt['artifacts']}, 'review artifact mismatch'
        result['manual_reviews'] = [{'path':p,'sha256':sha(p)} for p in args.reviews]
        result['runs']=certify(args.frozen,args.reports,args.packages)
        result['passed']=True
    except Exception as exc:
        result['error']=type(exc).__name__+': '+str(exc)
    with Path(args.output).open('x',encoding='utf-8') as output:
        json.dump(result,output,ensure_ascii=False,indent=2)
    print(json.dumps({'passed':result['passed'],'output':args.output,'error':result.get('error')}))
    return 0 if result['passed'] else 1


if __name__=='__main__':
    raise SystemExit(main())
