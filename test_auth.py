import pexpect
import sys

child = pexpect.spawn('cargo run --bin hub-rs config auth', encoding='utf-8')
child.logfile = sys.stdout

try:
    # Expect Provider Selection
    child.expect('Select Provider')
    print("\n[Found Provider Prompt]")
    
    # We press Enter to select OpenAI (default)
    child.sendline('\r')
    
    # Expect Authentication Method Selection
    child.expect('Select Authentication Method')
    print("\n[Found Auth Method Prompt]")
    
    # We press Down arrow to select API Key
    child.send('\x1b[B')
    child.sendline('\r')
    
    # Expect Account Name
    child.expect('Account Name')
    print("\n[Found Account Name Prompt]")
    child.sendline('My Test Key')
    
    # Expect Base URL
    child.expect('Base URL')
    print("\n[Found Base URL Prompt]")
    child.sendline('\r')
    
    # Expect API Key
    child.expect('API Key')
    print("\n[Found API Key Prompt]")
    child.sendline('sk-1234567890')
    
    # Wait for completion
    child.expect(pexpect.EOF)
    print("\n[Test Completed Successfully]")
except Exception as e:
    print(f"\n[Test Failed]: {e}")
