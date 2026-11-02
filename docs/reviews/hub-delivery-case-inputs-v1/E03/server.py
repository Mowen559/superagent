import http.server,json
n=0
class H(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  global n
  n+=1;self.send_response(503 if self.path=='/always' or n==1 else 200);self.end_headers();self.wfile.write(b'{"value":"NETWORK_RECOVERY_42"}')
 def log_message(self,*a):pass
s=http.server.ThreadingHTTPServer(('127.0.0.1',0),H);print(json.dumps({'port':s.server_port}),flush=True);s.serve_forever()
