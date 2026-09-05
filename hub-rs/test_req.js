const http = require('http');

const data = JSON.stringify({
  model: 'gpt-5.6-sol',
  messages: [{ role: 'user', content: 'Say hello world' }],
  stream: true
});

const req = http.request('http://127.0.0.1:3003/v1/chat/completions', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'Authorization': 'Bearer dummy'
  }
}, (res) => {
  console.log(`STATUS: ${res.statusCode}`);
  res.on('data', (chunk) => {
    console.log(`BODY: ${chunk}`);
  });
});

req.on('error', (e) => {
  console.error(`problem with request: ${e.message}`);
});

req.write(data);
req.end();

