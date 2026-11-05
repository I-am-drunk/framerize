import type { ClientCommand, ClientName } from './clients.js';

/** Official native installers: code.claude.com/docs/en/setup, developers.openai.com/codex/cli. */
const SCRIPTS: Readonly<Record<ClientName, { readonly posix: string; readonly windows: string }>> = {
  claude: { posix: 'curl -fsSL https://claude.ai/install.sh | bash', windows: 'irm https://claude.ai/install.ps1 | iex' },
  codex: { posix: 'curl -fsSL https://chatgpt.com/codex/install.sh | sh', windows: 'irm https://chatgpt.com/codex/install.ps1 | iex' },
};

export function installCommand(client: ClientName, platform: NodeJS.Platform): ClientCommand | null {
  const script = SCRIPTS[client];
  if (platform === 'win32') {
    return { command: 'powershell.exe', args: ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script.windows],
      display: `powershell -ExecutionPolicy ByPass -c "${script.windows}"` };
  }
  if (platform === 'darwin' || platform === 'linux') {
    return { command: '/bin/sh', args: ['-c', script.posix], display: script.posix };
  }
  return null;
}
