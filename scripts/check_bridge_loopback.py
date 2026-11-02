"""在任何真实socket.bind发生前检查交付桥接的监听地址。"""
import argparse
import contextlib
import io
import json
from pathlib import Path
import runpy
import socket
import sys


class BindObserved(BaseException):
    def __init__(self, host):
        self.host = host


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bridge',required=True)
    parser.add_argument('--target',required=True)
    parser.add_argument('--override')
    args = parser.parse_args()
    sys.argv = [args.bridge,'--app-dir',args.target,'--port','18002']
    if args.override is not None:
        sys.argv += ['--host',args.override]
    original = socket.socket.bind
    outcome = {'passed':False,'actual_network_bind':False,'override':args.override}
    # The sentinel is a BaseException so normal server recovery cannot swallow it.
    def observe(sock, address):
        raise BindObserved(address[0] if isinstance(address,tuple) else str(address))
    socket.socket.bind = observe
    captured = io.StringIO()
    try:
        with contextlib.redirect_stdout(captured),contextlib.redirect_stderr(captured):
            runpy.run_path(str(Path(args.bridge).resolve()),run_name='__main__')
    except BindObserved as observed:
        outcome['attempted_host'] = observed.host
        outcome['passed'] = observed.host == '127.0.0.1'
        outcome['reason'] = 'loopback binding observed' if outcome['passed'] else 'non-loopback binding attempted'
    except SystemExit as exc:
        # A default-bind positive is mandatory in the parent verifier. With
        # that baseline, explicit nonzero CLI rejection is valid regardless
        # of whether the program uses argparse's 2 or its own exit status.
        outcome['passed'] = args.override is not None and isinstance(exc.code,int) and exc.code != 0
        outcome['reason'] = 'CLI override rejected before binding' if outcome['passed'] else 'unconfirmed early exit'
    except (ValueError,RuntimeError) as exc:
        message = str(exc).lower()
        outcome['passed'] = args.override is not None and any(t in message for t in ['loopback','127.0.0.1','localhost'])
        outcome['reason'] = 'explicit loopback guard' if outcome['passed'] else 'unconfirmed runtime failure'
    finally:
        socket.socket.bind = original
    print(json.dumps(outcome,ensure_ascii=False))
    return 0 if outcome['passed'] else 1


if __name__=='__main__':
    sys.stdout.reconfigure(encoding='utf-8')
    raise SystemExit(main())
