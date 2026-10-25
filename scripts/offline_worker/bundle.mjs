// Assemble software only. This never opens an account or invokes the SDK.
import fs from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join, resolve, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { parseArgs } from 'node:util';
import { CDN, MODULES, SUPPORT, DOM_PINS } from './pins.mjs';

const source = dirname(fileURLToPath(import.meta.url));
const modules = ['worker', 'offline_mint', 'protocol', 'device', 'pins',
  'graphics_loader', 'canvas_webgl', 'sdk_loader', 'branch', 'resources',
  'shared_workers', 'shared_runtime', 'dedicated_workers', 'dedicated_pool',
  'dedicated_replies', 'dedicated_runtime', 'collection', 'collected_values'];

async function digest(path) {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest('hex');
}

async function checked(path, expected) {
  if ((await fs.lstat(path)).isSymbolicLink() || await digest(path) !== expected)
    throw new Error('Worker input identity mismatch: ' + basename(path));
}

async function copy(from, to) {
  await fs.mkdir(dirname(to), { recursive: true });
  await fs.copyFile(from, to);
}

async function assemble(root, options) {
  const requirements = JSON.parse(await fs.readFile(join(source, 'dependencies/package.json')));
  if (process.versions.node !== requirements.engines.node)
    throw new Error('Build with Node ' + requirements.engines.node);
  await copy(process.execPath, join(root, 'bin/node'));
  await copy(resolve(dirname(process.execPath), '../LICENSE'), join(root, 'bin/LICENSE'));
  for (const name of modules) await copy(join(source, name + '.mjs'), join(root, name + '.mjs'));
  for (const [path, hash] of Object.values(SUPPORT)) {
    const input = join(source, 'support', basename(path));
    await checked(input, hash);
    await copy(input, join(root, 'inputs/support', basename(path)));
  }
  for (const [name, , hash] of Object.values(MODULES)) {
    const target = join(root, 'inputs/assets', name);
    await fs.mkdir(dirname(target), { recursive: true });
    if (options.assets) await copy(join(options.assets, name), target);
    else {
      if (options.offline) throw new Error('--offline requires --assets');
      const reply = await fetch(CDN + name, { signal: AbortSignal.timeout(15000), redirect: 'error' });
      if (!reply.ok) throw new Error('Cannot download pinned asset: ' + name);
      const bytes = Buffer.from(await reply.arrayBuffer());
      if (bytes.length > 8 * 1024 * 1024) throw new Error('SDK asset exceeds size limit');
      await fs.writeFile(target, bytes);
    }
    await checked(target, hash);
  }
  const dom = join(root, 'inputs/dom');
  for (const name of ['package.json', 'package-lock.json']) {
    await checked(join(source, 'dependencies', name), DOM_PINS[name]);
    await copy(join(source, 'dependencies', name), join(dom, name));
  }
  const npm = resolve(dirname(process.execPath), '../lib/node_modules/npm/bin/npm-cli.js');
  const result = spawnSync(process.execPath, [npm, 'ci', '--ignore-scripts', '--no-audit',
    '--no-fund', '--bin-links=false', ...(options.offline ? ['--offline'] : [])], {
    cwd: dom, stdio: 'inherit', timeout: 90000,
    env: { PATH: dirname(process.execPath) + ':/usr/bin:/bin', HOME: root,
      npm_config_userconfig: '/dev/null', npm_config_globalconfig: join(root, 'no-npmrc'),
      npm_config_cache: options.cache || join(root, '.npm-cache') },
  });
  if (result.error || result.status !== 0) throw new Error('Pinned npm installation failed');
  await fs.rm(join(root, '.npm-cache'), { recursive: true, force: true });
  for (const [name, hash] of Object.entries(DOM_PINS)) await checked(join(dom, name), hash);
  const names = ['bin/node', ...modules.map(name => name + '.mjs'),
    ...Object.values(SUPPORT).map(([name]) => 'inputs/support/' + basename(name)),
    ...Object.values(MODULES).map(([name]) => 'inputs/assets/' + name),
    ...Object.keys(DOM_PINS).map(name => 'inputs/dom/' + name)];
  const pins = [];
  for (const path of names) pins.push({ path, sha256: await digest(join(root, path)) });
  if (pins.length > 64) throw new Error('Worker pin limit exceeded');
  await fs.writeFile(join(root, 'worker-config.json'), JSON.stringify({
    node: 'bin/node', entry: 'worker.mjs', pins }, null, 2) + '\n');
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: source, encoding: 'utf8' });
  const head = commit.status === 0 ? commit.stdout.trim() : '';
  if (!/^[0-9a-f]{40}$/.test(head)) throw new Error('Worker source commit is unavailable');
  await fs.writeFile(join(root, 'bundle-source.json'), JSON.stringify({ commit: head }) + '\n');
}

async function inventory(root, directory = root, result = {}) {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) await inventory(root, path, result);
    else if (entry.isFile()) result[path.slice(root.length + 1)] = await digest(path);
    else throw new Error('Worker bundle contains a non-regular file');
  }
  return result;
}

const { values } = parseArgs({ options: { output: { type: 'string' },
  assets: { type: 'string' }, cache: { type: 'string' }, offline: { type: 'boolean' } } });
if (!values.output) throw new Error('Required: --output NEW_DIRECTORY');
const output = resolve(values.output);
await fs.mkdir(dirname(output), { recursive: true });
try {
  await fs.lstat(output);
  throw new Error('Output already exists; choose a new worker directory');
} catch (error) { if (error.code !== 'ENOENT') throw error; }
const temporary = await fs.mkdtemp(output + '.building-');
try {
  await assemble(temporary, values);
  const files = await inventory(temporary);
  await fs.writeFile(join(temporary, 'bundle-manifest.json'), JSON.stringify(files, null, 2) + '\n');
  await fs.rename(temporary, output);
  console.log(JSON.stringify({ output, files: Object.keys(files).length, sdk_executed: false }));
} finally {
  await fs.rm(temporary, { recursive: true, force: true });
}
