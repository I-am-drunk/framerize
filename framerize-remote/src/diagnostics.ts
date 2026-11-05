const messages = {
  command_arguments: ['The command options are invalid, incomplete or repeated.', 'Run framerize-remote help for supported commands and options.'],
  setup_arguments: ['The setup options are invalid, incomplete or repeated.', 'Run framerize-remote setup --help for supported options.'],
  remote_required: ['The remote server address is missing.', 'Run frm up with the pairing link from framerize-server pair, or set FRM_REMOTE.'],
  remote_invalid: ['The remote server address is invalid or uses unapproved HTTP.', 'Use an HTTPS origin without a path, query or credentials. Local HTTP requires numeric loopback.'],
  server_reference: ['The server token reference is missing or invalid.', 'Use exactly one valid --server-token-file path or --server-token-env variable name.'],
  local_reference: ['The local token reference is missing or invalid.', 'Use exactly one valid --local-token-file path or --local-token-env variable name.'],
  inference_invalid: ['The inference timeout is invalid.', 'Use integer milliseconds between 1 and 2147463647 with --inference-timeout-ms.'],
  profile_write: ['The profile could not be saved.', 'Check the profile directory permissions and that the profile path is not a directory.'],
  listen_invalid: ['The listen address or port is invalid.', 'Use a valid host and a port between 0 and 65535.'],
  listen_failed: ['The local API could not listen on the requested address.', 'Choose a free local port and a host assigned to this machine.'],
  profile_missing: ['No saved profile or FRM_REMOTE was found.', 'Run frm setup, or set FRM_REMOTE to the server origin.'],
  profile_home_invalid: ['FRM_HOME, or FRAMERIZE_REMOTE_HOME when FRM_HOME is unset, must be an absolute directory path without control characters.',
    'Set FRM_HOME to an absolute directory without control characters, or unset both home variables to use the default.'],
  pairing_link_invalid: ['The pairing link is invalid.', 'Copy the whole link from framerize-server pair, or pass the server origin instead.'],
  service_unsupported: ['The background service needs Linux with systemd or macOS with launchd.', 'Run frm in the foreground instead.'],
  service_user_manager: ['Cannot reach the systemd user manager.', 'Run `systemctl --user status` in a login session for this user. Do not run frm with sudo.'],
  service_linger: ['Lingering is disabled, so the frm service would stop when your last login session ends.', 'Run `sudo loginctl enable-linger "$(id -un)"`, then run frm up again as your normal user.'],
  service_linger_unavailable: ['Cannot check whether this user can run services after logout.', 'Run `loginctl show-user "$(id -un)" --property=Linger` and check that systemd-logind is available.'],
  service_failed: ['The background service command failed.', 'Run frm service status and check the service log.'],
  service_cleanup_failed: ['Could not stop background service command processes. Some may still be running.',
    'Inspect the service manager and its command processes before retrying frm up or frm service.'],
  service_port_busy: ['Another program already listens on the local API port.', 'Stop that program, or run frm up again with FRM_PORT set to a free port.'],
  service_not_ready: ['The frm service did not start its local API.', 'Run frm service status and check frm-service.log in FRM_HOME.'],
  service_rollback_failed: ['Setup failed and the previous profile or service state could not be fully restored.', 'Check the profile in FRM_HOME and run frm service status before retrying frm up.'],
  service_other_home: ['The installed frm service serves another FRM_HOME.', 'Set FRM_HOME to that directory, or run frm service uninstall from there first.'],
  service_env_token: ['The background service cannot read tokens from your shell environment.', 'Run frm up with the pairing link, which saves the server token in the profile, or save token files with frm setup.'],
  profile_invalid: ['The saved profile is invalid or unreadable.', 'Repair the profile or run framerize-remote setup --help.'],
  server_credential: ['The server token is missing or invalid.', 'Check the server-token file or environment reference.'],
  local_credential: ['The local token is missing or invalid.', 'Check the local-token file or environment reference.'],
  dns_failed: ['The remote hostname could not be resolved.', 'Check the remote hostname and DNS connection.'],
  connection_failed: ['The remote connection failed.', 'Check the server address, port and network connection.'],
  tls_failed: ['Remote TLS verification or negotiation failed.', 'Repair the server certificate or HTTPS configuration.'],
  remote_auth: ['The remote server rejected the request authentication.', 'Set FRM_SERVER_TOKEN or save a server token reference if the server requires one.'],
  http_refused: ['The remote refused the readiness request.', 'Check that the origin points to a Framerize server.'],
  headers_timeout: ['The remote did not send headers in time.', 'Check server health and network reachability.'],
  idle_timeout: ['The remote readiness response stalled.', 'Check server health and network stability.'],
  readiness_timeout: ['The remote readiness check exceeded its deadline.', 'Check server health and network stability.'],
  response_too_large: ['The remote readiness response exceeds its limit.', 'Check that the origin points to a Framerize server.'],
  malformed_response: ['The remote returned invalid readiness data.', 'Check the server readiness endpoint.'],
  incompatible_response: ['The remote readiness contract is incompatible.', 'Use compatible Remote and server versions.'],
  aborted: ['The command was cancelled.', 'Run the command again to retry.'],
  configuration_unavailable: ['The remote configuration is unavailable.', 'Check the server configuration.'],
  credentials_unavailable: ['The remote provider credentials are unavailable.', 'Check credentials on the server.'],
  upstream_unavailable: ['The remote upstream is unavailable.', 'Check the provider connection on the server.'],
  cleanup_failed: ['Remote cleanup failed.', 'Confirm the Remote process has exited, then restart it with the same profile.'],
} as const;

export type DiagnosticCode = keyof typeof messages;

export function diagnostic(code: DiagnosticCode) {
  const [message, next] = messages[code];
  return { code, message, next };
}

export function diagnosticText(code: DiagnosticCode): string {
  const detail = diagnostic(code);
  const message = code === 'aborted' || code === 'cleanup_failed'
    || code === 'command_arguments' || code.startsWith('service_')
    ? detail.message : `Not ready: ${detail.message}`;
  return `${message}\nNext: ${detail.next}`;
}

export function commandFailure(error: unknown, json: boolean): string | undefined {
  if (error instanceof Error && error.name === 'AbortError') error = new RemoteDiagnosticError('aborted');
  if (!(error instanceof RemoteDiagnosticError)) return undefined;
  const setup = error instanceof SetupRollbackError ? diagnostic(error.setupCode) : undefined;
  const cleanup = error instanceof SetupRollbackError && error.cleanupCode
    ? diagnostic(error.cleanupCode) : undefined;
  return json ? JSON.stringify({ error: error.code, ...diagnostic(error.code),
    ...(setup ? { setup } : {}), ...(cleanup ? { cleanup } : {}) })
    : `${setup ? `Original failure: ${setup.message}\n` : ''}${diagnosticText(error.code)}`
      + (cleanup ? `\n${diagnosticText(cleanup.code)}` : '');
}

export class RemoteDiagnosticError extends Error {
  constructor(readonly code: DiagnosticCode, message: string = messages[code][0]) {
    super(message);
  }
}

export class ServiceCommandCleanupError extends RemoteDiagnosticError {
  constructor(cause: unknown) {
    super('service_cleanup_failed');
    this.cause = cause;
  }
}

export class SetupRollbackError extends RemoteDiagnosticError {
  readonly setupCode: DiagnosticCode;
  readonly cleanupCode: DiagnosticCode | undefined;

  constructor(error: unknown, failures: readonly unknown[]) {
    super('service_rollback_failed');
    this.setupCode = error instanceof RemoteDiagnosticError ? error.code : 'service_failed';
    this.cleanupCode = failures.some(failure => failure instanceof ServiceCommandCleanupError)
      ? 'service_cleanup_failed' : undefined;
    this.cause = new AggregateError([error, ...failures], this.message);
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
