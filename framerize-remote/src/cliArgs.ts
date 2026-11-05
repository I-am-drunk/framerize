import { RemoteDiagnosticError } from './diagnostics.js';

export type OptionSpec = Readonly<Record<string, 'string' | 'boolean'>>;

export interface ParsedArgs {
  readonly options: Readonly<Record<string, string | boolean>>;
}

export function parseArgs(argv: readonly string[], spec: OptionSpec): ParsedArgs {
  const options: Record<string, string | boolean> = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === undefined || !token.startsWith('--')) {
      throw new RemoteDiagnosticError('command_arguments', 'unexpected argument');
    }
    const name = token.slice(2);
    const kind = Object.hasOwn(spec, name) ? spec[name] : undefined;
    if (kind === undefined) throw new RemoteDiagnosticError('command_arguments', 'unknown option');
    if (Object.hasOwn(options, name)) throw new RemoteDiagnosticError('command_arguments', 'duplicate option');
    if (kind === 'boolean') { options[name] = true; continue; }
    const value = argv[index + 1];
    if (value === undefined || value === '' || value.startsWith('--')) {
      throw new RemoteDiagnosticError('command_arguments', 'option requires a value');
    }
    options[name] = value;
    index += 1;
  }
  return { options };
}
