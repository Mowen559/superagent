const http=require('node:http');module.exports=http.createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({amount:'42.50'}));});
