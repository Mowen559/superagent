import urllib.request
import json
import subprocess

# Try to get password from cmdkey
output = subprocess.check_output("cmdkey /list", shell=True).decode('gbk', errors='ignore')
# we can't easily extract the password using cmdkey /list alone, it doesn't show passwords.

# Let's just use keyring library
try:
    import keyring
    token = keyring.get_password("openai", "antigravity")
    if not token:
        print("Token not found in keyring")
        exit(1)
        
    req = urllib.request.Request(
        "https://chatgpt.com/backend-api/models",
        headers={
            "Authorization": f"Bearer {token}",
            "User-Agent": "codex-cli",
            "Accept": "application/json"
        }
    )
    
    try:
        with urllib.request.urlopen(req) as response:
            print("Status:", response.status)
            body = response.read().decode('utf-8')
            print("Body:", body[:500])
    except urllib.error.HTTPError as e:
        print("HTTP Error:", e.code)
        body = e.read().decode('utf-8')
        print("Error Body:", body[:500])
except Exception as e:
    print("Error:", e)
