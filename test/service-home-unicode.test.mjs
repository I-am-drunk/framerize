import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build, version as esbuildVersion } from 'esbuild';

const digest = value => createHash('sha256').update(value).digest('hex');
for (const [name, relative] of [
  ['Remote', '../framerize-remote/src/serviceUnits.ts'],
  ['Server', '../framerize-server/src/service-units.ts'],
]) {
  const source = fileURLToPath(new URL(relative, import.meta.url));
  const result = await build({ entryPoints: [source], bundle: true, platform: 'node',
    format: 'esm', write: false });
  const bytes = result.outputFiles[0].contents;
  const service = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
  console.log(JSON.stringify({ evidence: 'source-function round trips, no service execution',
    runtime: process.version, name, source: digest(await readFile(source)),
    bundle: digest(bytes), esbuildVersion }));
  for (const point of [0x2028, 0x2029]) {
    test(`${name} preserves U+${point.toString(16)} inside the service home`, { timeout: 5000 }, context => {
      const home = `/synthetic/first${String.fromCodePoint(point)}last`;
      const plan = { home, program: ['/bin/true'], logPath: `${home}/service.log`,
        homeDir: '/synthetic', environmentPath: '/usr/bin', platform: 'linux', uid: 1000 };
      const unit = service.renderServiceUnit(plan);
      const plist = service.renderServicePlist(plan, plan);
      assert.ok(unit.includes(home));
      assert.equal(service.serviceHomeOf(plist), home, 'launchd round trip control');
      const actual = service.serviceHomeOf(unit);
      context.diagnostic(JSON.stringify({ name, point, expectedLength: home.length,
        actualLength: actual?.length, plistRoundTrip: true, systemdRoundTrip: actual === home }));
      assert.equal(actual, home, 'a generated systemd unit must retain its service-home identity');
    });
  }
}
