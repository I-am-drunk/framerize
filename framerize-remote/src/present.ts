import type { BackendReadiness } from '@framerize/core';
import packageJson from '../package.json' with { type: 'json' };

export const VERSION = packageJson.version;

export const SERVE_HELP = `Usage: frm serve [options]

  --config FILE              Use a saved profile
  --host ADDRESS             Override the listen host for this run
  --port NUMBER              Override the port, or use 0 for an assigned port
  --allow-non-loopback        Accepted for older profiles; no longer required
  --json                     Emit machine-readable JSON

Overrides never change the saved profile. FRM_REMOTE, FRM_HOST and FRM_PORT
also apply for one run. Without a local token, clients connect without a bearer
token. Binding an address does not prove reachability from other devices.
`;

export const SETUP_HELP = `Usage: frm setup [--remote ORIGIN] [options]

  --server-token-env NAME | --server-token-file FILE
  --local-token-env NAME  | --local-token-file FILE
  --config FILE              Override the saved profile path
  --start                    Save, check the remote, and start the local API
  --host ADDRESS             Listen host, default 127.0.0.1
  --port NUMBER              Listen port, default 8788, or 0 for an assigned port
  --inference-timeout-ms N    Inference response wait, default 300000
  --allow-non-loopback        Accepted for older profiles; no longer required
  --allow-insecure-remote     Allow HTTP to a public address

Token references are optional. Use at most one per role. Never pass token values
as arguments. In a terminal, setup asks for a missing server address. Otherwise
it reads FRM_REMOTE. HTTP needs no consent for loopback, LAN or Tailscale servers.
Profiles default to ~/.framerize-remote/framerize-remote.json.
FRM_HOME overrides that directory. FRAMERIZE_REMOTE_HOME is the fallback
when FRM_HOME is unset. Use an absolute path without control characters.
Use --start for first run, or frm serve to start a saved profile.
`;

export const HELP = `frm ${VERSION}

A local Framerize instance. It exposes the Framerize client API on loopback and
forwards inference to one configured remote server.

Usage:
  frm <command> [options]

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

Run \`frm setup --help\` or \`frm serve --help\` for options.
`;

export function readinessLine(result: BackendReadiness): { ok: boolean; text: string } {
  if (result === true) return { ok: true, text: 'Remote readiness passed.' };
  const reasons = { upstream_unavailable: 'remote upstream unavailable',
    credentials_unavailable: 'remote credentials unavailable', configuration_unavailable: 'remote configuration unavailable' };
  return { ok: false, text: `Not ready: ${reasons[result.reason]}.\nNext: check the server, then run frm check.` };
}

export const FRM_HELP = `frm ${VERSION}

Usage:
  frm                 Start the saved profile or FRM_REMOTE; on first run, connect interactively
  frm connect [LINK]  Pair with a server link or address, then install and start the service
  frm up [LINK]       Same as frm connect LINK
  frm LINK            Same as frm connect LINK
  frm setup           Save a profile; asks for missing values in a terminal
  frm serve           Start in the foreground
  frm check           Check the remote server
  frm client          Set up or launch Claude Code or Codex
  frm service install|status|start|stop|restart|uninstall

Environment:
  FRM_REMOTE          Server origin, such as http://100.64.0.1:8789
  FRM_HOME            Profile directory, default ~/.framerize-remote
  FRM_HOST, FRM_PORT  Local listen address, default 127.0.0.1:8788
  FRM_SERVER_TOKEN    Optional token sent to the server
  FRM_LOCAL_TOKEN     Optional token local clients must send

Environment values are used for one run and are not saved. The service runs the
saved profile in FRM_HOME. A pairing link saves its server token in that profile.
`;
