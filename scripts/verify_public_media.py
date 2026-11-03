"""Host-only, windowless media verification. Does not implement a target rule."""
import argparse, importlib.util, json, sys
from pathlib import Path
from types import SimpleNamespace
from urllib.parse import urlsplit

def main():
    ap=argparse.ArgumentParser(); ap.add_argument('--decoder-dir',required=True); args=ap.parse_args()
    variant=json.load(sys.stdin)
    host=urlsplit(variant['url']).hostname or ''
    assert host.endswith('.nasa.gov'),'media must belong to the frozen target'
    source=Path(__file__).resolve().parents[1]/'skills/local-app-inspector/scripts/dmghg_probe.py'
    spec=importlib.util.spec_from_file_location('existing_decoder_probe',source)
    probe=importlib.util.module_from_spec(spec); spec.loader.exec_module(probe)
    media=probe.media_probe(variant['url'],variant.get('headers',{})); assert media['bytes_read']>0,'empty actual media'
    attempts=[]
    for _ in range(2):
        decoded=probe.decode_probe(SimpleNamespace(app_dir=Path(args.decoder_dir)),variant,progress_seconds=1.0,deadline_seconds=16,diagnostics=True)
        attempts.append(decoded)
        if decoded.get('decoded_video'): break
    assert decoded.get('decoded_video'),'video decode did not advance over one second: '+json.dumps(attempts)
    print(json.dumps({'passed':True,'media':media,'decoder_attempts':attempts,'decoder':decoded,'gui_used':False}))

if __name__=='__main__':
    try: main()
    except Exception as e:
        print(json.dumps({'passed':False,'error':str(e)})); sys.exit(1)
