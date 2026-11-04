import type { BackendReadiness } from '@framerize/core';

export const VERSION = '0.2.0';

export const SERVE_HELP = `Usage: framerize-remote serve [options]

  --config FILE              Use a saved profile
  --host ADDRESS             Override the listen host for this run
  --port NUMBER              Override the port, or use 0 for an assigned port
  --allow-non-loopback        Allow listening outside loopback for this run
  --json                     Emit machine-readable JSON

Overrides never change the saved profile. A non-loopback --host requires
--allow-non-loopback on the same command. Without --host, saved consent applies.
Local bearer authentication remains required. Network routing and TLS are
operator-managed. Binding an address does not prove internet reachability.
`;

export const SETUP_HELP = `Usage: framerize-remote setup --remote ORIGIN [options]

  --server-token-env NAME | --server-token-file FILE
  --local-token-env NAME  | --local-token-file FILE
  --config FILE              Override the saved profile path
  --start                    Save, check the remote, and start the local API
  --host ADDRESS             Listen host, default 127.0.0.1
  --port NUMBER              Listen port, default 8788, or 0 for an assigned port
  --inference-timeout-ms N    Inference response wait, default 300000
  --allow-non-loopback        Allow listening outside loopback
  --allow-insecure-remote     Allow HTTP outside numeric loopback

Use one credential reference per role. Never pass token values as arguments.
Profiles default to ~/.framerize-remote/framerize-remote.json.
FRAMERIZE_REMOTE_HOME overrides that directory. Use an absolute path.
Use --start for first run, or framerize-remote serve to start a saved profile.
`;

export const HELP = `framerize-remote ${VERSION}

A local Framerize instance. It exposes the Framerize client API on loopback and
forwards inference to one configured remote server.

Usage:
  framerize-remote <command> [options]

Commands:
  setup     Write a remote profile (remote URL and credential references)
  check     Report authenticated readiness of the configured remote
  serve     Start the local API and forward inference to the remote
  help      Show this help
  version   Print the version

Common options:
  --config FILE   Override ~/.framerize-remote/framerize-remote.json
  --json          Emit machine-readable JSON

Check options:
  --tailscale     Also inspect local Tailscale status without changing it

Tailscale status is advisory. It does not prove remote reachability.

Run \`framerize-remote setup --help\` or \`framerize-remote serve --help\` for options.
`;

export function readinessLine(result: BackendReadiness): { ok: boolean; text: string } {
  if (result === true) return { ok: true, text: 'Remote readiness passed.' };
  const reasons = { upstream_unavailable: 'remote upstream unavailable',
    credentials_unavailable: 'remote credentials unavailable', configuration_unavailable: 'remote configuration unavailable' };
  return { ok: false, text: `Not ready: ${reasons[result.reason]}.\nNext: check the server, then run framerize-remote check.` };
}
