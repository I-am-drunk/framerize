import { execFileSync } from 'node:child_process';
import { copyFileSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';

const platform = process.platform;
const architecture = process.arch;
if (!['linux', 'darwin', 'win32'].includes(platform)) {
  throw new Error(`unsupported release platform: ${platform}`);
}
const suffix = `${platform === 'darwin' ? 'macos' : platform}-${architecture}`;
const executable = `framer-next${platform === 'win32' ? '.exe' : ''}`;
const archive = `dist/framer-next-${suffix}.${platform === 'win32' ? 'zip' : 'tar.gz'}`;
const license = path.join(path.dirname(process.execPath), '..', 'LICENSE');
copyFileSync(license, 'dist/NODE-LICENSE');
if (platform === 'win32') {
  execFileSync('powershell', ['-NoLogo', '-NoProfile', '-Command',
    `Compress-Archive -LiteralPath 'dist/${executable}', 'dist/NODE-LICENSE' -DestinationPath '${archive}' -Force`],
  { stdio: 'inherit' });
} else {
  execFileSync('tar', ['-czf', archive, '-C', 'dist', executable, 'NODE-LICENSE'],
    { stdio: 'inherit' });
}
const digest = (file) => createHash('sha256')
  .update(readFileSync(file)).digest('hex');
const existing = readFileSync('dist/SHA256SUMS', 'utf8').trim();
const entries = [`dist/${executable}`, archive].map((file) =>
  `${digest(file)}  ${file.replace(/^dist\//, '')}`);
writeFileSync('dist/SHA256SUMS', `${[existing, ...entries].filter(Boolean).join('\n')}\n`);
console.log(`Packaged ${archive}`);
