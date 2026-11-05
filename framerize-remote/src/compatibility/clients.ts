export type ClientName = 'claude' | 'codex';

/** A process to run with inherited stdio, plus the text shown before asking. */
export interface ClientCommand {
  readonly command: string;
  readonly args: readonly string[];
  readonly display: string;
  readonly env?: Readonly<Record<string, string>>;
}

export interface ClientDefinition {
  readonly label: string;
  readonly command: string;
  readonly npmPackage: string;
  readonly docs: string;
}

export const CLIENTS: Readonly<Record<ClientName, ClientDefinition>> = {
  claude: { label: 'Claude Code', command: 'claude', npmPackage: '@anthropic-ai/claude-code',
    docs: 'https://code.claude.com/docs/en/setup' },
  codex: { label: 'Codex', command: 'codex', npmPackage: '@openai/codex',
    docs: 'https://developers.openai.com/codex/cli' },
};

export function isClientName(value: unknown): value is ClientName {
  return value === 'claude' || value === 'codex';
}
