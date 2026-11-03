import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync, copyFileSync, rmSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { existsSync } from 'node:fs';

function npmCli() {
  if (process.env.npm_execpath) return process.env.npm_execpath;
  const binary = path.dirname(process.execPath);
  const candidates = [
    path.join(binary, 'node_modules', 'npm', 'bin', 'npm-cli.js'),
    path.join(binary, '..', 'lib', 'node_modules', 'npm', 'bin', 'npm-cli.js'),
  ];
  const sibling = candidates.find(existsSync);
  if (sibling) return sibling;
  const manifest = createRequire(path.join(binary, 'resolve.js')).resolve('npm/package.json');
  return path.join(path.dirname(manifest), 'bin', 'npm-cli.js');
}
const npm = npmCli();
const smoke = fileURLToPath(new URL('../../scripts/package-smoke.js', import.meta.url));
const run = (command, args, options = {}) => execFileSync(command, args, { encoding: 'utf8', ...options });
const temp = (prefix) => mkdtempSync(path.join(tmpdir(), prefix));

export function packageFixture(t, { environmentAware = false } = {}) {
  const root = temp('framerize-package-fixture-');
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const repository = path.join(root, 'checkout');
  const directory = path.join(repository, 'framerize-server');
  for (const folder of ['src', 'bin', 'scripts', 'dist']) mkdirSync(path.join(directory, folder), { recursive: true });
  const providerFiles = ['provider.js', 'provider.d.ts', 'core.d.ts',
    'backend.js', 'backend.d.ts', 'castle-loader.js', 'castle-loader.d.ts',
    'castle-assets.js', 'castle-assets.d.ts',
    'framer-auth.js', 'framer-auth.d.ts',
    'account-backend.js', 'account-backend.d.ts',
    'framer-backend.js', 'framer-backend.d.ts', 'framer-request.js', 'framer-request.d.ts',
    'cli.js', 'cli.d.ts', 'logger.js', 'logger.d.ts', 'server.js', 'server.d.ts',
    ...['backend', 'http-policy', 'instance', 'instance-options', 'readiness'].map((name) => `core-types/${name}.d.ts`)];
  mkdirSync(path.join(directory, 'dist/core-types'));
  writeFileSync(path.join(directory, 'dist/castle-manifest.json'), '{"version":1,"files":[]}');
  for (const name of providerFiles) writeFileSync(path.join(directory, 'dist', name), 'export {};\n');
  writeFileSync(path.join(directory, 'dist/core.js'), 'export const fixture = true;\n');
  writeFileSync(path.join(directory, 'dist/accounts.js'), 'export async function loadAccounts() { return Object.freeze([]); }\n');
  writeFileSync(path.join(directory, 'dist/accounts.d.ts'), 'export declare function loadAccounts(): Promise<readonly unknown[]>;\n');
  writeFileSync(path.join(directory, 'dist/errors.js'), 'export {};\n');
  writeFileSync(path.join(directory, 'dist/errors.d.ts'), 'export {};\n');
  writeFileSync(path.join(directory, 'dist/config.d.ts'), 'export declare function loadConfig(values: unknown): unknown;\n');
  writeFileSync(path.join(directory, 'src/config.ts'), 'throw new Error("must not ship TypeScript source");\n');
  writeFileSync(path.join(directory, 'package.json'), JSON.stringify({
    name: 'framerize-server', version: '0.1.0', private: true, type: 'module',
    bin: { 'framerize-server': './bin/framerize-server.js' },
    exports: { '.': './dist/server.js', './config': './dist/config.js', './accounts': './dist/accounts.js' },
    files: ['bin', 'dist/core.js', 'dist/config.js', 'dist/config.d.ts', 'dist/castle-manifest.json',
      'dist/errors.js', 'dist/errors.d.ts', 'dist/accounts.js', 'dist/accounts.d.ts',
      ...providerFiles.map((name) => `dist/${name}`), 'README.md', 'LICENSE'],
    scripts: { 'pack:check': 'node scripts/package-smoke.js' },
  }));
  writeFileSync(path.join(directory, 'LICENSE'), 'Fixture license\n');
  writeFileSync(path.join(directory, 'README.md'), '# Fixture package\n');
  writeFileSync(path.join(directory, 'dist', 'config.js'), environmentAware ? `
export const loadConfig = (values, env = process.env) => {
  const selected = (key, name, fallback) => Object.hasOwn(values, key) ? values[key] : env[name] ?? fallback;
  return { ...values,
    host: selected('host', 'FRAMERIZE_SERVER_HOST', '127.0.0.1'),
    port: Number(selected('port', 'FRAMERIZE_SERVER_PORT', 8789)),
    upstream: selected('upstream', 'FRAMERIZE_SERVER_UPSTREAM', ''),
  };
};
` : 'export const loadConfig = (values) => values;\n');
  writeFileSync(path.join(directory, 'dist', 'server.js'), `
import http from 'node:http';
export async function createServer(config) {
  ${environmentAware ? "if (config.host !== '127.0.0.1' || config.port !== 0 || config.upstream !== '') throw new Error('Unsafe fixture configuration before listen');" : ''}
  const server = http.createServer((request, response) => {
    response.setHeader('content-type', 'application/json');
    if (request.url === '/health') return response.end('{}');
    if (request.headers.authorization !== 'Bearer package-fixture') {
      response.statusCode = 401; return response.end('{}');
    }
    if (request.url === '/ready') return response.end('{"apiVersion":1}');
    request.resume(); request.on('end', () => response.end('{}'));
  });
  return {
    async listen() { await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve)); return server.address(); },
    async close() { server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)); },
  };
}
`);
  copyFileSync(smoke, path.join(directory, 'scripts', 'package-smoke.js'));
  const launcher = Buffer.from('#!/usr/bin/env node\r\nconsole.log("body");\r\n');
  writeFileSync(path.join(directory, 'bin', 'framerize-server.js'), launcher);
  const git = (args) => run('git', args, { cwd: repository });
  git(['init', '-q']);
  git(['config', 'user.email', 'fixture@example.invalid']);
  git(['config', 'user.name', 'Fixture']);
  git(['config', 'core.autocrlf', 'false']);
  git(['add', '.']);
  git(['commit', '-qm', 'fixture']);
  const commit = git(['rev-parse', 'HEAD']).trim();
  return { root, repository, directory, git, commit };
}

export function runPackageSmoke(directory, downloaded = false, env = {}) {
  return spawnSync(process.execPath, [path.join(directory, 'scripts', 'package-smoke.js'), ...(downloaded ? ['--downloaded'] : [])], {
    cwd: directory, encoding: 'utf8', timeout: 30_000,
    env: { ...process.env, ...env, npm_execpath: npm },
  });
}

export function packageTarball(directory) {
  run(process.execPath, [npm, 'pack', '--ignore-scripts', '--pack-destination', directory], { cwd: directory });
}

export function assertPackageFailure(result, message) {
  if (result.status === 0 || result.error || result.signal) {
    throw new Error(`Package check did not fail cleanly: ${result.stdout}\n${result.stderr}`);
  }
  if (!`${result.stdout}\n${result.stderr}`.includes(message)) {
    throw new Error(`Missing ${message}: ${result.stdout}\n${result.stderr}`);
  }
}

export function mutateTarball(t, directory, entry, content) {
  const root = temp('framerize-package-tar-');
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const archive = path.join(directory, 'framerize-server-0.1.0.tgz');
  const local = path.join(root, 'package.tgz');
  copyFileSync(archive, local);
  run('tar', ['-xzf', 'package.tgz'], { cwd: root });
  writeFileSync(path.join(root, 'package', entry), content);
  run('tar', ['-czf', 'package.tgz', 'package'], { cwd: root });
  copyFileSync(local, archive);
}
