"""Read-only supplemental diagnosis using the frozen production oracle.

Never changes artifacts or the frozen oracle and never prints media URLs/headers.
This is diagnostic evidence, not an independent generation or certification.
"""
import argparse
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--frozen', required=True)
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--debug-probe', help='Optional additional diagnostic decoder source; never replaces the frozen check')
    parser.add_argument('--alternate-port', action='store_true', help='Exercise the documented fallback with default port unavailable')
    args = parser.parse_args()
    output = Path(args.output)
    assert not output.exists(), 'never overwrite diagnostic evidence'
    read = lambda p: json.loads(Path(p).read_text(encoding='utf-8-sig'))
    frozen = read(args.frozen)
    definition = read(frozen['requests'])[0]
    command = [part.replace('{workspace}', str(Path(args.workspace).resolve()))
               for part in definition['oracle_command']]
    spec = importlib.util.spec_from_file_location('frozen_delivery_oracle', command[1])
    oracle = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(oracle)
    if args.alternate_port:
        socket_class = oracle.socket.socket
        class AlternatePortSocket(socket_class):
            def bind(self, address):
                if address == ('127.0.0.1', 18002):
                    raise OSError('default port unavailable for read-only alternate-port check')
                return super().bind(address)
        oracle.socket.socket = AlternatePortSocket
    workspace = Path(args.workspace).resolve()
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    hashes = {name: digest(workspace/name) for name in ['bridge.py', 'rule.json', 'README.md']}
    receipt = {'diagnostic_only': True, 'certification': False,
               'oracle_sha256': digest(Path(command[1])), 'artifacts': hashes,
               'observations': []}
    if args.alternate_port:
        receipt['default_port_simulated_unavailable'] = True
    debug_probe = None
    if args.debug_probe:
        debug_path = Path(args.debug_probe).resolve()
        debug_spec = importlib.util.spec_from_file_location('supplemental_debug_probe', debug_path)
        debug_probe = importlib.util.module_from_spec(debug_spec)
        debug_spec.loader.exec_module(debug_probe)
        receipt['additional_debug_probe_sha256'] = digest(debug_path)
    original_module = oracle.module
    original_reference = oracle.reference
    selected = {}

    def inspect_reference(bridge):
        result = original_reference(bridge)
        selected.update(result)
        return result

    oracle.reference = inspect_reference

    def instrumented_module():
        probe = original_module()
        decode = probe.decode_probe
        media = probe.media_probe

        def inspect_decode(*positional, **keywords):
            variant = positional[1]
            parsed = urlsplit(variant['url'])
            receipt['observations'].append({'stage': 'rule_media_shape', 'facts': {
                'scheme': parsed.scheme, 'header_names': sorted(variant.get('headers', {})),
                'has_fragment': bool(parsed.fragment),
                'has_whitespace': any(char.isspace() for char in variant['url'])}})
            result = decode(*positional, **keywords)
            receipt['observations'].append({'stage': 'decoder', 'facts': {
                key: result[key] for key in ['audio-codec-name', 'video-params/w',
                    'video-params/h', 'time-pos', 'decoded_video', 'outputs'] if key in result}})
            if result.get('decoded_video') is not True:
                if debug_probe is not None:
                    try:
                        facts = debug_probe.decode_probe(positional[0], variant,
                            progress_seconds=1.0, diagnostics=True)
                        receipt['observations'].append({'stage': 'same_url_debug_decoder', 'facts': facts})
                    except Exception as exc:
                        receipt['observations'].append({'stage': 'same_url_debug_decoder_failed',
                                                        'error_type': type(exc).__name__})
                try:
                    bridge = positional[0]
                    detail = bridge.call('catalog.get_video_detail', {'id': selected['detail_id']})
                    group = next(part for part in detail['parts'] if part.get('part'))
                    play = bridge.call('video.play', {'id': selected['detail_id'],
                        'part': group['part'][0], 'play': group['play'], 'resolve': True})
                    direct = play['list'][int(play.get('default', 0))]
                    direct_parsed = urlsplit(direct['url'])
                    receipt['observations'].append({'stage': 'native_comparison', 'facts': {
                        'same_url': direct['url'] == variant['url'],
                        'same_origin_and_path': (direct_parsed.scheme, direct_parsed.netloc, direct_parsed.path)
                            == (parsed.scheme, parsed.netloc, parsed.path),
                        'same_headers': direct.get('headers', {}) == variant.get('headers', {})}})
                    direct_facts = decode(bridge, direct, progress_seconds=1.0)
                    receipt['observations'].append({'stage': 'native_decoder', 'facts': {
                        key: direct_facts[key] for key in ['audio-codec-name', 'video-params/w',
                            'video-params/h', 'time-pos', 'decoded_video', 'outputs'] if key in direct_facts}})
                except Exception as exc:
                    # No raw resolver errors: they could include signed URLs.
                    receipt['observations'].append({'stage': 'native_comparison_failed',
                                                    'error_type': type(exc).__name__})
            return result

        def inspect_media(*positional, **keywords):
            result = media(*positional, **keywords)
            receipt['observations'].append({'stage': 'media', 'facts': {
                key: result[key] for key in ['status', 'content_type', 'bytes_read',
                    'hls_manifest'] if key in result}})
            return result

        probe.decode_probe = inspect_decode
        probe.media_probe = inspect_media
        return probe

    oracle.module = instrumented_module
    sys.argv = command[1:]
    try:
        # The oracle's stdout is deliberately not persisted: only selected facts.
        with contextlib.redirect_stdout(io.StringIO()):
            oracle.main()
        receipt['oracle_passed_this_diagnostic'] = True
    except Exception as exc:
        import re
        receipt['oracle_passed_this_diagnostic'] = False
        receipt['error'] = re.sub(r'https?://\S+', '[URL]', str(exc))[:400]
    finally:
        receipt['source_unchanged'] = all(digest(workspace/name) == value for name, value in hashes.items())
        with output.open('x', encoding='utf-8') as stream:
            json.dump(receipt, stream, ensure_ascii=False, indent=2)
    print(json.dumps(receipt, ensure_ascii=False))
    return 0


if __name__ == '__main__':
    sys.stdout.reconfigure(encoding='utf-8')
    raise SystemExit(main())
