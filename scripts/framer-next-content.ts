export const accountTemplate = JSON.stringify([{
  id: 'personal', enabled: false, projectId: 'replace-with-your-project-id',
  session: { file: 'private/session' },
}], null, 2) + '\n';

export function recoveryReadme(executable: string): string {
  const command = './' + executable;
  const options = ' --backend framer --accounts-file ./accounts.json --account-id personal'
    + ' --server-token-file ./private/server-token --host 127.0.0.1 --port 8789';
  return [
    '# Framer Next', '',
    'A compact recovery folder for the browserless Node server. It uses the',
    'existing server executable, not another backend. Node is embedded; no',
    'system Node installation, compiler or service manager is needed.', '',
    '1. Copy accounts.example.json to accounts.json. Set the real project ID',
    '   and enable only the account you intend to use.',
    '2. Create a private directory named private. Provision session and',
    '   server-token files through your secret manager. Never put their values',
    '   in command arguments. Use owner-only POSIX permissions or Windows ACLs.',
    '3. Open a POSIX shell or PowerShell in this folder. Check, then start:', '',
    '```sh', command + ' --help', command + ' check' + options,
    command + ' serve' + options, '```', '',
    'The listener binds only to 127.0.0.1:8789. Output goes to the terminal.',
    'Ctrl+C stops the server and waits for cleanup. No service is installed.',
    'Use a separately configured TLS/private-network endpoint for Remote.', '',
    'SHA256SUMS covers every supplied file. recovery.json records the original',
    'server commit, target, Node version and pinned native manifest hash.',
    'Compare that hash with your trusted source before executing this copy.',
    'The example account is disabled and contains no credentials.', '',
    'This folder preserves the source artifact qualification; creating it does',
    'not establish live compatibility or qualify a new release. Keep the native',
    'release evidence with the backup. Do not store credentials in the archive.',
    'Restore into a new directory rather than overwriting an active instance.', '',
  ].join('\n');
}
