const errors = {
  not_found: [404, 'Not found'],
  method_not_allowed: [405, 'Method not allowed'],
  unauthorized: [401, 'Authentication required'],
  invalid_json: [400, 'Invalid JSON body'],
  invalid_request_body: [400, 'Invalid inference request'],
  body_too_large: [413, 'Request body too large'],
  headers_timeout: [504, 'Upstream headers timed out'],
  idle_timeout: [504, 'Request timed out'],
  request_timeout: [408, 'Request timed out'],
  server_closing: [503, 'Server is closing'],
  upstream_error: [502, 'Upstream request failed'],
  upstream_http_error: [502, 'Upstream request failed'],
  upstream_response_invalid: [502, 'Upstream returned an invalid completed reply'],
  provider_unavailable: [503, 'Provider unavailable'],
  provider_closed: [503, 'Provider unavailable'],
  credentials_unavailable: [503, 'Provider unavailable'],
  configuration_error: [500, 'Provider unavailable'],
  client_disconnected: [499, 'Client disconnected'],
} as const;

export type ErrorCode = keyof typeof errors;

export class HttpError extends Error {
  readonly status: number;
  constructor(readonly code: ErrorCode, status: number = errors[code][0],
    readonly retryable?: false) {
    super(errors[code][1]);
    this.status = status;
  }
}

export function publicError(error: unknown): HttpError {
  try {
    const source = error as { code?: unknown; status?: unknown; retryable?: unknown } | undefined;
    const code = source?.code;
    const status = source?.status;
    const known = typeof code === 'string' && Object.hasOwn(errors, code)
      ? code as ErrorCode : 'upstream_error';
    return new HttpError(known, typeof status === 'number' && Number.isInteger(status)
      && status >= 400 && status <= 599 ? status : errors[known][0],
    source?.retryable === false ? false : undefined);
  } catch { return new HttpError('upstream_error'); }
}
