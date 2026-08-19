import sys
import json

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"status": "error", "message": "No input argument provided"}))
        sys.exit(1)
        
    try:
        args = json.loads(sys.argv[1])
    except Exception as e:
        print(json.dumps({"status": "error", "message": f"Failed to parse JSON argument: {e}"}))
        sys.exit(1)

    name = args.get("name", "")
    response = {
        "status": "success",
        "message": f"Hello {name}, this is a message from the newly created dynamic Python plugin!"
    }
    
    print(json.dumps(response, ensure_ascii=False))

if __name__ == "__main__":
    main()
