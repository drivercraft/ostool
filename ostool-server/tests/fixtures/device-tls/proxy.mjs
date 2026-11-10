// Authenticated TLS envelope for isolated, in-process device protocol fixtures.
import https from 'node:https';
import http from 'node:http';
import { readFileSync } from 'node:fs';
const [cert,key,token,target]=process.argv.slice(2);
const origin=new URL(target);
if(origin.hostname!=='127.0.0.1')throw new Error('fixture proxy requires loopback');
const server=https.createServer({cert:readFileSync(cert),key:readFileSync(key)},(request,response)=>{
  if(request.headers.authorization!=='Bearer '+token){response.writeHead(401).end();return;}
  const upstream=http.request(new URL(request.url,origin),{method:request.method,headers:request.headers},reply=>{
    response.writeHead(reply.statusCode,reply.headers);reply.pipe(response);
  });
  upstream.on('error',()=>{if(!response.headersSent)response.writeHead(502);response.end();});
  request.on('aborted',()=>upstream.destroy());
  request.pipe(upstream);
});
server.listen(0,'127.0.0.1',()=>process.stdout.write(String(server.address().port)+'\n'));
