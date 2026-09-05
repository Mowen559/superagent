import sys
import json
import os

CONTEXT_FILE = ".agent_context.json"

def load_context():
    if os.path.exists(CONTEXT_FILE):
        with open(CONTEXT_FILE, "r", encoding="utf-8") as f:
            return json.load(f)
    return {"files": []}

def save_context(ctx):
    with open(CONTEXT_FILE, "w", encoding="utf-8") as f:
        json.dump(ctx, f, indent=2)

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "Missing action argument"}))
        sys.exit(1)
    
    action = sys.argv[1]
    ctx = load_context()
    
    if action == "add":
        if len(sys.argv) < 3:
            print(json.dumps({"error": "Missing file path"}))
            sys.exit(1)
        path = os.path.abspath(sys.argv[2])
        if not os.path.exists(path):
            print(json.dumps({"error": f"File not found: {path}"}))
            sys.exit(1)
        if path not in ctx["files"]:
            ctx["files"].append(path)
            save_context(ctx)
        print(json.dumps({"status": "success", "message": f"Added {path} to context"}))
        
    elif action == "remove":
        if len(sys.argv) < 3:
            print(json.dumps({"error": "Missing file path"}))
            sys.exit(1)
        path = os.path.abspath(sys.argv[2])
        if path in ctx["files"]:
            ctx["files"].remove(path)
            save_context(ctx)
            print(json.dumps({"status": "success", "message": f"Removed {path} from context"}))
        else:
            print(json.dumps({"error": f"File not in context: {path}"}))
            
    elif action == "list":
        print(json.dumps({"status": "success", "files": ctx["files"]}))
        
    elif action == "read":
        result = {}
        for path in ctx["files"]:
            if os.path.exists(path):
                with open(path, "r", encoding="utf-8") as f:
                    content = f.read()
                    # Simple truncation for safety
                    if len(content) > 10000:
                        content = content[:10000] + "\n...[TRUNCATED]..."
                    result[path] = content
            else:
                result[path] = "[FILE NOT FOUND]"
        print(json.dumps({"status": "success", "context": result}))
        
    else:
        print(json.dumps({"error": f"Unknown action: {action}"}))

if __name__ == "__main__":
    main()
