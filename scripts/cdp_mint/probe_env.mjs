import { spawn } from 'node:child_process';
import http from 'node:http';
const CHROME='/home/tjninja/.local/share/lets-study-test-browser/chrome/opt/google/chrome/chrome';
const PORT=9444;
function get(p){return new Promise((res,rej)=>{http.get({host:'127.0.0.1',port:PORT,path:p},r=>{let b='';r.on('data',c=>b+=c);r.on('end',()=>{try{res(JSON.parse(b))}catch(e){rej(e)}})}).on('error',rej)})}
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const p=spawn(CHROME,['--headless=new','--no-first-run','--disable-gpu','--no-sandbox','--remote-debugging-port='+PORT,'--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36','--user-data-dir=/tmp/probe-prof','about:blank'],{stdio:['ignore','ignore','ignore']});
for(let i=0;i<60;i++){try{await get('/json/version');break}catch{await sleep(250)}}
const targets=await get('/json/list');
const page=targets.find(t=>t.type==='page');
const {default:WebSocket}=await import('../offline_worker/inputs/dom/node_modules/ws/wrapper.mjs');
const ws=new WebSocket(page.webSocketDebuggerUrl,{perMessageDeflate:false});
await new Promise((res,rej)=>{ws.on('open',res);ws.on('error',rej)});
const expr = `({
  processType: typeof process,
  hasBuffer: typeof Buffer!=='undefined',
  domAutomation: typeof window.domAutomation,
  domAutomationController: typeof window.domAutomationController,
  webdriver: navigator.webdriver,
  ua: navigator.userAgent,
  platform: navigator.platform,
  vendor: navigator.vendor,
  headlessHint: /Headless/.test(navigator.userAgent)
})`;
const r=await new Promise(res=>{ws.on('message',d=>{const m=JSON.parse(d);if(m.id===1)res(m)});ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:expr,returnByValue:true}}))});
console.log(JSON.stringify(r.result.result.value,null,1));
p.kill('SIGKILL');
