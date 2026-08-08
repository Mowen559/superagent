import subprocess
import time
import sys
import os

# Set environment variable to make dialoguer think it's not a real terminal, 
# or just force standard input
env = os.environ.copy()
env['TERM'] = 'dumb'

# Start the process
p = subprocess.Popen(
    ['cargo', 'run', '--bin', 'hub-rs', 'config', 'auth'],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    env=env,
    bufsize=1
)

# Function to read output until a prompt appears
def read_until(process, keyword, timeout=10):
    start = time.time()
    output = ""
    while time.time() - start < timeout:
        # read 1 char at a time to avoid blocking
        char = process.stdout.read(1)
        if char:
            output += char
            sys.stdout.write(char)
            sys.stdout.flush()
            if keyword in output:
                return output
    print(f"\n[Timeout waiting for '{keyword}']")
    print(f"Current output: {output}")
    return output

print("Starting test...")

try:
    # We want to select "OpenAI (GPT)" -> "API Key (Manual Entry)"
    
    # 1. Wait for "Select Provider"
    out = read_until(p, "Select Provider")
    print("\n[Sending Enter for OpenAI]")
    # For dialoguer, if we just send '\n', it might select the default (0 - OpenAI)
    p.stdin.write("\n")
    p.stdin.flush()
    
    # 2. Wait for "Select Authentication Method"
    out = read_until(p, "Select Authentication Method")
    print("\n[Sending Down Arrow + Enter for API Key]")
    # Send Down Arrow (ESC [ B) then Enter, or just 'j' + Enter if vim keys work, 
    # but in dumb term, maybe it falls back to typing the number or just arrow keys
    # Let's send arrow down (depending on terminal)
    p.stdin.write("\x1b[B\n") 
    p.stdin.flush()
    
    # 3. Wait for "Account Name"
    out = read_until(p, "Account Name")
    print("\n[Sending Account Name]")
    p.stdin.write("Test OpenAI Key\n")
    p.stdin.flush()

    # 4. Wait for "Base URL"
    out = read_until(p, "Base URL")
    print("\n[Sending Base URL]")
    p.stdin.write("\n") # Accept default
    p.stdin.flush()
    
    # 5. Wait for "API Key"
    out = read_until(p, "API Key")
    print("\n[Sending API Key]")
    p.stdin.write("sk-test-123\n")
    p.stdin.flush()
    
    # Check if it finishes
    stdout, stderr = p.communicate(timeout=5)
    print("\n[Remaining Output]")
    print(stdout)
    if stderr:
        print("\n[Stderr]")
        print(stderr)
        
    print("\n[Test finished successfully]")

except Exception as e:
    print(f"Error during test: {e}")
    p.kill()
