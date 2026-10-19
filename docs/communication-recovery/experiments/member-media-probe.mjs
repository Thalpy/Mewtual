// Loopback-only architecture experiment. NOT an application TURN server or authorization model.
// Tests whether relay-only Chromium endpoints accept synthetic relay tuples carried through a
// bounded TCP bridge. No app identities/keys, microphone, external listener or real media relay.
import dgram from 'node:dgram';
import net from 'node:net';
import http from 'node:http';
import { createHash, createHmac, randomBytes, timingSafeEqual } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

// Keep this manually invoked experiment outside the runtime/build/test entry points. The native
// mode inspects the built host's WebView2 implementation, never the application's call pipeline.
// Read MEDIA-RELAY-NEXT.md for limitations before using the result as acceptance evidence.
const ROOT = fileURLToPath(new URL('../../../', import.meta.url));

const sleep = ms => new Promise(r => setTimeout(r, ms));
const MAGIC = 0x2112a442, PORT = 50000, realm = 'local-media-probe';
const fault=process.argv.find(argument=>argument.startsWith('--fault='))?.slice(8)??'';
if(!['','missing-page','child-start','stalled-cdp','silent-sockets','high-water','absolute-deadline'].includes(fault))throw new Error('unknown probe fault');
const duration=fault==='absolute-deadline'?200:80000, started=performance.now(), deadline=started+duration;
let bridgeLive = true, bridgePackets = 0, bridgeBytes = 0, refused = 0;
const bridgePeers = new Map(), gateways = [], children = [];
const servers = [];
const acceptedSockets = new Set(), httpSockets = new Set(), fixtureSockets = new Set();
let ws;
const stop = new AbortController();
const halted = new Promise((_,reject)=>stop.signal.addEventListener('abort',()=>reject(new Error('probe absolute deadline reached')),{once:true}));
// Every external wait observes this same absolute watchdog. Timing out ends the owned lifecycle;
// it cannot leave a later setup continuation able to spawn another server or application.
halted.catch(()=>{});
const watchdog = setTimeout(()=>stop.abort(),duration);
async function bounded(operation,label,ms=5000) {
  if(stop.signal.aborted)throw new Error('probe stopped');
  let timer;const remaining=deadline-performance.now(),absolute=remaining<=ms;
  try { return await Promise.race([operation,halted,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(absolute?'probe absolute deadline reached':label+' timed out')),Math.max(1,Math.min(ms,remaining)));})]); }
  finally {clearTimeout(timer);}
}
const expired = () => performance.now() >= deadline;
const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32BE(n); return b; };
function attr(kind, bytes) {
  const out = Buffer.alloc(4 + ((bytes.length + 3) & ~3));
  out.writeUInt16BE(kind); out.writeUInt16BE(bytes.length, 2); bytes.copy(out, 4); return out;
}
function address(ip, port) {
  const b = Buffer.alloc(8); b[1] = 1; b.writeUInt16BE(port ^ (MAGIC >>> 16), 2);
  const octets = ip.split('.').map(Number); for (let n = 0; n < 4; n++) b[n+4] = octets[n] ^ ((MAGIC >>> ((3-n)*8)) & 255);
  return b;
}
function decodeAddress(b) {
  if (!b || b.length !== 8 || b[1] !== 1) throw new Error('IPv4 only');
  return { ip: Array.from(b.subarray(4), (v,n) => v ^ ((MAGIC >>> ((3-n)*8)) & 255)).join('.'), port: b.readUInt16BE(2) ^ (MAGIC >>> 16) };
}
function message(type, transaction, attrs, key = null) {
  const body = Buffer.concat(attrs), header = Buffer.alloc(20);
  header.writeUInt16BE(type); header.writeUInt16BE(body.length + (key ? 24 : 0), 2);
  header.writeUInt32BE(MAGIC, 4); transaction.copy(header, 8);
  const prefix = Buffer.concat([header,body]);
  return key ? Buffer.concat([prefix,attr(8,createHmac('sha1',key).update(prefix).digest())]) : prefix;
}
function parse(b) {
  if (b.length < 20 || b.length > 4096 || b.readUInt32BE(4) !== MAGIC || b.readUInt16BE(2) + 20 !== b.length) throw new Error('frame');
  const attrs = new Map(); let offset = 20;
  while (offset < b.length) {
    if (attrs.size >= 32 || offset + 4 > b.length) throw new Error('attributes');
    const kind = b.readUInt16BE(offset), length = b.readUInt16BE(offset+2);
    if (offset + 4 + length > b.length || attrs.has(kind)) throw new Error('attribute length/duplicate');
    attrs.set(kind,{ bytes:b.subarray(offset+4,offset+4+length), offset }); offset += 4 + ((length+3)&~3);
  }
  if (offset !== b.length) throw new Error('padding');
  return { type:b.readUInt16BE(0), transaction:b.subarray(8,20), attrs, get:kind => attrs.get(kind)?.bytes };
}
function authenticated(b, p, g) {
  if (p.get(6)?.toString() !== g.username || p.get(0x14)?.toString() !== realm || p.get(0x15)?.toString() !== g.nonce) return false;
  const integrity = p.attrs.get(8); if (!integrity || integrity.bytes.length !== 20) return false;
  const prefix = Buffer.from(b.subarray(0,integrity.offset)); prefix.writeUInt16BE(integrity.offset+24-20,2);
  return timingSafeEqual(integrity.bytes,createHmac('sha1',g.key).update(prefix).digest());
}
function frameData(socket, data) {
  if (data.length > 2048 || socket.destroyed || socket.writableLength > 65536 || expired()) { refused++; return false; }
  const length = Buffer.alloc(2); length.writeUInt16BE(data.length);
  try {socket.write(Buffer.concat([length,data]));return true;}catch{refused++;return false;}
}
function bridgeWrite(socket,data) {
  if(!frameData(socket,data))return false;
  bridgePackets++;bridgeBytes+=data.length;return true;
}
function framed(socket, handler) {
  let pending = Buffer.alloc(0);
  socket.on('data', chunk => {
    if (pending.length + chunk.length > 65536 || expired()) return socket.destroy();
    pending = Buffer.concat([pending,chunk]);
    while (pending.length >= 2) {
      const length = pending.readUInt16BE(0); if (length > 2048) return socket.destroy();
      if (pending.length < length+2) break;
      const data = pending.subarray(2,length+2); pending = pending.subarray(length+2); handler(data);
    }
  });
}
async function experiment() {
const bridge = net.createServer(socket => {
  if(acceptedSockets.size>=2||expired()||stop.signal.aborted){socket.destroy();return;}
  acceptedSockets.add(socket);
  socket.setNoDelay(true); let role = null;
  // Absolute admission deadline: a partial frame dribbled by a loopback client must not renew
  // an unassigned socket indefinitely. Traffic/inactivity timers would not enforce this rule.
  const roleDeadline=setTimeout(()=>{if(role===null)socket.destroy();},2000);
  socket.on('close',()=>{clearTimeout(roleDeadline);acceptedSockets.delete(socket);if(bridgePeers.get(role)===socket)bridgePeers.delete(role);});
  framed(socket,data => {
    if (role === null) {
      if (data.length !== 1 || data[0] > 1 || bridgePeers.has(data[0])) return socket.destroy();
      role = data[0]; clearTimeout(roleDeadline); bridgePeers.set(role,socket); return;
    }
    if (!bridgeLive) return;
    const target = bridgePeers.get(1-role); if (!target) return;
    // Count accepted bounded writes, not attempts refused by the downstream high-water gate.
    // These are local enqueue observations; selected RTC paths and receive-side assertions
    // separately establish endpoint receipt. A write alone is never remote-delivery evidence.
    bridgeWrite(target,data);
  });
  socket.on('error',()=>{});
});
servers.push(bridge);
await bounded(new Promise((resolve,reject)=>{bridge.once('error',reject);bridge.listen(0,'127.0.0.1',resolve);}), 'bridge listen');
if(fault==='absolute-deadline')await bounded(new Promise(()=>{}),'injected global stall',60000);
if(fault==='high-water') {
  let writes=0;
  const socket={destroyed:false,writableLength:65537,write(){writes++;}};
  if(bridgeWrite(socket,Buffer.from([1]))||writes!==0||bridgePackets!==0||bridgeBytes!==0||refused!==1)throw new Error('high-water counter regression');
  console.log('ok high-water refusal does not count accepted writes');return;
}
if(fault==='silent-sockets') {
  for(let index=0;index<3;index++) {
    const socket=net.connect({host:'127.0.0.1',port:bridge.address().port});fixtureSockets.add(socket);
    socket.on('error',()=>{});
    await bounded(new Promise(resolve=>socket.once('connect',resolve)),'silent client connect');
  }
  await bounded(sleep(100),'silent admission');
  if(acceptedSockets.size!==2)throw new Error('bridge socket cap regression');
  // Claim a full-sized frame and keep sending partial bytes. An inactivity timer would keep
  // this first socket alive, whereas the original two-second role deadline must still fire.
  const dribbling=[...fixtureSockets][0];dribbling.write(Buffer.from([8,0]));
  for(let pass=0;pass<9;pass++){await bounded(sleep(250),'partial role frame');if(!dribbling.destroyed)dribbling.write(Buffer.from([0]));}
  if(acceptedSockets.size!==0||bridgePeers.size!==0)throw new Error('silent sockets survived handshake deadline');
  console.log('ok two-socket cap and absolute silent/dribbling handshake retirement');return;
}

async function gateway(role) {
  const socket = dgram.createSocket('udp4'), g = {
    role, socket, ip:`198.18.0.${role+1}`, remote:`198.18.0.${2-role}`,
    username:randomBytes(12).toString('hex'), password:randomBytes(16).toString('hex'), nonce:randomBytes(16).toString('hex'),
    client:null, permission:false, channel:null, packets:0, methods:{}, authAccepted:0, failures:{},
  };
  // Register every acquired owner before its first fallible await, including unconnected sockets.
  gateways.push(g);
  g.key = createHash('md5').update(`${g.username}:${realm}:${g.password}`).digest();
  const stream = net.connect({host:'127.0.0.1',port:bridge.address().port}); stream.setNoDelay(true);
  g.stream=stream;
  await bounded(new Promise((resolve,reject)=>{stream.once('connect',resolve);stream.once('error',reject);}), 'gateway connect'); frameData(stream,Buffer.from([role]));
  framed(stream, data => {
    if (!g.client || !g.permission || expired()) return;
    let out;
    if (g.channel !== null) { out = Buffer.alloc(4+data.length); out.writeUInt16BE(g.channel); out.writeUInt16BE(data.length,2); data.copy(out,4); }
    else out = message(0x0017,randomBytes(12),[attr(0x12,address(g.remote,PORT)),attr(0x13,data)]);
    socket.send(out,g.client.port,g.client.address);
  });
  stream.on('error',()=>{}); g.stream = stream;
  const permitted = peer => peer.ip === g.remote && peer.port === PORT;
  const forward = data => { if (!g.permission || data.length > 2048) { refused++; return; } frameData(stream,data); };
  socket.on('message',(b,client) => {
    if (expired() || ++g.packets > 20000 || client.address !== '127.0.0.1') return;
    const same = g.client?.address === client.address && g.client?.port === client.port;
    try {
      if (b.length >= 4 && (b[0]&0xc0) === 0x40) {
        if (!same || b.readUInt16BE(0) !== g.channel || b.readUInt16BE(2)+4 > b.length) throw new Error('channel');
        forward(b.subarray(4,4+b.readUInt16BE(2))); return;
      }
      const p = parse(b), send = out => socket.send(out,client.port,client.address);
      g.methods[p.type]=(g.methods[p.type]??0)+1;
      if (p.type === 1) { send(message(0x0101,p.transaction,[attr(0x20,address(client.address,client.port))])); return; }
      if (p.type === 0x0016) {
        if (!same || !permitted(decodeAddress(p.get(0x12))) || !p.get(0x13)) throw new Error('send');
        forward(p.get(0x13)); return;
      }
      if (!authenticated(b,p,g)) {
        const error = Buffer.concat([Buffer.from([0,0,4,1]),Buffer.from('Unauthorized')]);
        send(message(p.type|0x0110,p.transaction,[attr(9,error),attr(0x14,Buffer.from(realm)),attr(0x15,Buffer.from(g.nonce))])); return;
      }
      g.authAccepted++;
    if (g.client && !same) {
      // Chromium gathers one allocation per local interface. This experiment admits one
      // tuple only and gives all other authenticated Allocate attempts a terminal refusal.
      const error=Buffer.concat([Buffer.from([0,0,4,86]),Buffer.from('Allocation quota reached')]);
      send(message(p.type|0x0110,p.transaction,[attr(9,error)],g.key)); return;
    }
      if (p.type === 3) {
        if (p.get(0x19)?.[0] !== 17) throw new Error('UDP required');
        g.client ??= client;
        send(message(0x0103,p.transaction,[attr(0x16,address(g.ip,PORT)),attr(0x20,address(client.address,client.port)),attr(0x0d,u32(60))],g.key));
      } else if (p.type === 4 && same) send(message(0x0104,p.transaction,[attr(0x0d,u32(60))],g.key));
      else if ((p.type === 8 || p.type === 9) && same) {
        if (!permitted(decodeAddress(p.get(0x12)))) throw new Error('destination refused');
        g.permission = true;
        if (p.type === 9) { const n = p.get(0x0c)?.readUInt16BE(0); if (!(n>=0x4000 && n<=0x7fff)) throw new Error('number'); g.channel = n; }
        send(message(p.type|0x0100,p.transaction,[],g.key));
      } else throw new Error('method');
    } catch(error) { refused++;g.failures[error.message]=(g.failures[error.message]??0)+1; }
  });
  await bounded(new Promise((resolve,reject)=>{socket.once('error',reject);socket.bind(0,'127.0.0.1',resolve);}), 'gateway bind');
  return { urls:`turn:127.0.0.1:${socket.address().port}?transport=udp`,username:g.username,credential:g.password };
}
const configurations = [await gateway(0),await gateway(1)];
const probePage = await bounded(readFile(new URL(fault==='missing-page'?'./missing-probe-page.txt':'./member-media-probe-page.txt',import.meta.url),'utf8'), 'page read');
const web = http.createServer((request,response) => {
  response.writeHead(200,{'Content-Type':request.url==='/'?'text/html':'application/json','Cache-Control':'no-store'});
  response.end(request.url==='/'?'<html><title>Local media feasibility</title></html>':JSON.stringify({bridgeLive,bridgePackets}));
}); servers.push(web);
web.maxConnections=4;
web.on('connection',socket=>{httpSockets.add(socket);socket.on('close',()=>httpSockets.delete(socket));});
await bounded(new Promise((resolve,reject)=>{web.once('error',reject);web.listen(0,'127.0.0.1',resolve);}), 'page listen');
  const profile = await bounded(mkdtemp(join(tmpdir(),'mewtual-media-probe-')), 'profile creation');
  const native = process.argv.includes('--native');
  const edge = fault==='child-start'?spawn(join(ROOT,'target','missing-media-probe.exe'),[],{windowsHide:true,stdio:'ignore'}):native ? spawn(join(ROOT,'apps/desktop/src-tauri/target/debug/mewtual-desktop.exe'),[],{
    windowsHide:true,stdio:'ignore',env:{...process.env,WEBVIEW2_USER_DATA_FOLDER:profile,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=0 --autoplay-policy=no-user-gesture-required'}
  }) : spawn(process.env.EDGE_PATH ?? 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',[
    '--headless=new','--no-first-run','--no-default-browser-check','--remote-debugging-port=0',
    `--user-data-dir=${profile}`,'--autoplay-policy=no-user-gesture-required','about:blank'
  ],{windowsHide:true,stdio:'ignore'}); children.push(edge);
  await bounded(new Promise((resolve,reject)=>{edge.once('spawn',resolve);edge.once('error',reject);}),'child startup');
  let port;
  for (let n=0;n<90&&!port;n++) { for(const dir of [profile,join(profile,'EBWebView')]) {try { port = Number((await bounded(readFile(join(dir,'DevToolsActivePort'),'utf8'),'debugger file')).split('\n')[0]); } catch {if(stop.signal.aborted)throw new Error('probe stopped');}} if(!port)await bounded(sleep(200),'debugger retry'); }
  if (!port) throw new Error('Edge debugger unavailable');
  const pages = await bounded(fetch(`http://127.0.0.1:${port}/json/list`,{signal:stop.signal}).then(reply=>reply.json()),'debugger targets');
  ws = new WebSocket(pages.find(p=>p.type==='page').webSocketDebuggerUrl);
  await bounded(new Promise((resolve,reject)=>{ws.onopen=resolve;ws.onerror=reject;}),'debugger connect');
  let serial=0; const pending = new Map();
  ws.onmessage = e => {
    const m=JSON.parse(e.data);
    if(m.id) { pending.get(m.id)?.(m);pending.delete(m.id); }
    if(m.method==='Runtime.bindingCalled'&&m.params.name==='memberMediaProbeControl') {
      const request=JSON.parse(m.params.payload);
      // Test-runner control only. These three closed operations never forward media, touch
      // application state, or expose an IPC command. Observe the exact forwarding counter.
      if(!Number.isInteger(request.id)||request.id<1||request.id>8||!['cut','resume','status'].includes(request.operation))return;
      if(request.operation==='cut')bridgeLive=false;
      if(request.operation==='resume')bridgeLive=true;
      const result={bridgeLive,bridgePackets};
      void send('Runtime.evaluate',{contextId:m.params.executionContextId,
        expression:`window.__memberMediaProbeReply(${request.id},${JSON.stringify(result)})`}).catch(()=>{});
    }
  };
  const send = async (method,params,ms=5000) => {
    const id=++serial;
    try{return await bounded(new Promise((resolve,reject)=>{
      pending.set(id,m=>m.error?reject(new Error(m.error.message)):resolve(m.result));
      ws.send(JSON.stringify({id,method,params}));
    }),method,ms);}finally{pending.delete(id);}
  };
  ws.addEventListener('close',()=>{for(const settle of pending.values())settle({error:{message:'owned debugger closed'}});pending.clear();});
  if(fault==='stalled-cdp')await bounded(new Promise(()=>{}),'injected stalled CDP',100);
  if(!native) {
    const navigation=await send('Page.navigate',{url:`http://127.0.0.1:${web.address().port}/`});
    if(navigation.errorText)throw new Error(navigation.errorText);
    await bounded(sleep(500),'page navigation');
  }
  if(native) {
    // The debugger appears before setup's initial about:blank -> app navigation. Inject only
    // after the actual app document mounts, otherwise that navigation destroys the experiment.
    let ready=false;
    const mountDeadline=Math.min(deadline,performance.now()+20000);
    while(!ready&&performance.now()<mountDeadline) {
      try { const state=await send('Runtime.evaluate',{expression:"location.origin==='http://localhost:1420' && document.readyState==='complete' && !!document.getElementById('app')?.firstElementChild",returnByValue:true});ready=state.result?.value===true; }catch{}
      if(!ready)await bounded(sleep(200),'native document retry');
    }
    if(!ready) {
      // Keep the readiness assertion strict, but distinguish a missing dev server/navigation
      // from a mounted origin whose app failed to initialize. Do not print user/page content.
      const state=await send('Runtime.evaluate',{expression:"JSON.stringify({href:location.href,ready:document.readyState,appChildren:document.getElementById('app')?.childElementCount??0,viteError:!!document.querySelector('vite-error-overlay')})",returnByValue:true});
      console.log('native mount diagnostic '+state.result?.value);
      throw new Error('Native application document did not mount');
    }
  }
  const origin = await send('Runtime.evaluate',{expression:'JSON.stringify({href:location.href,title:document.title})',returnByValue:true});
  console.log('probe page '+origin.result?.value);
  await send('Runtime.addBinding',{name:'memberMediaProbeControl'});
  const expression = probePage.replace('__CONFIGURATIONS__',JSON.stringify(configurations));
  const answer = await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:true},30000);
  if (answer.exceptionDetails) throw new Error(answer.exceptionDetails.exception?.description ?? answer.exceptionDetails.text);
  const result = {...answer.result.value,bridgePackets,bridgeBytes,refused,seconds:(performance.now()-started)/1000};
  await writeFile(join(ROOT,'target',native?'member-media-probe-native-result.json':'member-media-probe-result.json'),JSON.stringify(result,null,2));
  console.log(JSON.stringify(result,null,2));
}
try {
  await experiment();
} finally {
  clearTimeout(watchdog);stop.abort();bridgeLive=false;
  console.log(JSON.stringify({diagnostic:gateways.map(g=>({role:g.role,packets:g.packets,methods:g.methods,authAccepted:g.authAccepted,allocated:!!g.client,permission:g.permission,failures:g.failures})),bridgePackets,refused}));
  ws?.close(); for (const child of children) if (child.pid) spawnSync('taskkill',['/F','/T','/PID',String(child.pid)],{windowsHide:true,stdio:'ignore',timeout:10000});
  for (const g of gateways) { g.stream?.destroy();try{g.socket.close();}catch{} }
  for (const socket of [...acceptedSockets,...httpSockets,...fixtureSockets])socket.destroy();
  let cleanupTimer;
  try {
    await Promise.race([
      Promise.all(servers.map(server=>new Promise(resolve=>{try{server.close(()=>resolve());}catch{resolve();}}))),
      new Promise((_,reject)=>{cleanupTimer=setTimeout(()=>reject(new Error('probe listener cleanup timed out')),2000);}),
    ]);
  } finally {clearTimeout(cleanupTimer);}
  console.log('ok owned listeners and sockets closed');
}
