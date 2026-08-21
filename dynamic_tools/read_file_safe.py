import sys
import json
import os

def main():
    try:
        args = json.loads(sys.argv[1])
        path = args['path']
        start_line = int(args.get('start_line', 1)) - 1
        end_line = args.get('end_line', None)

        if not os.path.exists(path):
            print(json.dumps({"success": False, "error": f"File not found: {path}"}))
            return

        with open(path, 'r', encoding='utf-8') as f:
            lines = f.readlines()

        if end_line is not None:
            end_line = int(end_line)
        else:
            end_line = len(lines)
        
        start_line = max(0, start_line)
        end_line = min(end_line, len(lines))
        
        selected_lines = lines[start_line:end_line]
        content = "".join(selected_lines)
        
        print(json.dumps({
            "success": True,
            "total_lines": len(lines),
            "returned_lines": len(selected_lines),
            "content": content
        }))
    except Exception as e:
        print(json.dumps({"success": False, "error": str(e)}))

if __name__ == "__main__":
    main()
