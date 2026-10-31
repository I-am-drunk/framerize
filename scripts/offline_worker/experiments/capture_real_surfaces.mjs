import { spawn } from 'node:child_process';
import http from 'node:http';
const CHROME='/home/tjninja/.local/share/lets-study-test-browser/chrome/opt/google/chrome/chrome';
const PORT=9566;
function get(p){return new Promise((res,rej)=>{http.get({host:'127.0.0.1',port:PORT,path:p},r=>{let b='';r.on('data',c=>b+=c);r.on('end',()=>{try{res(JSON.parse(b))}catch(e){rej(e)}})}).on('error',rej)})}
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const p=spawn(CHROME,['--headless=new','--no-first-run','--no-sandbox','--remote-debugging-port='+PORT,'--user-data-dir=/tmp/cap-surf','about:blank'],{stdio:['ignore','ignore','ignore']});
for(let i=0;i<60;i++){try{await get('/json/version');break}catch{await sleep(250)}}
const targets=await get('/json/list');
const page=targets.find(t=>t.type==='page');
const {default:WebSocket}=await import('../inputs/dom/node_modules/ws/wrapper.mjs');
const ws=new WebSocket(page.webSocketDebuggerUrl,{perMessageDeflate:false});
await new Promise((res,rej)=>{ws.on('open',res);ws.on('error',rej)});
const expr = `(async () => {
  const o = {};
  try { o.chromeKeys = Object.keys(window.chrome); } catch(e){ o.chromeKeys=null; }
  try { o.chromeApp = JSON.parse(JSON.stringify(window.chrome.app)); } catch(e){ o.chromeApp=String(window.chrome.app); }
  try { o.chromeLoadTimesType = typeof window.chrome.loadTimes; } catch(e){ o.chromeLoadTimesType=null; }
  try { o.chromeCsiType = typeof window.chrome.csi; } catch(e){ o.chromeCsiType=null; }
  try { const lt = window.chrome.loadTimes(); o.loadTimesSample = JSON.parse(JSON.stringify(lt)); } catch(e){ o.loadTimesSample='throws'; }
  try { const c = window.chrome.csi(); o.csiSample = JSON.parse(JSON.stringify(c)); } catch(e){ o.csiSample='throws'; }
  try { o.notifPerm = Notification.permission; } catch(e){ o.notifPerm='throws'; }
  try { o.permQuery = await navigator.permissions.query({name:'notifications'}).then(p=>p.state); } catch(e){ o.permQuery='throws:'+e.name; }
  try { o.hasFocus = document.hasFocus(); } catch(e){ o.hasFocus='throws'; }
  return o;
})()`;
const r=await new Promise(res=>{ws.on('message',d=>{const m=JSON.parse(d);if(m.id===1)res(m)});ws.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:expr,returnByValue:true,awaitPromise:true,timeout:15000}}))});
console.log(JSON.stringify(r.result.result.value, null, 1));
p.kill('SIGKILL');
