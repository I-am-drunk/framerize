export class ProviderError extends Error {
  declare readonly status: number;
  declare readonly code: string;
  declare readonly retryable?: false;

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
