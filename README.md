# Framerize

Node.js tools for connecting local clients to an operator-authorized inference API.
Use Node.js 22.12 or newer. Run the commands below from this repository's root.

## Choose a package

- [Framerize server](framerize-server/README.md) exposes authenticated Responses
  and Chat Completions routes. Its default listener is `127.0.0.1:8789`.
- [Framerize Nexus](framerize-nexus/README.md) forwards local client requests to
  a configured endpoint. Its default listener is `127.0.0.1:8788`.
- [The separate experimental library](framerize-castle/README.md) is excluded
  from the server's package and native executable.

Nexus is not published to npm yet. Use the source commands here instead of
assuming that `npx framerize-nexus` installs this revision.

## Inspect the commands

```sh
node framerize-server/bin/framerize-server.js --help
node framerize-nexus/bin/framerize-nexus.js --help
node framerize-server/bin/framerize-server.js check
node framerize-nexus/bin/framerize-nexus.js check
```

`check` validates local configuration and files. It does not contact an upstream
or prove that inference or authentication works.

## Configure an endpoint

Nexus already reads `FRAMER_UPSTREAM`, `FRAMER_HOST`, `FRAMER_PORT`, and
`FRAMER_ACCOUNTS` from the process environment. Explicit CLI flags override them.
For an existing local endpoint on port 9000, a POSIX-shell example is:

```sh
export FRAMER_UPSTREAM=http://127.0.0.1:9000
node framerize-nexus/bin/framerize-nexus.js check
node framerize-nexus/bin/framerize-nexus.js serve
```

Use the endpoint base without `/v1`; Nexus appends `/v1/responses` or
`/v1/chat/completions`. This example does not start the upstream on port 9000.
Neither CLI automatically loads a `.env` file or named presets today.

The server currently takes its upstream and listener settings through flags.
It reads its client-facing token from `FRAMERIZE_SERVER_TOKEN` by default and
uses a separate upstream credential reference. Follow the
[server setup instructions](framerize-server/README.md#run) for those options.

## Current integration status

The server has fixture-tested npm and native packaging for Linux x64, macOS
arm64, and Windows x64. Nexus still retains its legacy account/provider options;
its planned remote-only authentication and readiness contract is separate work.
See the [server and Nexus contract](docs/framerize-server-plan.md).

For development checks, use each package's README. The
[two-package check](docs/package-coexistence.md) verifies installed package
coexistence. Fixture and installation results do not establish live inference
compatibility or publication of a release.
