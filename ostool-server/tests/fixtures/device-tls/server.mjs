// Disposable TLS device used only by integration tests. These PEM files are test fixtures.
import https from 'node:https';
import { readFileSync } from 'node:fs';
const [cert, key, token, mode] = process.argv.slice(2);
const status = {
  protocol_version: 6, boot_epoch: 'secure-epoch', mac_address: '02:00:00:00:11:09',
  current_mac_address: '02:00:00:00:11:09', arch: 'x86_64', loader_version: 'secure-test',
  hardware: {manufacturer:null,product:null,version:null,serial:null}, boot:null, ota:null, serial:null
};
const server = https.createServer({cert:readFileSync(cert),key:readFileSync(key)}, (request, response) => {
  if(request.headers.authorization !== 'Bearer ' + token) { response.writeHead(401).end(); return; }
  if(request.url !== '/api/v1/status') { response.writeHead(404).end(); return; }
  if(mode==='pause') {request.socket.once('close',()=>process.stdout.write('closed\n'));process.stdout.write('authenticated\n');return;}
  if(mode?.startsWith('redirect:')) {response.writeHead(302,{Location:'https://127.0.0.1:'+mode.split(':')[1]+'/forbidden'}).end();return;}
  if(mode==='oversized') {response.writeHead(200,{'Content-Length':'65537'});response.flushHeaders();response.end();return;}
  response.setHeader('Content-Type','application/json');
  response.end(JSON.stringify(status));
});
server.listen(0, '127.0.0.1', () => process.stdout.write(String(server.address().port) + '\n'));
