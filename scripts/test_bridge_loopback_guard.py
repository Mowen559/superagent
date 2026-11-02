"""验证监听负例在绑定前被拦截，不创建网络服务。"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class LoopbackGuardTests(unittest.TestCase):
    def check(self, source, override=None):
        with tempfile.TemporaryDirectory() as root:
            bridge = Path(root)/'bridge.py'
            bridge.write_text(source,encoding='utf-8')
            cmd = [sys.executable,str(Path(__file__).with_name('check_bridge_loopback.py')),
                   '--bridge',str(bridge),'--target',root]
            if override is not None:
                cmd += ['--override',override]
            result = subprocess.run(cmd,capture_output=True,encoding='utf-8',timeout=10)
            return json.loads(result.stdout)

    def test_actual_binding_is_intercepted_for_default_and_public_override(self):
        source = """import argparse,socket
p=argparse.ArgumentParser()
p.add_argument('--app-dir');p.add_argument('--port');p.add_argument('--host',default='127.0.0.1')
a=p.parse_args();socket.socket().bind((a.host,int(a.port)))
"""
        self.assertTrue(self.check(source)['passed'])
        bad = self.check(source,'0.0.0.0')
        self.assertFalse(bad['passed'])
        self.assertFalse(bad['actual_network_bind'])
        self.assertEqual(bad['attempted_host'],'0.0.0.0')

    def test_explicit_cli_guard_passes_without_binding(self):
        source = """import argparse,socket
p=argparse.ArgumentParser()
p.add_argument('--app-dir');p.add_argument('--port');p.add_argument('--host',default='127.0.0.1',choices=['127.0.0.1'])
a=p.parse_args();socket.socket().bind((a.host,int(a.port)))
"""
        for host in ['0.0.0.0','192.0.2.1']:
            self.assertTrue(self.check(source,host)['passed'])

    def test_unrelated_startup_failure_is_not_an_address_guard(self):
        result = self.check("raise RuntimeError('missing application')",'0.0.0.0')
        self.assertFalse(result['passed'])

    def test_explicit_exit_one_is_valid_but_default_early_exit_is_not(self):
        source = """import argparse,socket,sys
p=argparse.ArgumentParser()
p.add_argument('--app-dir');p.add_argument('--port');p.add_argument('--host',default='127.0.0.1')
a=p.parse_args()
if a.host != '127.0.0.1':
    print('Only loopback binding is permitted');sys.exit(1)
socket.socket().bind(('127.0.0.1',int(a.port)))
"""
        self.assertTrue(self.check(source)['passed'])
        self.assertTrue(self.check(source,'0.0.0.0')['passed'])
        self.assertFalse(self.check('raise SystemExit(1)')['passed'])
        self.assertFalse(self.check('raise SystemExit(0)','0.0.0.0')['passed'])


if __name__=='__main__':
    unittest.main()
