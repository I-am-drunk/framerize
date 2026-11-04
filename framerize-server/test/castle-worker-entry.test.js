import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';

test('embedded worker modules avoid top-level await during termination', () => {
  const source = readFileSync(new URL('../dist/castle.js', import.meta.url), 'utf8');
  const workers = [...source.matchAll(/data:text\/javascript;base64,([A-Za-z0-9+/]+={0,2})/g)];
  assert.equal(workers.length, 3);
  for (const [index, match] of workers.entries()) {
    const code = Buffer.from(match[1], 'base64').toString('utf8');
    const parsed = ts.createSourceFile(`worker-${index}.mjs`, code, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
    const awaits = [];
    const visit = node => {
      if (ts.isFunctionLike(node)) return;
      if (ts.isAwaitExpression(node) || ts.isForOfStatement(node) && node.awaitModifier)
        awaits.push(node.getText(parsed));
      ts.forEachChild(node, visit);
    };
    visit(parsed);
    assert.deepEqual(awaits, [], `Worker ${index} has top-level await`);
  }
});
