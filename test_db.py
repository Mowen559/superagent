import os
import sys

print("Hello from python script", flush=True)
user = os.environ.get("USERPROFILE")
print(f"USERPROFILE: {user}", flush=True)

cache_dir = os.path.join(user, ".cache", "codebase-memory-mcp") if user else ""
print(f"Cache dir: {cache_dir}, Exists: {os.path.exists(cache_dir)}", flush=True)

if os.path.exists(cache_dir):
    print(f"Files: {os.listdir(cache_dir)}", flush=True)
