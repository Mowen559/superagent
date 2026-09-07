import subprocess
import json
import sys
import time

sys.stdout.reconfigure(encoding='utf-8')

def main():
    cmd = ["pnpm.cmd", "--dir", r"D:\DeepSeek\deepseek-harness", "run", "demo:acp"]
    print("Spawning process...", cmd)
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1
    )

    def send(msg):
        line = json.dumps(msg) + "\n"
        print("-->", line.strip())
        proc.stdin.write(line)
        proc.stdin.flush()

    def recv_json():
        while True:
            line = proc.stdout.readline()
            if not line:
                return None
            line_str = line.strip()
            if not line_str:
                continue
            try:
                data = json.loads(line_str)
                print("<-- (JSON)", data)
                return data
            except json.JSONDecodeError:
                print("<-- (NON-JSON IGNORED)", line_str[:120])

    # Step 1: initialize
    send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1, "clientCapabilities": {}}})
    res1 = recv_json()

    # Step 2: session/new
    send({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": r"D:\桌面\gitai\superagent", "mcpServers": []}})
    res2 = recv_json()

    print("\n=== Result Summary ===")
    print("Init response:", res1)
    print("Session response:", res2)

    proc.terminate()

if __name__ == "__main__":
    main()
