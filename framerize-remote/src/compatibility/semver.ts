// Version comparison adapted from T3 Code packages/shared/src/semver.ts
// (normalizeSemverVersion, parseSemver, compareSemverVersions) at 5a906583 (MIT).
const NUMBER = /^\d+$/;

function parse(value: string) {
  const [noBuild = ''] = value.trim().replace(/^v/, '').split('+', 1);
  const [main = '', ...pre] = noBuild.split('-');
  const segments = main.split('.').map(part => part.trim()).filter(Boolean);
  while (segments.length > 0 && segments.length < 3) segments.push('0');
  if (segments.length !== 3 || !segments.every(part => NUMBER.test(part))) return null;
  const [major, minor, patch] = segments.map(Number) as [number, number, number];
  return { major, minor, patch, prerelease: pre.join('-').split('.').map(part => part.trim()).filter(Boolean) };
}

function compareIdentifier(left: string, right: string): number {
  if (NUMBER.test(left) && NUMBER.test(right)) return Number(left) - Number(right);
  if (NUMBER.test(left)) return -1;
  if (NUMBER.test(right)) return 1;
  return left === right ? 0 : left < right ? -1 : 1;
}

export function compareSemverVersions(left: string, right: string): number {
  const a = parse(left);
  const b = parse(right);
  if (!a || !b) return left.localeCompare(right);
  if (a.major !== b.major) return a.major - b.major;
  if (a.minor !== b.minor) return a.minor - b.minor;
  if (a.patch !== b.patch) return a.patch - b.patch;
  if (a.prerelease.length === 0 || b.prerelease.length === 0) return b.prerelease.length - a.prerelease.length;
  for (let index = 0; index < Math.max(a.prerelease.length, b.prerelease.length); index += 1) {
    const [l, r] = [a.prerelease[index], b.prerelease[index]];
    if (l === undefined) return -1;
    if (r === undefined) return 1;
    const order = compareIdentifier(l, r);
    if (order !== 0) return order;
  }
  return 0;
}
