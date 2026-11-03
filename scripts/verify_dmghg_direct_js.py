"""Trusted standalone-JS oracle. Native libraries are reference/decoder tools only."""
import argparse, hashlib, importlib.util, json, os, re, subprocess, sys
from pathlib import Path
from types import SimpleNamespace

ROOT=Path(__file__).resolve().parents[1]
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
    ap=argparse.ArgumentParser();ap.add_argument('--workspace',required=True);ap.add_argument('--artifact',required=True)
    args=ap.parse_args();workspace=Path(args.workspace).resolve();artifact=Path(args.artifact).resolve()
    assert artifact.is_relative_to(workspace),'artifact outside workspace'
    before=sha(artifact);raw=artifact.read_text(encoding='utf-8-sig');rule=json.loads(raw)
    assert not re.search(r'127\.0\.0\.1|localhost|bridgeBase|electron_bridge|child_process|ctypes',raw,re.I),'rule still depends on local bridge/native execution'
    assert not re.search(r'(?:D:|C:)[\\/]',raw),'rule embeds an installed/workspace path'
    spec=importlib.util.spec_from_file_location('existing_oracle',ROOT/'scripts/dmghg_delivery_oracle.py')
    oracle=importlib.util.module_from_spec(spec);spec.loader.exec_module(oracle)
    probe=oracle.module();app=Path(r'D:\太极\动漫共和国概念版');native=probe.Bridge(app)
    try: reference=oracle.reference(native)
    finally: native.close()
    runtime=ROOT.parent/'deepcloud-ui/scripts/hub-rule-direct-runtime.cjs'
    result=subprocess.run(['node',str(runtime)],input=json.dumps({'rule_path':str(artifact),'target':'original-dmghg-remote-api','reference':reference}),capture_output=True,text=True,encoding='utf-8',timeout=100,
        env={**os.environ,'NODE_PATH':str(ROOT.parent/'deepcloud-ui/node_modules')},creationflags=subprocess.CREATE_NO_WINDOW)
    lines=result.stdout.strip().splitlines();assert lines,'production runtime returned no structured output'
    data=json.loads(lines[-1]);assert result.returncode==0 and data.get('passed'),data.get('error','production runtime failed')
    variant=oracle.playback_variant(data['playback']);media=probe.media_probe(variant['url'],variant.get('headers',{}))
    assert media['bytes_read']>0,'empty media bytes';attempts=[]
    for _ in range(3):
        decoded=probe.decode_probe(SimpleNamespace(app_dir=app),variant,progress_seconds=1.0,deadline_seconds=16,diagnostics=True)
        attempts.append(decoded)
        if decoded.get('decoded_video'):break
    assert decoded.get('decoded_video') and float(decoded.get('time-pos','0'))>1,'real decode did not advance over one second'
    assert before==sha(artifact),'artifact changed during validation'
    report={'passed':True,'independent_js_runtime':True,'rule_sha256':before,'checks':data['checks']+['no_local_bridge_or_install_paths','actual_media_bytes','null_output_decode_over_one_second'],
        'observed':data['observed'],'decoder':decoded,'decoder_attempts':attempts,'android_verified':False,'no_desktop':True,
        'reference_note':'Native installed target used by host oracle for source identity and decoder only; rule executes JS remote requests in production sandbox'}
    print(json.dumps(report,ensure_ascii=False))
if __name__=='__main__':
    try:main()
    except Exception as e:
        print(json.dumps({'passed':False,'error':str(e)},ensure_ascii=False));sys.exit(1)
