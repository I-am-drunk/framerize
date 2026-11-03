import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import test from 'node:test';

const workflowPath = new URL('../.github/workflows/package-coexistence.yml', import.meta.url);
test('release requires source and package qualification without repacking or publishing', () => {
  const ci = readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8');
  const release = readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');
  assert.match(ci, /needs: source/);
  assert.match(ci, /uses: \.\/\.github\/workflows\/package-coexistence.yml/);
  assert.match(release, /needs: qualify/);
  assert.match(release, /uses: \.\/\.github\/workflows\/ci.yml/);
  assert.match(release, /node \.tools\/scripts\/package-release.js/);
  assert.match(release, /NODE_OPTIONS: ''/);
  assert.doesNotMatch(release, /npm (?:pack|publish)|build-binary|package-binary|contents: write/);
});
const requireBuiltin = createRequire(import.meta.url);
test('source triggers cover build inputs and package workflow changes', () => {
  const core = readFileSync(new URL('../.github/workflows/core-ci.yml', import.meta.url), 'utf8');
  const pair = readFileSync(workflowPath, 'utf8');
  for (const source of [core, pair]) {
    for (const file of ['framerize-server/**', 'framerize-castle/**',
      'scripts/core-package-check.ts', '.github/workflows/ci.yml',
      '.github/workflows/release.yml', '.github/workflows/package-coexistence.yml']) {
      assert.equal(source.split(`      - '${file}'`).length - 1, 2, file);
    }
  }
});
const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');

function workflowScript(name) {
  const source = readFileSync(workflowPath, 'utf8').replace(/\r\n/g, '\n');
  const step = source.split(/^      - /m).find((entry) => entry.startsWith('name: ' + name + '\n'));
  assert.ok(step, 'missing workflow step: ' + name);
  const body = step.match(/^          node <<'JS'\n([\s\S]*?)^          JS$/m)?.[1];
  assert.ok(body, 'missing inline Node script: ' + name);
  return new Function('require', 'process', 'console', body.replace(/^          /gm, ''));
}

function evidenceFixture(platform = 'linux', arch = 'x64') {
  const directory = path.resolve('virtual-coexistence');
  const input = path.join(directory, 'input');
  const report = path.join(directory, 'report');
  const files = new Map();
  const put = (file, bytes) => files.set(path.resolve(file), Buffer.from(bytes));
  const packages = ['framerize-remote', 'framerize-server'].map((name) => {
    const version = name === 'framerize-remote' ? '0.1.9' : '0.1.0';
    const tarball = name + '-' + version + '.tgz';
    const bytes = Buffer.from('fixture bytes for ' + name);
    put(path.join(input, tarball), bytes);
    put(name + '/package.json', JSON.stringify({ name, version }));
    return { name, version, tarball, sha256: digest(bytes), bytes: bytes.length };
  });
  const source = { repository: 'fixture/framerize', sha: 'a'.repeat(40), tree: 'c'.repeat(40), event: 'pull_request',
    runId: '1234', runAttempt: '1' };
  const tooling = ['package.json', 'scripts/package-coexistence.js', 'scripts/package-archive.js',
    'scripts/core-package-check.js', 'scripts/fixtures/core-installed.js',
    'scripts/fixtures/installed-server.js', 'scripts/fixtures/transport-scenario.js',
    'test/installed-remote-flow.test.js', 'test/package-coexistence.test.js', 'test/package-archive.test.js']
    .map((file) => {
      const bytes = Buffer.from(file === 'package.json' ? '{"type":"module"}\n' : 'synthetic compiled tool');
      put(path.join('.tools', file), bytes);
      put(path.join(input, 'tooling', file), bytes);
      return { file, bytes: bytes.length, sha256: digest(bytes) };
    });
  const manifest = { source, node: process.version, npm: '10.9.9',
    platform: 'linux', arch: 'x64', packages, tooling };
  const checker = { node: process.version, npm: '10.9.9', platform, arch,
    packages: packages.map(({ bytes, ...entry }) => ({ ...entry, files: 13 })) };
  const manifestFile = path.join(input, 'manifest.json');
  const checkerFile = path.join(report, 'checker.json');
  put(path.join(report, 'installed.log'), 'synthetic successful installed check');
  const save = () => {
    put(manifestFile, JSON.stringify(manifest));
    put(checkerFile, JSON.stringify(checker));
  };
  save();
  const env = {
    INPUT_DIR: input, ARTIFACT_DIR: input, REPORT_DIR: report,
    GITHUB_ENV: path.join(directory, 'env'),
    GITHUB_OUTPUT: path.join(directory, 'output'),
    GITHUB_SHA: source.sha, GITHUB_REPOSITORY: source.repository,
    GITHUB_EVENT_NAME: source.event,
    GITHUB_RUN_ID: source.runId, GITHUB_RUN_ATTEMPT: '2',
    PRODUCER_NPM: '10.9.9',
    PRODUCER_MANIFEST_SHA256: digest(files.get(manifestFile)),
    EXPECTED_PLATFORM: platform, EXPECTED_ARCH: arch,
    RUNNER_LABEL: platform + '-fixture', TARGET: platform + '-' + arch,
  };
  const writes = [];
  const fs = {
    mkdirSync() {},
    lstatSync(file) {
      return { isFile: () => files.has(path.resolve(file)),
        isDirectory: () => !files.has(path.resolve(file)) };
    },
    readFileSync(file, encoding) {
      const bytes = files.get(path.resolve(file));
      assert.ok(bytes, 'missing virtual file: ' + file);
      return encoding ? bytes.toString(encoding) : Buffer.from(bytes);
    },
    writeFileSync(file, bytes) {
      writes.push(file);
      put(file, bytes);
    },
    appendFileSync(file, bytes) {
      writes.push(file);
      put(file, Buffer.concat([files.get(path.resolve(file)) ?? Buffer.alloc(0), Buffer.from(bytes)]));
    },
    readdirSync(folder, options) {
      const relative = [...files.keys()].filter((file) => file.startsWith(path.resolve(folder) + path.sep))
        .map((file) => path.relative(folder, file));
      return options?.recursive ? relative : [...new Set(relative.map(file => file.split(path.sep)[0]))];
    },
  };
  let checkout = source.sha;
  const execute = (name) => {
    workflowScript(name)((module) => {
      if (module === 'node:fs') return fs;
      if (module === 'node:child_process') return {
        execFileSync(command, args) {
          assert.equal(command, 'git', 'fixtures must not invoke package managers or the network');
          if (args[0] === 'rev-parse' && args[1] === 'HEAD^{tree}') return 'c'.repeat(40);
          assert.deepEqual(args, ['rev-parse', 'HEAD']);
          return checkout;
        },
      };
      assert.ok(['node:assert/strict', 'node:crypto', 'node:path'].includes(module));
      return requireBuiltin(module);
    }, { env, version: process.version, platform, arch }, { log() {} });
    for (const line of (files.get(env.GITHUB_ENV)?.toString() ?? '').split('\n')) {
      const separator = line.indexOf('=');
      if (separator > 0) env[line.slice(0, separator)] = line.slice(separator + 1);
    }
  };
  return { files, put, packages, manifest, checker, manifestFile, checkerFile,
    input, report, env, save, execute, writes,
    setCheckout(value) { checkout = value; } };
}

const producerStep = 'Write producer manifest beside exactly two tarballs';
const verifyStep = 'Verify producer manifest, source and input hashes';
const reportStep = 'Match checker identities to producer manifest';

test('actual workflow reuses attempt-one producer bytes on attempt-two consumers', () => {
  const producer = evidenceFixture();
  producer.files.delete(producer.manifestFile);
  for (const file of producer.files.keys()) {
    if (file.startsWith(path.join(producer.input, 'tooling') + path.sep)) producer.files.delete(file);
  }
  producer.env.GITHUB_RUN_ATTEMPT = '1';
  producer.execute(producerStep);
  const manifestBytes = producer.files.get(producer.manifestFile);
  const metadata = JSON.parse(manifestBytes);
  assert.equal(metadata.source.runAttempt, '1');
  for (const [platform, arch] of [['linux', 'x64'], ['darwin', 'arm64'], ['win32', 'x64']]) {
    const consumer = evidenceFixture(platform, arch);
    consumer.put(consumer.manifestFile, manifestBytes);
    consumer.execute(verifyStep);
    consumer.execute(reportStep);
    const report = JSON.parse(consumer.files.get(path.join(consumer.report, 'report.json')));
    assert.equal(report.source.runAttempt, '2');
    assert.equal(report.producer.source.runAttempt, '1');
    assert.equal(report.producer.manifestSha256, digest(manifestBytes));
    assert.deepEqual(report.packages.map((entry) => entry.sha256),
      metadata.packages.map((entry) => entry.sha256));
    assert.equal(report.source.sha, report.producer.source.sha);
  }
});

function rejectsBeforeExport(change, expected) {
  const fixture = evidenceFixture();
  change(fixture);
  fixture.save();
  assert.throws(() => fixture.execute(verifyStep), expected);
  assert.deepEqual(fixture.writes, [], 'invalid artifacts must not reach install-path exports');
}

const identityFailures = [
  ['wrong tree', fixture => { fixture.manifest.source.tree = 'd'.repeat(40); }, /source tree/],
  ['wrong source', (fixture) => { fixture.manifest.source.sha = 'b'.repeat(40); }, /different source SHA/],
  ['wrong checkout', (fixture) => { fixture.setCheckout('b'.repeat(40)); }, /triggering SHA/],
  ['wrong repository', (fixture) => { fixture.manifest.source.repository = 'other/repository'; }],
  ['wrong run', (fixture) => { fixture.manifest.source.runId = '9999'; }],
  ['wrong event', (fixture) => { fixture.manifest.source.event = 'push'; }],
  ['absent package evidence', (fixture) => { delete fixture.manifest.packages; }],
  ['missing package', (fixture) => { fixture.manifest.packages.pop(); }],
  ['duplicate package', (fixture) => { fixture.manifest.packages[1] = fixture.packages[0]; }],
  ['unexpected package', (fixture) => { fixture.packages[1].name = 'unexpected'; }],
  ['wrong version', (fixture) => { fixture.packages[0].version = '9.9.9'; }, /version differs/],
  ['wrong size', (fixture) => { fixture.packages[0].bytes += 1; }, /size differs/],
  ['wrong digest', (fixture) => { fixture.packages[0].sha256 = 'b'.repeat(64); }, /hash differs/],
];
for (const [name, change, expected] of identityFailures) {
  test('actual preinstall verification rejects ' + name, () => rejectsBeforeExport(change, expected));
}

test('actual preinstall verification rejects changed tarball bytes with unchanged size', () => {
  rejectsBeforeExport((fixture) => {
    const file = path.join(fixture.input, fixture.packages[0].tarball);
    const bytes = Buffer.from(fixture.files.get(file));
    bytes[0] ^= 1;
    fixture.put(file, bytes);
  }, /hash differs/);
});

test('actual preinstall verification rejects absent, extra, and renamed tarballs', () => {
  rejectsBeforeExport((fixture) => {
    fixture.files.delete(path.join(fixture.input, fixture.packages[0].tarball));
  }, /exactly the manifest and two tarballs/);
  rejectsBeforeExport((fixture) => {
    fixture.put(path.join(fixture.input, 'extra.tgz'), 'extra');
  }, /exactly the manifest and two tarballs/);
  rejectsBeforeExport((fixture) => {
    const entry = fixture.packages[0];
    const old = path.join(fixture.input, entry.tarball);
    fixture.put(path.join(fixture.input, 'renamed.tgz'), fixture.files.get(old));
    fixture.files.delete(old);
    entry.tarball = 'renamed.tgz';
  }, /renamed.tgz/);
});

function rejectsBeforeReport(change, expected) {
  const fixture = evidenceFixture();
  fixture.execute(verifyStep);
  fixture.writes.length = 0;
  change(fixture);
  fixture.save();
  assert.throws(() => fixture.execute(reportStep), expected);
  assert.deepEqual(fixture.writes, [], 'invalid checks must not produce a successful report');
}

test('compiled tooling is hash-bound before and after execution', () => {
  rejectsBeforeExport(fixture => {
    fixture.put(path.join(fixture.input, 'tooling/scripts/package-coexistence.js'), 'tampered');
  }, /tooling (size|hash) mismatch/);
  rejectsBeforeReport(fixture => {
    fixture.put(path.join(fixture.input, 'tooling/scripts/package-coexistence.js'), 'tampered');
  }, /tooling changed/);
});

for (const key of ['name', 'version', 'tarball', 'sha256']) {
  test('actual final comparison rejects substituted checker ' + key, () => {
    rejectsBeforeReport((fixture) => {
      fixture.checker.packages[0][key] = 'substituted';
    }, /checker identities differ/);
  });
}

test('actual final comparison rejects missing or duplicate checker packages', () => {
  rejectsBeforeReport((fixture) => { fixture.checker.packages.pop(); }, /identities differ/);
  rejectsBeforeReport((fixture) => {
    fixture.checker.packages[1] = fixture.checker.packages[0];
  }, /identities differ/);
});

test('actual final comparison rejects changed checkout, manifest, or payload', () => {
  rejectsBeforeReport((fixture) => {
    fixture.setCheckout('b'.repeat(40));
  }, /checkout changed/);
  rejectsBeforeReport((fixture) => {
    fixture.manifest.source.sha = 'b'.repeat(40);
  }, /manifest changed/);
  rejectsBeforeReport((fixture) => {
    fixture.put(path.join(fixture.input, fixture.packages[0].tarball), 'changed');
  }, /changed after verification/);
});
const requiredPaths = [
  'framerize-remote/**', 'framerize-server/**',
  'framerize-core/**', 'scripts/build-server.ts', 'package*.json', 'tsconfig*.json',
  'scripts/package-coexistence.ts', 'test/package-coexistence.test.ts',
  'scripts/package-archive.ts', 'test/package-archive.test.ts',
  '.github/workflows/package-coexistence.yml',
  'test/package-coexistence-workflow.test.mjs',
];

function section(source, heading, indent = 0) {
  const lines = source.replace(/\r\n/g, '\n').split('\n');
  const start = lines.findIndex((line) => line === `${' '.repeat(indent)}${heading}:`);
  assert.notEqual(start, -1, `missing ${heading} section`);
  const end = lines.findIndex((line, index) => index > start && line.trim()
    && line.search(/\S/) <= indent);
  return lines.slice(start + 1, end < 0 ? undefined : end).join('\n');
}

function validateEvents(source) {
  const events = section(source, 'on');
  assert.match(events, /^  workflow_dispatch:/m);
  for (const event of ['push', 'pull_request']) {
    const body = section(events, event, 2);
    const paths = section(body, 'paths', 4).split('\n')
      .map((line) => line.trim().replace(/^-\s*['"]?|['"]$/g, ''));
    for (const path of requiredPaths) assert.ok(paths.includes(path), `${event} omits ${path}`);
    if (event === 'push') {
      assert.match(body, /branches:[\s\S]*\bmain\b/);
      assert.match(body, /branches:[\s\S]*\bframer-next\b/);
    }
  }
}

function validatePermissions(source) {
  assert.deepEqual(section(source, 'permissions').trim().split('\n')
    .map((line) => line.trim()), ['contents: read']);
  assert.equal([...source.matchAll(/^\s*permissions:/gm)].length, 1);
  assert.doesNotMatch(source, /pull_request_target|id-token:|contents: write|npm publish|action-gh-release/);
}

function validateUploads(source) {
  const uploads = source.split(/^      - /m).filter((step) => /uses: actions\/upload-artifact@/.test(step));
  assert.equal(uploads.length, 2, 'retain producer and matrix consumer evidence');
  const names = [];
  for (const step of uploads) {
    assert.match(step, /overwrite: true/);
    assert.match(step, /if-no-files-found: error/);
    assert.match(step, /name:.*\$\{\{\s*github\.run_id\s*\}\}/);
    assert.doesNotMatch(step, /github\.ref_name|github\.run_attempt/);
    const name = step.match(/^          name:\s*(.+)$/m)?.[1];
    assert.ok(name, 'artifact name is required');
    names.push(name.replace(/^['"]|['"]$/g, ''));
  }
  const producers = names.filter((name) => !/matrix\./.test(name));
  const consumers = names.filter((name) => /matrix\./.test(name));
  assert.equal(producers.length, 1, 'require one producer artifact name');
  assert.equal(consumers.length, 1, 'require a matrix-specific consumer artifact name');
  const matrix = section(section(source, 'matrix', 6), 'include', 8);
  const rows = matrix.split(/^          - /m).filter((row) => row.trim()).map((row) =>
    Object.fromEntries(row.trim().split('\n').map((line) => {
      const match = line.trim().match(/^([\w-]+):\s*(\S+)$/);
      assert.ok(match, 'matrix entries must be scalar identities');
      return [match[1], match[2].replace(/^['"]|['"]$/g, '')];
    })));
  assert.equal(rows.length, 3, 'require three platform identities');
  const nodes = ['22.12.0', '22.23.4', '24.13.0'];
  assert.match(source, /node: \['22\.12\.0', '22\.23\.4', '24\.13\.0'\]/);
  const expanded = [expandName(producers[0]), ...nodes.flatMap(node =>
    rows.map(row => expandName(consumers[0], { ...row, node })))];
  assert.equal(new Set(expanded).size, 10, 'producer and platform artifacts must not collide');
}

function expandName(template, matrix = {}, attempt = 1) {
  return template.replace(/\$\{\{\s*([^}]+?)\s*\}\}/g, (match, expression) => {
    const values = { 'github.run_id': '1234', 'github.run_attempt': String(attempt),
      ...Object.fromEntries(Object.entries(matrix).map(([key, value]) => [`matrix.${key}`, value])) };
    assert.ok(Object.hasOwn(values, expression), `unknown artifact identity ${expression}`);
    return values[expression];
  });
}

function validateExecution(source) {
  const pins = new Map([
    ['actions/checkout', '11bd71901bbe5b1630ceea73d27597364c9af683'],
    ['actions/setup-node', '49933ea5288caeca8642d1e84afbd3f7d6820020'],
    ['actions/upload-artifact', 'ea165f8d65b6e75b540449e92b4886f43607fa02'],
    ['actions/download-artifact', 'd3f86a106a0bac45b974a628896c90dbdf5c8093'],
  ]);
  const actions = [...source.matchAll(/\buses:\s*(\S+)@(\S+)/g)];
  assert.ok(actions.length >= pins.size);
  for (const [, action, pin] of actions) {
    assert.ok(pins.has(action), `unexpected action ${action}`);
    assert.equal(pin, pins.get(action), `unexpected pin for ${action}`);
  }
  for (const action of pins.keys()) assert.ok(actions.some((match) => match[1] === action));
  for (const runner of ['ubuntu-22.04', 'macos-15', 'windows-2022']) {
    assert.ok(source.includes(runner), `missing runner ${runner}`);
  }
  for (const [, directory] of source.matchAll(/working-directory:\s*([^\r\n]+)/g)) {
    assert.ok(['.', './'].includes(directory.trim()), 'commands must run from repository root');
  }
  const versions = [...source.matchAll(/node-version:\s*([^\r\n]+)/g)];
  assert.ok(versions.length > 0, 'select a Node version');
  for (const [, version] of versions) {
    assert.match(version.trim(), /^(?:['"]22\.23\.4['"]|\$\{\{ matrix\.node \}\})$/, 'qualify pinned Node');
  }
  const build = 'npm ci --ignore-scripts --no-audit --no-fund && npm run build';
  const withoutBuild = source.replace(build, '').replace('npm ci --ignore-scripts --no-audit --no-fund', '');
  assert.doesNotMatch(withoutBuild, /\bnpm (?:ci|install|publish)\b|\bnpm run (?:binary|build)\b/);
  assert.doesNotMatch(source, /(?:echo|printf)[^\r\n]*NODE_OPTIONS/,
    'GitHub blocks NODE_OPTIONS writes through GITHUB_ENV');
}

test('workflow covers both package trees and root verifier changes', () => {
  validateEvents(readFileSync(workflowPath, 'utf8'));
});

test('workflow uses read-only permissions without publication', () => {
  validatePermissions(readFileSync(workflowPath, 'utf8'));
});

test('workflow retains rerunnable evidence with mandatory uploads', () => {
  validateUploads(readFileSync(workflowPath, 'utf8'));
});

test('failed installed checks remain visible without hiding their exit status', () => {
  const source = readFileSync(workflowPath, 'utf8');
  const step = source.split(/^      - /m)
    .find(value => value.startsWith('name: Exercise the downloaded pair without repacking\n'));
  assert.ok(step);
  assert.match(step, /set -euo pipefail/);
  assert.match(step, /node --test .*installed-remote-flow\.test\.js" 2>&1 \| tee "\$REPORT_DIR\/installed\.log"/);
  assert.doesNotMatch(step, /continue-on-error|\|\| true/);
});

test('workflow declares pinned actions, all three runners, and root cwd', () => {
  validateExecution(readFileSync(workflowPath, 'utf8'));
});

test('only the producer builds the shared core before packaging', () => {
  const source = readFileSync(workflowPath, 'utf8');
  const producer = section(source, 'produce', 2);
  const build = 'npm ci --ignore-scripts --no-audit --no-fund && npm run build';
  assert.equal(source.split(build).length - 1, 1);
  assert.ok(producer.includes(build));
  assert.ok(producer.indexOf(build) < producer.indexOf('Pack each package once'));
});

function validateProducerDownload(input) {
  const source = input.replace(/\r\n/g, '\n');
  assert.match(section(source, 'env'), /^  NODE_OPTIONS: ''$/m);
  const steps = source.split(/^      - /m);
  const upload = steps.find((step) => step.startsWith('name: Upload producer artifact\n'));
  const download = steps.find((step) => step.startsWith('name: Download producer artifact\n'));
  const artifactName = (step) => step?.match(/^          name: (.+)$/m)?.[1];
  assert.ok(artifactName(upload));
  assert.ok(artifactName(download));
  const stored = new Map([[expandName(artifactName(upload), {}, 1), 'producer-attempt-one']]);
  assert.equal(stored.get(expandName(artifactName(download), {}, 2)), 'producer-attempt-one');
}

test('workflow clears Node options declaratively and downloads the original producer', () => {
  validateProducerDownload(readFileSync(workflowPath, 'utf8'));
});

test('workflow producer binding accepts a CRLF checkout', () => {
  validateProducerDownload(readFileSync(workflowPath, 'utf8').replace(/\r?\n/g, '\r\n'));
});

const eventFixture = `on:
  push:
    branches: [main, framer-next]
    paths:
${requiredPaths.map((path) => `      - '${path}'`).join('\n')}
  pull_request:
    paths:
${requiredPaths.map((path) => `      - '${path}'`).join('\n')}
  workflow_dispatch:
permissions:
  contents: read
`;

test('policy fixture rejects every omitted path for either event', () => {
  validateEvents(eventFixture);
  validateEvents(eventFixture.replace(/\n/g, '\r\n'));
  for (const path of requiredPaths) {
    const line = `      - '${path}'\n`;
    assert.throws(() => validateEvents(eventFixture.replace(line, '')), /push omits/);
    const last = eventFixture.lastIndexOf(line);
    assert.throws(() => validateEvents(eventFixture.slice(0, last)
      + eventFixture.slice(last + line.length)), /pull_request omits/);
  }
});

test('policy fixture rejects privileged permissions and trigger changes', () => {
  validatePermissions(eventFixture);
  for (const change of ['  contents: write', '  id-token: write', '  actions: write']) {
    assert.throws(() => validatePermissions(eventFixture.replace('  contents: read', change)));
  }
  assert.throws(() => validatePermissions(`${eventFixture}jobs:\n  extra:\n    permissions: write-all\n`));
  assert.throws(() => validatePermissions(eventFixture.replace('pull_request:', 'pull_request_target:')));
});

test('policy fixture rejects colliding, unsafe, or silently missing artifacts', () => {
  const upload = `      - uses: actions/upload-artifact@${'a'.repeat(40)}
        with:
          name: pair-\${{ github.run_id }}
          overwrite: true
          if-no-files-found: error
`;
  const consumer = upload.replace('pair-${{ github.run_id }}',
    'pair-${{ github.run_id }}-${{ matrix.target }}-node-${{ matrix.node }}');
  const source = `jobs:
  producer:
    steps:
${upload}
  consumers:
    strategy:
      matrix:
        node: ['22.12.0', '22.23.4', '24.13.0']
        include:
          - target: linux-x64
            arch: x64
          - target: macos-arm64
            arch: arm64
          - target: windows-x64
            arch: x64
    steps:
${consumer}`;
  validateUploads(source);
  assert.throws(() => validateUploads(upload.repeat(2)), /one producer/);
  assert.throws(() => validateUploads(source.replace('-${{ matrix.target }}', '')), /must not collide/);
  assert.throws(() => validateUploads(source.replace('matrix.target', 'matrix.arch')), /must not collide/);
  assert.throws(() => validateUploads(source.replace('windows-x64', 'linux-x64')), /must not collide/);
  assert.throws(() => validateUploads(source.replace('matrix.target', 'matrix.missing')), /unknown artifact/);
  assert.throws(() => validateUploads(source.replace('name: pair-${{ github.run_id }}\n',
    'name: pair-${{ github.run_id }}-linux-x64-node-22.12.0\n')), /must not collide/);
  for (const [before, after] of [
    ['overwrite: true', 'overwrite: false'],
    ['if-no-files-found: error', 'if-no-files-found: warn'],
    ['github.run_id', 'github.ref_name'],
    ['github.run_id', 'github.run_attempt'],
  ]) {
    assert.throws(() => validateUploads(source.replace(before, after)));
  }
});

test('policy fixture rejects changed action pins, platforms, and package cwd', () => {
  const source = `jobs:
  package:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683
      - uses: actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020
        with:
          node-version: '22.23.4'
      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02
  consumers:
    strategy:
      matrix:
        os: [ubuntu-22.04, macos-15, windows-2022]
    steps:
      - uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093
      - run: node --test test/package-coexistence.test.mjs
        working-directory: .
`;
  validateExecution(source);
  for (const [before, after] of [
    ['11bd71901bbe5b1630ceea73d27597364c9af683', 'v4'],
    ['actions/checkout', 'unexpected/checkout'],
    ['macos-15', 'ubuntu-24.04'],
    ['working-directory: .', 'working-directory: framerize-remote'],
    ["node-version: '22.23.4'", 'node-version: 20'],
    ["node-version: '22.23.4'", ''],
    ['node --test test/package-coexistence.test.mjs', 'echo "NODE_OPTIONS=" >> "$GITHUB_ENV"'],
    ['node --test test/package-coexistence.test.mjs', 'npm ci'],
  ]) {
    assert.throws(() => validateExecution(source.replace(before, after)));
  }
});
