import { isProxy } from 'node:util/types';

const mintRejectionCodes = ['configuration', 'busy', 'closed', 'aborted', 'timeout', 'collection', 'encoder'] as const;
type MintFailureDiagnostic = Readonly<{
  stage: 'mint';
  code: (typeof mintRejectionCodes)[number] | 'unknown';
}>;

export function mintFailureDiagnostic(rejection: unknown): MintFailureDiagnostic {
  let candidate: unknown;
  try {
    if (rejection !== null && typeof rejection === 'object' && !isProxy(rejection)) {
      const property = Object.getOwnPropertyDescriptor(rejection, 'code');
      if (property && Object.hasOwn(property, 'value')) candidate = property.value;
    }
  } catch { candidate = undefined; }
  const code = mintRejectionCodes.find(value => value === candidate) ?? 'unknown';
  return Object.freeze({ stage: 'mint', code });
}

export class ProviderError extends Error {
  declare readonly status: number;
  declare readonly code: string;
  declare readonly retryable?: false;
  declare readonly diagnostic?: MintFailureDiagnostic;

  constructor(message: string, { status = 502, code = 'upstream_error', retryable }:
    { status?: number; code?: string; retryable?: false } = {}) {
    super(message);
    this.name = 'ProviderError';
    this.status = status;
    this.code = code;
    if (retryable === false) this.retryable = false;
  }
}

export class ConfigurationError extends ProviderError {
  constructor(message: string) {
    super(message, { status: 500, code: 'configuration_error' });
    this.name = 'ConfigurationError';
  }
}

export class ServerTokenFileError extends ConfigurationError {
  constructor() {
    super('Server token file unavailable');
  }
}

export class AccountFileError extends ConfigurationError {}

export class AccountSelectionError extends ConfigurationError {}

export class ServerListenError extends Error {
  constructor() {
    super('Server listen failed');
    this.name = 'ServerListenError';
  }
}
