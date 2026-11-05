import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/clientCli.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { offerClientSetup } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

test('noninteractive setup never reads a profile or offers client installation', async () => {
  const io = { stdout() { assert.fail('Unexpected output'); }, stderr() { assert.fail('Unexpected output'); } };
  await offerClientSetup('/missing/profile.json', io, {});
});

test('declining client setup leaves the saved connection and client installations alone', async () => {
  let questions = 0;
  const io = { stdout() { assert.fail('Unexpected output'); }, stderr() { assert.fail('Unexpected output'); },
    async ask() { questions += 1; return 'no'; } };
  await offerClientSetup('/missing/profile.json', io, {});
  assert.equal(questions, 1);
});

test('accepting client setup still asks separately before each installation', async () => {
  const answers = ['yes', 'no', 'no'];
  const questions = [];
  let output = '';
  const io = { stdout(text) { output += text; }, stderr() { assert.fail('Unexpected error'); },
    async ask(question) { questions.push(question); return answers.shift(); } };
  const deps = {
    platform: 'linux', env: { PATH: '/synthetic/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' },
    io, prompt: io.ask, probe: () => false,
    runner: { async run() { assert.fail('Installation requires its own consent'); },
      async capture() { assert.fail('No installed client to probe'); } },
  };
  await offerClientSetup('/missing/profile.json', io, {}, deps);
  assert.equal(questions.length, 3);
  assert.match(questions[1], /Install Claude Code/);
  assert.match(questions[2], /Install Codex/);
  assert.match(output, /frm client claude or frm client codex/);
});
