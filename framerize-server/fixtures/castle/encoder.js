import assert from "node:assert/strict";
import { writeFileSync } from "node:fs";
const mode = process.argv[2];
const chunks = [];
for await (const chunk of process.stdin) chunks.push(Buffer.from(chunk));
const frame = Buffer.concat(chunks);
assert.equal(process.cwd(), process.env.HOME);
assert.equal(process.env.HOME, process.env.TMPDIR);
assert.equal(process.env.CASTLE_PRIVATE_CANARY, void 0);
assert.equal(process.env.NODE_USE_ENV_PROXY, void 0);
assert.equal(process.env.HTTPS_PROXY, void 0);
if (mode === "success") process.stdout.write(frame.toString("hex") + "\n");
else if (mode === "invalid") process.stdout.write(Buffer.from([255]));
else if (mode === "empty") process.stdout.write(" \n");
else if (mode === "failure") {
  process.stderr.write("synthetic-secret");
  process.exitCode = 7;
} else {
  writeFileSync("started", String(process.pid));
  if (mode === "stdout") process.stdout.write("x".repeat(300 * 1024));
  if (mode === "stderr") process.stderr.write("x".repeat(300 * 1024));
  setInterval(() => {
  }, 1e3);
}
