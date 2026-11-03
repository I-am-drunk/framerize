export class ProviderError extends Error {
  constructor(message, { status = 502, code = 'upstream_error' } = {}) {
    super(message);
    this.name = 'ProviderError';
    this.status = status;
    this.code = code;
  }
}

export class ConfigurationError extends ProviderError {
  constructor(message) {
    super(message, { status: 500, code: 'configuration_error' });
    this.name = 'ConfigurationError';
  }
}
