import sys
import json
import ctypes

def main():
    if len(sys.argv) < 2:
        print("Error: Missing JSON argument")
        sys.exit(1)
        
    try:
        args = json.loads(sys.argv[1])
        title = args.get("title", "SuperAgent Notification")
        message = args.get("message", "")
        
        # MB_OK = 0, MB_ICONINFORMATION = 0x40
        # MB_SYSTEMMODAL = 0x1000 (to make it appear on top)
        # MB_SETFOREGROUND = 0x10000
        # MB_SERVICE_NOTIFICATION = 0x00200000 (forces it onto the interactive desktop)
        ctypes.windll.user32.MessageBoxW(0, message, title, 0x40 | 0x1000 | 0x10000 | 0x00200000)
        
        print(json.dumps({"status": "success", "message": "Notification displayed"}))
    except Exception as e:
        print(f"Error: {str(e)}")
        sys.exit(1)

if __name__ == "__main__":
    main()