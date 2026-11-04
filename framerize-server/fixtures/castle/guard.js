import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import http from "node:http";
import dns from "node:dns";
import childProcess from "node:child_process";
import workerThreads, { parentPort, workerData } from "node:worker_threads";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { installCollectionGuard } from "../../dist/castle.js";
const { directory, modulePath, moduleSource, guard } = workerData;
const approved = join(directory, "approved.txt");
const unknown = join(directory, "absent.txt");
const state = join(directory, "realm-state.json");
let underlying = 0;
const sentinel = () => {
  underlying += 1;
  throw new Error("Underlying operation reached");
};
fs.readFileSync = sentinel;
fs.writeFileSync = sentinel;
fs.promises.readFile = sentinel;
const snapshot = /* @__PURE__ */ new Map([[approved, Buffer.from("approved")], [modulePath, Buffer.from(moduleSource)]]);
if (guard) installCollectionGuard(snapshot);
snapshot.get(approved).fill(0);
assert.equal(fs.readFileSync(approved, "utf8"), "approved");
const copy = fs.readFileSync(approved);
copy.fill(0);
assert.equal(fs.readFileSync(approved, "utf8"), "approved");
assert.equal(await fs.promises.readFile(approved, "utf8"), "approved");
assert.equal(fs.realpathSync(approved), approved);
const denied = [
  () => fs.readFileSync("/tmp/font-probe-capture.json"),
  () => fs.readFileSync(state),
  () => fs.readFileSync(unknown),
  () => fs.readFileSync(approved, { flag: "r+" }),
  () => fs.promises.readFile(unknown),
  () => fs.promises.open(unknown),
  () => fs.openSync(unknown, "r"),
  () => fs.createReadStream(unknown),
  () => fs.writeFileSync(state, "value"),
  () => fs.promises.writeFile(state, "value"),
  () => fs.renameSync(unknown, state),
  () => fs.rmSync(unknown),
  () => fs.symlinkSync(unknown, state),
  () => fs.statSync(unknown),
  () => net.connect({ port: -1 }),
  () => http.request("invalid-url"),
  () => dns.lookup({}, () => {
  }),
  () => childProcess.spawn(void 0),
  () => new workerThreads.Worker(void 0),
  () => fetch("invalid-url")
];
for (const attempt of denied) {
  await assert.rejects(async () => {
    await attempt();
  }, { code: "collection" });
}
const module = await import(pathToFileURL(modulePath).href);
assert.equal(module.marker, "synthetic-module");
await assert.rejects(import(pathToFileURL(unknown).href), /Unapproved Castle module/);
const unapproved = 'data:text/javascript,throw new Error("Unexpected import")';
await assert.rejects(import(unapproved), /Unapproved Castle module/);
await assert.rejects(import("node:child_process"), /Unapproved Castle module/);
assert.equal(underlying, 0);
parentPort?.postMessage({ checks: denied.length, imported: true });
