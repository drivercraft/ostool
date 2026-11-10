// Disposable, loopback-only PostgreSQL wire proxy. Hold the BEGIN acknowledgement
// so the regression can cancel a request at exactly the vulnerable await point.
import net from 'node:net';
import readline from 'node:readline';

const upstream = new URL(process.argv[2]);
if (!['127.0.0.1', 'localhost', '[::1]'].includes(upstream.hostname)) {
  throw new Error('test proxy requires loopback PostgreSQL');
}
let armed = false;
let held = null;
readline.createInterface({ input: process.stdin }).on('line', line => {
  if (line === 'arm') {
    armed = true;
    console.log('armed');
  } else if (line === 'resume') {
    held?.();
    held = null;
  }
});
const server = net.createServer(client => {
  const backend = net.connect(Number(upstream.port || 5432), upstream.hostname);
  let first = true;
  let frontend = Buffer.alloc(0);
  let pending = Buffer.alloc(0);
  let holdBegin = false;
  let paused = false;
  let buffered = [];
  client.on('data', chunk => {
    backend.write(chunk);
    frontend = Buffer.concat([frontend, chunk]);
    while (frontend.length >= (first ? 4 : 5)) {
      const size = frontend.readInt32BE(first ? 0 : 1) + (first ? 0 : 1);
      if (frontend.length < size) break;
      const packet = frontend.subarray(0, size);
      if (!first && packet[0] === 81 && packet.subarray(5).toString() === 'BEGIN\0' && armed) {
        armed = false;
        holdBegin = true;
      }
      first = false;
      frontend = frontend.subarray(size);
    }
  });
  backend.on('data', chunk => {
    pending = Buffer.concat([pending, chunk]);
    while (pending.length >= 5) {
      const size = pending.readInt32BE(1) + 1;
      if (pending.length < size) break;
      const packet = pending.subarray(0, size);
      pending = pending.subarray(size);
      if (holdBegin && packet[0] === 90 && packet[5] === 84) {
        holdBegin = false;
        paused = true;
        held = () => {
          paused = false;
          for (const buffer of buffered) client.write(buffer);
          buffered = [];
        };
        console.log('ready-held');
      }
      if (paused) buffered.push(packet);
      else client.write(packet);
    }
  });
  client.on('close', () => backend.destroy());
  backend.on('close', () => client.destroy());
  client.on('error', () => backend.destroy());
  backend.on('error', () => client.destroy());
});
server.listen(0, '127.0.0.1', () => console.log(server.address().port));
