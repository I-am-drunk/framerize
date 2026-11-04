# Environment configuration

`loadConfig(values, env)` reads these optional environment settings for both
the CLI and library callers. Explicit options take precedence.

| Variable | Option | Default |
| --- | --- | --- |
| `FRAMERIZE_SERVER_BACKEND` | `backend`, `--backend` | `http` |
| `FRAMERIZE_SERVER_ACCOUNTS` | `accountsFile`, `--accounts-file` | empty |
| `FRAMERIZE_SERVER_ACCOUNT` | `accountId`, `--account-id` | empty, select the sole enabled account |
| `FRAMERIZE_SERVER_FRAMER_ORIGIN` | `framerOrigin`, `--framer-origin` | `https://api.framer.com` |
| `FRAMERIZE_SERVER_HOST` | `host`, `--host` | `127.0.0.1` |
| `FRAMERIZE_SERVER_PORT` | `port`, `--port` | `8789` |
| `FRAMERIZE_SERVER_UPSTREAM` | `upstream`, `--upstream` | empty, no HTTP provider |

Timeouts, body limits and logging use explicit options, not environment
settings. `FRAMERIZE_SERVER_TOKEN` supplies the server credential separately,
as described below.

## Precedence

1. An explicit value wins. That is a key present in `values`, or a CLI flag.
2. Otherwise a variable that is set, including an empty string, is used.
3. Otherwise the existing default applies.

An explicit value replaces the variable before validation, so an invalid
variable is ignored when the matching option is given.

`env` defaults to `process.env`. The CLI passes `dependencies.env` when a
test injects one. A library caller that omits `env` inherits these variables,
the same way it already inherits `FRAMERIZE_SERVER_TOKEN`. Pass `{}` as `env`
to use defaults only.

## Validation

Environment values go through the same checks as explicit values:

- `host` must be a non-empty string without whitespace, NUL or `/`.
- `port` must be decimal digits in the range 0 to 65535. `0` asks the
  operating system for a free port.
- `upstream` must be an `http:` or `https:` URL with no user name, password,
  query or fragment. An empty value disables the generic HTTP provider.

For the Framer backend:

- Set `backend` to `framer` and provide an `accountsFile` path. The default
  `http` backend uses the generic upstream instead.
- Leave `upstream`, `authorizationEnv` and `authorizationFile` empty.
  These generic upstream options cannot be combined with `framer`.
- `accountId` selects an enabled account. Omit it only when exactly one is
  enabled. It must start with a letter or digit and contain at most 64 letters,
  digits, periods, underscores or hyphens. It is rejected with `http`.
- `framerOrigin` must be an HTTPS origin with no credentials, query, fragment
  or non-root path. HTTP is accepted only for numeric loopback `127.0.0.1` or
  `[::1]`, for local fixtures. It does not disable certificate checks.

Account paths must contain no NUL. Reading and validating the account file
happens when the backend starts, not in `loadConfig`. See the
[account format](framer-backend.md#select-a-json-account).

Invalid settings raise `ConfigurationError`. A rejected environment port
reports `Invalid FRAMERIZE_SERVER_PORT`; an invalid explicit port reports
`Invalid port`. Other errors can name an option or an incompatible combination,
such as `Invalid Framer backend configuration`. Messages never echo supplied
values, including paths, URLs or credentials.

## Credential boundary

The settings in the table carry no credentials. The server token comes from
`serverTokenEnv` (default `FRAMERIZE_SERVER_TOKEN`) or `serverTokenFile`.
Generic HTTP upstream credentials come from `authorizationEnv` or
`authorizationFile`, never both. Framer sessions use private environment or
file references in the account JSON. Do not put credentials in that JSON,
command arguments or either upstream URL. URL validation rejects embedded
user info, queries and fragments.
