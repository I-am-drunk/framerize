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

test('workflow covers both package trees and root verifier changes', () => {
  validateEvents(readFileSync(workflowPath, 'utf8'));
});

test('workflow uses read-only permissions without publication', () => {
  validatePermissions(readFileSync(workflowPath, 'utf8'));
});

test('workflow retains rerunnable evidence with mandatory uploads', () => {
  validateUploads(readFileSync(workflowPath, 'utf8'));
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
