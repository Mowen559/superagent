"""User-side media check of the URL actually returned by a generated rule."""
import argparse, importlib.util, json, sys
from pathlib import Path
from types import SimpleNamespace
from urllib.parse import urlsplit

observed = {}

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--domain', required=True)
    ap.add_argument('--decoder-dir', required=True)
    args = ap.parse_args()
    variant = json.load(sys.stdin)
    host = (urlsplit(variant['url']).hostname or '').lower()
    domain = args.domain.lower().strip('.')
    assert host == domain or host.endswith('.' + domain), 'playback source differs from requested content'
    source = Path(__file__).resolve().parents[1] / 'skills/local-app-inspector/scripts/dmghg_probe.py'
    spec = importlib.util.spec_from_file_location('existing_media_decoder', source)
    probe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(probe)
    media = probe.media_probe(variant['url'], variant.get('headers', {}))
    observed.update({'media_host': host, 'media': media})
    assert media['bytes_read'] > 0, 'no media bytes'
    attempts = []
    observed['decoder_attempts'] = attempts
    for _ in range(2):
        decoded = probe.decode_probe(SimpleNamespace(app_dir=Path(args.decoder_dir)), variant,
                                     progress_seconds=1.0, deadline_seconds=16, diagnostics=True)
        attempts.append(decoded)
        if decoded.get('decoded_video') and float(decoded.get('time-pos', '0')) > 1:
            break
    assert decoded.get('decoded_video') and float(decoded.get('time-pos', '0')) > 1, 'video did not decode beyond one second'
    print(json.dumps({'passed': True, 'media_host': host, 'media': media, 'decoder': decoded,
                      'decoder_attempts': attempts, 'gui_used': False}))

if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'passed': False, 'error': str(error), 'observed': observed}))
        sys.exit(1)
