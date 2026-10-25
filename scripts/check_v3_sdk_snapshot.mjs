// One synthetic SDK operation, only in the dedicated isolated comparison view.
import fs from 'node:fs';
import {OfflineCastleMinter} from '/work/offline_mint.mjs';
import {takeSnapshot, writeCapturedCollection} from './castle_v3_snapshot.mjs';

const ns = kind => fs.readlinkSync('/proc/self/ns/'+kind);
for (const [kind,key] of [['net','FRAMERIZE_PARENT_NETNS'],['pid','FRAMERIZE_PARENT_PIDNS']]) {
  if (!process.env[key] || ns(kind) === process.env[key]) throw new Error('snapshot-isolation');
}
if (fs.existsSync('/home/tjninja/.framerize') || fs.existsSync('/run/user/1000/bus'))
  throw new Error('snapshot-private-state-visible');
const group = '/sys/fs/cgroup'+fs.readFileSync('/proc/self/cgroup','utf8').trim().split('::')[1];
for (const [name,value] of [['memory.max',134217728],['memory.swap.max',0],['pids.max',32]]) {
  if (Number(fs.readFileSync(group+'/'+name,'utf8')) !== value) throw new Error('snapshot-caps');
}
const minter = new OfflineCastleMinter({version:2,budget_ms:16000,
  target:{project_id:'fixture-project',url:'https://framer.com/projects/fixture-project',path:'/ai/v3/responses/'},
  device:{id:'synthetic-native-encoding-comparison'}});
const result = await minter.mint();
if (result.metadata.status !== 'passed' || !result.collection) {
  fs.writeFileSync('/out/sdk-result.json',JSON.stringify(result.metadata),{flag:'wx',mode:0o600});
  process.exitCode = 1;
} else {
  // The comparison loader retains the original encoder inside this operation.
  const snapshot = takeSnapshot(result.collection.take());
  // The isolated parent owns and drains this private pipe. Never log its bytes.
  writeCapturedCollection(1);
  fs.writeFileSync('/out/snapshot.json',JSON.stringify(snapshot),{flag:'wx',mode:0o600});
  fs.writeFileSync('/out/sdk-result.json',JSON.stringify({instrumented:true,synthetic:true,
    ...result.metadata,serialized_bytes:snapshot.serialized.length,token_discarded:true}),{flag:'wx',mode:0o600});
}
