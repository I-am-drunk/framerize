export class ProviderError extends Error {
  declare readonly status: number;
  declare readonly code: string;

  constructor(message: string, { status = 502, code = 'upstream_error' }:
    { status?: number; code?: string } = {}) {
    super(message);
    this.name = 'ProviderError';
    this.status = status;
    this.code = code;
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
