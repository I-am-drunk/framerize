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
| `FRAMERIZE_SERVER_TOKEN_ENV` | `serverTokenEnv`, `--server-token-env` | `FRAMERIZE_SERVER_TOKEN` |
| `FRAMERIZE_SERVER_TOKEN_FILE` | `serverTokenFile`, `--server-token-file` | empty |
| `FRAMERIZE_SERVER_AUTHORIZATION_ENV` | `authorizationEnv`, `--authorization-env` | empty |
| `FRAMERIZE_SERVER_AUTHORIZATION_FILE` | `authorizationFile`, `--authorization-file` | empty |
| `FRAMERIZE_SERVER_LOG_LEVEL` | `logLevel`, `--log-level` | `info` |
| `FRAMERIZE_SERVER_MAX_BODY_BYTES` | `maxBodyBytes`, `--max-body-bytes` | `16777216` |
| `FRAMERIZE_SERVER_HEADERS_TIMEOUT_MS` | `headersTimeoutMs`, `--headers-timeout-ms` | `15000` |
| `FRAMERIZE_SERVER_INFERENCE_TIMEOUT_MS` | `inferenceTimeoutMs`, `--inference-timeout-ms` | `300000` |
| `FRAMERIZE_SERVER_IDLE_TIMEOUT_MS` | `idleTimeoutMs`, `--idle-timeout-ms` | `60000` |
| `FRAMERIZE_SERVER_REQUEST_HEADERS_TIMEOUT_MS` | `requestHeadersTimeoutMs`, `--request-headers-timeout-ms` | `15000` |
| `FRAMERIZE_SERVER_SHUTDOWN_TIMEOUT_MS` | `shutdownTimeoutMs`, `--shutdown-timeout-ms` | `5000` |

`FRAMERIZE_SERVER_TOKEN` itself holds the API token value, as described below.

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
- `maxBodyBytes` and each timeout must be decimal digits from 1 to 2147483647.
- `logLevel` must be `debug`, `info`, `warn`, `error` or `silent`.
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
`Invalid port`. Numeric settings, host, backend, account and origin follow the
same pattern. Other errors can name an option or an incompatible combination,
such as `Invalid Framer backend configuration`. Messages do not include the
supplied values.

## API token

Without a token, the server serves `/ready` and every inference route with no
`Authorization` header. Use this only on a listener that trusted clients alone
can reach, such as loopback or a private network.

Set `FRAMERIZE_SERVER_TOKEN` to require `Authorization: Bearer <token>`.
`FRAMERIZE_SERVER_TOKEN_ENV` or `--server-token-env` names a different variable.
`FRAMERIZE_SERVER_TOKEN_FILE` or `--server-token-file` reads the token from a
regular file of at most 16 KiB instead. On Unix, group and other permission
bits must be unset. The server reads the token once at startup.
An unset or empty `FRAMERIZE_SERVER_TOKEN` counts as no token. A variable named
by `--server-token-env` or `FRAMERIZE_SERVER_TOKEN_ENV` must be set and non-empty.
A command-line token source replaces both token environment settings.
Naming a token variable and a token file at the same level fails startup.

## Upstream credentials

Generic HTTP upstream credentials come from `authorizationEnv` or
`authorizationFile`, never both. Framer sessions come from the account JSON.
A session can be written inline or referenced by environment variable or file.
See the [account format](framer-backend.md#select-a-json-account). Upstream
URL validation rejects embedded user info, queries and fragments.
