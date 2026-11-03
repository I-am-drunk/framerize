# Environment configuration

The server reads three optional listen and upstream settings from the
environment. `loadConfig(values, env)` applies them, so the CLI and library
callers share one rule.

| Variable | Option | Default |
| --- | --- | --- |
| `FRAMERIZE_SERVER_HOST` | `host`, `--host` | `127.0.0.1` |
| `FRAMERIZE_SERVER_PORT` | `port`, `--port` | `8789` |
| `FRAMERIZE_SERVER_UPSTREAM` | `upstream`, `--upstream` | empty, no provider |

No other variable selects a setting. `FRAMERIZE_SERVER_TOKEN` stays a token
reference, described below.

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
  query or fragment. It is stored in normalized form.

A rejected variable raises `ConfigurationError` with the message
`Invalid <VARIABLE_NAME>`, for example `Invalid FRAMERIZE_SERVER_PORT`.
A rejected explicit value keeps the existing message, such as `Invalid port`.
The message never contains the raw value, so an upstream URL that embeds a
password or token does not reach logs.

## Credential boundary

These variables carry no credentials. The server token still comes from
`serverTokenEnv` (default `FRAMERIZE_SERVER_TOKEN`) or `serverTokenFile`.
Upstream credentials still come from `authorizationEnv` or
`authorizationFile`, never both. `FRAMERIZE_SERVER_UPSTREAM` must not carry
credentials. The URL checks above reject embedded user info, queries and
fragments.
