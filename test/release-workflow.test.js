import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');

test('release publishes binaries before attempting npm authentication', () => {
  const release = workflow.indexOf('name: Publish GitHub release');
  const npm = workflow.indexOf('name: Publish npm package');
  assert.ok(release > 0);
  assert.ok(npm > release);
});

test('release verifies the packaged artifacts before upload', () => {
  const validate = workflow.indexOf('name: Locate build artifacts');
  const upload = workflow.indexOf('uses: actions/upload-artifact@');
  assert.ok(validate > 0);
  assert.ok(upload > validate);
  assert.match(workflow, /test -f dist\/framer-next-linux-x64\.tar\.gz/);
  assert.doesNotMatch(workflow, /dist\/dist\//);
});
