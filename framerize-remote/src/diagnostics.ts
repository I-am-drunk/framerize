const messages = {
  setup_arguments: ['The setup options are invalid, incomplete or repeated.', 'Run framerize-remote setup --help for supported options.'],
  remote_required: ['The remote server address is missing.', 'Add --remote with your server origin, such as https://server.example.'],
  remote_invalid: ['The remote server address is invalid or uses unapproved HTTP.', 'Use an HTTPS origin without a path, query or credentials. Local HTTP requires numeric loopback.'],
  server_reference: ['The server token reference is missing or invalid.', 'Use exactly one valid --server-token-file path or --server-token-env variable name.'],
  local_reference: ['The local token reference is missing or invalid.', 'Use exactly one valid --local-token-file path or --local-token-env variable name.'],
  inference_invalid: ['The inference timeout is invalid.', 'Use integer milliseconds between 1 and 2147463647 with --inference-timeout-ms.'],
  profile_write: ['The profile could not be saved.', 'Check the profile directory permissions and that the profile path is not a directory.'],
  listen_invalid: ['The listen address or port is invalid.', 'Use a valid host and a port between 0 and 65535.'],
  listen_failed: ['The local API could not listen on the requested address.', 'Choose a free local port and a host assigned to this machine.'],
  exposure_consent: ['Listening outside loopback requires explicit consent.', 'Add --allow-non-loopback or use --host 127.0.0.1.'],
  profile_missing: ['No saved profile was found.', 'Run framerize-remote setup --help.'],
  profile_invalid: ['The saved profile is invalid or unreadable.', 'Repair the profile or run framerize-remote setup --help.'],
  server_credential: ['The server token is missing or invalid.', 'Check the server-token file or environment reference.'],
  local_credential: ['The local token is missing or invalid.', 'Check the local-token file or environment reference.'],
  dns_failed: ['The remote hostname could not be resolved.', 'Check the remote hostname and DNS connection.'],
  connection_failed: ['The remote connection failed.', 'Check the server address, port and network connection.'],
  tls_failed: ['Remote TLS verification or negotiation failed.', 'Repair the server certificate or HTTPS configuration.'],
  remote_auth: ['The remote server rejected the server token.', 'Check the server-token reference and server authentication.'],
  http_refused: ['The remote refused the readiness request.', 'Check that the origin points to a Framerize server.'],
  headers_timeout: ['The remote did not send headers in time.', 'Check server health and network reachability.'],
  idle_timeout: ['The remote readiness response stalled.', 'Check server health and network stability.'],
  readiness_timeout: ['The remote readiness check exceeded its deadline.', 'Check server health and network stability.'],
  response_too_large: ['The remote readiness response exceeds its limit.', 'Check that the origin points to a Framerize server.'],
  malformed_response: ['The remote returned invalid readiness data.', 'Check the server readiness endpoint.'],
  incompatible_response: ['The remote readiness contract is incompatible.', 'Use compatible Remote and server versions.'],
  aborted: ['The readiness check was cancelled.', 'Run framerize-remote check to try again.'],
  configuration_unavailable: ['The remote configuration is unavailable.', 'Check the server configuration.'],
  credentials_unavailable: ['The remote provider credentials are unavailable.', 'Check credentials on the server.'],
  upstream_unavailable: ['The remote upstream is unavailable.', 'Check the provider connection on the server.'],
} as const;

export type DiagnosticCode = keyof typeof messages;

export function diagnostic(code: DiagnosticCode) {
  const [message, next] = messages[code];
  return { code, message, next };
}

export function diagnosticText(code: DiagnosticCode): string {
  const detail = diagnostic(code);
  return `Not ready: ${detail.message}\nNext: ${detail.next}`;
}

export function commandFailure(error: unknown, json: boolean): string | undefined {
  if (!(error instanceof RemoteDiagnosticError)) return undefined;
  return json ? JSON.stringify({ error: error.code, ...diagnostic(error.code) })
    : diagnosticText(error.code);
}

export class RemoteDiagnosticError extends Error {
  constructor(readonly code: DiagnosticCode, message: string = messages[code][0]) {
    super(message);
  }
}

export function networkDiagnostic(error: unknown): DiagnosticCode {
  try {
    if (error instanceof RemoteDiagnosticError) return error.code;
    const code: unknown = (error as NodeJS.ErrnoException | undefined)?.code;
    if (code === 'ENOTFOUND' || code === 'EAI_AGAIN') return 'dns_failed';
    if (code === 'HPE_HEADER_OVERFLOW') return 'response_too_large';
    if (typeof code === 'string' && (/^ERR_(TLS|SSL)_/.test(code) || [
      'CERT_HAS_EXPIRED', 'CERT_NOT_YET_VALID', 'DEPTH_ZERO_SELF_SIGNED_CERT',
      'SELF_SIGNED_CERT_IN_CHAIN', 'UNABLE_TO_VERIFY_LEAF_SIGNATURE',
      'UNABLE_TO_GET_ISSUER_CERT', 'UNABLE_TO_GET_ISSUER_CERT_LOCALLY',
      'CERT_REVOKED', 'INVALID_CA', 'EPROTO',
    ].includes(code))) return 'tls_failed';
  } catch {}
  return 'connection_failed';
}
