import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';

for (const name of ['exec', 'execSync', 'execFile', 'execFileSync', 'spawn', 'spawnSync', 'fork']) {
  childProcess[name] = () => {
    process.stderr.write('Unexpected child process during profile validation\n');
    throw new Error('Profile validation must not run a service command');
  };
}
syncBuiltinESMExports();
