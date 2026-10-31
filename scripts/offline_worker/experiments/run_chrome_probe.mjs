import { spawn } from 'node:child_process';
import http from 'node:http';
import { PROBE } from './castle_probe.mjs';
const CHROME='/home/tjninja/.local/share/lets-study-test-browser/chrome/opt/google/chrome/chrome';
const PORT=9555;
function get(p){return new Promise((res,rej)=>{http.get({host:'127.0.0.1',port:PORT,path:p},r=>{let b='';r.on('data',c=>b+=c);r.on('end',()=>{try{res(JSON.parse(b))}catch(e){rej(e)}})}).on('error',rej)})}
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const p=spawn(CHROME,['--headless=new','--no-first-run','--no-sandbox','--remote-debugging-port='+PORT,'--user-data-dir=/tmp/probe-chrome','about:blank'],{stdio:['ignore','ignore','ignore']});
for(let i=0;i<60;i++){try{await get('/json/version');break}catch{await sleep(250)}}
const targets=await get('/json/list');
const page=targets.find(t=>t.type==='page');
const {default:WebSocket}=await import('../inputs/dom/node_modules/ws/wrapper.mjs');
const ws=new WebSocket(page.webSocketDebuggerUrl,{perMessageDeflate:false,maxPayload:64*1024*1024});
await new Promise((res,rej)=>{ws.on('open',res);ws.on('error',rej)});
const r=await new Promise(res=>{ws.on('message',d=>{const m=JSON.parse(d);if(m.id===1)res(m)});ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:PROBE,returnByValue:true,awaitPromise:true,timeout:20000}}))});
console.log(JSON.stringify(r.result.result.value, null, 1));
p.kill('SIGKILL');
