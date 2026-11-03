import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflowPath = new URL('../.github/workflows/package-coexistence.yml', import.meta.url);
const requiredPaths = [
  'framerize-nexus/**', 'framerize-server/**',
  'scripts/package-coexistence.mjs', 'test/package-coexistence.test.mjs',
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
  assert.ok(uploads.length >= 2, 'retain both producer and consumer evidence');
  for (const step of uploads) {
    assert.match(step, /overwrite: true/);
    assert.match(step, /if-no-files-found: error/);
    assert.match(step, /name:.*\$\{\{\s*github\.run_id\s*\}\}/);
    assert.doesNotMatch(step, /github\.ref_name|github\.run_attempt/);
  }
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
    assert.match(version.trim(), /^['"]?22['"]?$/, 'qualify Node 22');
  }
  assert.doesNotMatch(source, /\bnpm (?:ci|install|publish)\b|\bnpm run (?:binary|build)\b/);
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

test('workflow declares pinned actions, all three runners, and root cwd', () => {
  validateExecution(readFileSync(workflowPath, 'utf8'));
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

test('policy fixture rejects unsafe upload names and silent missing artifacts', () => {
  const upload = `      - uses: actions/upload-artifact@${'a'.repeat(40)}
        with:
          name: pair-\${{ github.run_id }}
          overwrite: true
          if-no-files-found: error
`;
  const source = upload.repeat(2);
  validateUploads(source);
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
          node-version: 22
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
    ['working-directory: .', 'working-directory: framerize-nexus'],
    ['node-version: 22', 'node-version: 20'],
    ['node-version: 22', ''],
    ['node --test test/package-coexistence.test.mjs', 'npm ci'],
  ]) {
    assert.throws(() => validateExecution(source.replace(before, after)));
  }
});
