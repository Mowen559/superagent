"""Negative media checks use real HTTP bytes and the installed null-output decoder."""
import http.server
import json
import threading
import tempfile
import types
import unittest
from unittest.mock import patch
from pathlib import Path
from urllib.error import HTTPError
from dmghg_delivery_oracle import module, playback_variant, verify_decode_progress, DecodeProgressError, verification_ports


class MediaFailureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(410 if self.path == "/expired" else 200)
                self.send_header("Content-Type", "application/octet-stream")
                self.end_headers()
                if self.path == "/invalid":
                    self.wfile.write(b"not a playable media stream")
            def log_message(self, *_):
                pass
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = f"http://127.0.0.1:{cls.server.server_port}"
        cls.probe = module()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown(); cls.server.server_close(); cls.thread.join()

    def test_empty_and_expired_media_do_not_look_like_success(self):
        self.assertEqual(self.probe.media_probe(self.base + "/empty")["bytes_read"], 0)
        with self.assertRaises(HTTPError):
            self.probe.media_probe(self.base + "/expired")
        for value in [{}, [], "", "toast://failed"]:
            with self.assertRaises((AssertionError, ValueError)):
                playback_variant(value)

    def test_warning_classification_does_not_treat_url_tokens_as_errors(self):
        classify = self.probe.classify_decoder_warning
        self.assertEqual(classify('Failed to open https://tls.invalid/video?token=403-timeout'), {'open_failed'})
        self.assertEqual(classify('HTTP error 403 Forbidden https://example.invalid/private'), {'http_403'})
        self.assertEqual(classify('TLS handshake failed; connection timed out'), {'tls_error', 'network_timeout'})

    def test_retry_keeps_same_media_and_preserves_failed_attempt(self):
        variant = {'url': 'https://example.invalid/media', 'headers': {}}
        results = [{'decoded_video': False, 'time-pos': '0.3'},
                   {'decoded_video': True, 'time-pos': '1.082', 'video-params/w': '1920'}]
        calls = []
        def decode(native, actual_variant, **options):
            self.assertIs(actual_variant, variant)
            self.assertEqual(options, {'progress_seconds': 1.0, 'diagnostics': True})
            calls.append(actual_variant)
            return results[len(calls)-1]
        decoded, attempts = verify_decode_progress(types.SimpleNamespace(decode_probe=decode), object(), variant)
        self.assertEqual(attempts, results)
        self.assertIs(decoded, results[-1])
        self.assertEqual(len(calls), 2)

    def test_subsecond_attempts_cannot_be_accumulated_into_success(self):
        probe = types.SimpleNamespace(decode_probe=lambda *_args, **_options:
            {'decoded_video': False, 'time-pos': '0.9', 'video-params/w': '1920'})
        with self.assertRaises(DecodeProgressError) as failure:
            verify_decode_progress(probe, object(), {'url': 'https://example.invalid/media', 'headers': {}})
        self.assertEqual(len(failure.exception.decoder_attempts), 3)

    def test_real_port_selection_uses_an_approved_alternate(self):
        with tempfile.TemporaryDirectory() as folder:
            artifact = Path(folder)/'rule.json'
            artifact.write_text(json.dumps({'url': 'http://127.0.0.1:18002/list?page=fypage'}), encoding='utf-8')
            default, actual = verification_ports(artifact)
            self.assertEqual(default, 18002)
            self.assertIn(actual, [18000, 18001])

    def test_only_available_default_port_cannot_satisfy_override_check(self):
        with tempfile.TemporaryDirectory() as folder:
            artifact = Path(folder)/'rule.json'
            artifact.write_text(json.dumps({'url': 'http://127.0.0.1:18002/list'}), encoding='utf-8')
            attempted = []
            def bind(address):
                attempted.append(address[1])
                if address[1] != 18002:
                    raise OSError('occupied')
            with patch('dmghg_delivery_oracle.socket.socket') as factory:
                factory.return_value.__enter__.return_value.bind.side_effect = bind
                with self.assertRaisesRegex(AssertionError, 'alternate'):
                    verification_ports(artifact)
            self.assertEqual(attempted, [18001, 18000])

    @unittest.skipUnless(Path(r"D:\太极\动漫共和国概念版\libmpv-2.dll").is_file(), "requires the explicitly selected installed decoder")
    def test_http_200_garbage_is_rejected_by_actual_decoder(self):
        media = self.probe.media_probe(self.base + "/invalid")
        self.assertEqual(media["status"], 200)
        self.assertGreater(media["bytes_read"], 0)
        result = self.probe.decode_probe(types.SimpleNamespace(app_dir=Path(r"D:\太极\动漫共和国概念版")),
            {"url": self.base + "/invalid", "headers": {}}, progress_seconds=1.0, deadline_seconds=1.5, diagnostics=True)
        self.assertFalse(result["decoded_video"])
        self.assertEqual(result["outputs"], {"audio": "null", "video": "null"})
        self.assertLess(result['end_file_error'], 0)
        self.assertNotIn(self.base, json.dumps(result))
        def decode_bounded(native, variant, **options):
            return self.probe.decode_probe(native, variant, deadline_seconds=1.5, **options)
        with self.assertRaises(DecodeProgressError) as failure:
            verify_decode_progress(types.SimpleNamespace(decode_probe=decode_bounded),
                types.SimpleNamespace(app_dir=Path(r"D:\太极\动漫共和国概念版")),
                {'url': self.base + '/invalid', 'headers': {}})
        self.assertEqual(len(failure.exception.decoder_attempts), 3)
        self.assertTrue(all(not attempt['decoded_video'] for attempt in failure.exception.decoder_attempts))


if __name__ == "__main__":
    unittest.main()
