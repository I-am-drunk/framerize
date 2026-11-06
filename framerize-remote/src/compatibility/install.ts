import type { ClientCommand, ClientName } from './clients.js';

/** Official native installers: code.claude.com/docs/en/setup, developers.openai.com/codex/cli. */
const SCRIPTS: Readonly<Record<ClientName, { readonly url: string; readonly shell: string; readonly windows: string }>> = {
  claude: { url: 'https://claude.ai/install.sh', shell: 'bash', windows: 'irm https://claude.ai/install.ps1 | iex' },
  codex: { url: 'https://chatgpt.com/codex/install.sh', shell: 'sh', windows: 'irm https://chatgpt.com/codex/install.ps1 | iex' },
};

export function installCommand(client: ClientName, platform: NodeJS.Platform): ClientCommand | null {
  const script = SCRIPTS[client];
  if (platform === 'win32') {
    return { command: 'powershell.exe', args: ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script.windows],
      display: `powershell -ExecutionPolicy ByPass -c "${script.windows}"` };
  }
  if (platform === 'darwin' || platform === 'linux') {
    const command = [
      'installer=$(mktemp "${TMPDIR:-/tmp}/framerize-install.XXXXXX") || exit "$?"',
      'trap \'rm -f "$installer"\' 0',
      'trap \'exit 1\' HUP INT TERM',
      `curl -fsSL ${script.url} > "$installer" || exit "$?"`,
      `${script.shell} < "$installer"`,
    ].join('\n');
    return { command: '/bin/sh', args: ['-c', command],
      display: `Download ${script.url}, then run ${script.shell}` };
  }
  return null;
}
