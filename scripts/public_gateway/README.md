# Public inference proxy

This optional proxy listens on `127.0.0.1:8790` and forwards inference to the
existing Framerize service on `127.0.0.1:8787`. It requires no client password.
Anyone with network access can submit requests using the service's accounts.

The root accepts both dialects. OpenAI clients may also use `/openai/v1` as
their base URL; Anthropic clients may use `/anthropic`. The proxy supports
Responses, Chat Completions, Messages, model discovery and `/health`.

Management paths are unavailable. The proxy discards account-selection,
authorization, cookie and session headers, including private response headers.
It forwards SSE as received, closes upstream work when clients disconnect,
and limits public traffic to two concurrent requests and 16 MiB request bodies.
Existing backend admission and account restrictions still apply.

1. Copy `gateway.mjs` to `~/.local/share/framerize-public/gateway.mjs`.
2. Install `framerize-public.service` under `~/.config/systemd/user/`.
   Adjust its Node executable path if the installed version differs.
3. Run `systemctl --user daemon-reload` and
   `systemctl --user enable --now framerize-public.service`.
4. After enabling Tailscale Funnel for the machine, run
   `sudo tailscale funnel --bg --https=10000 http://127.0.0.1:8790`.

Port 10000 keeps existing private HTTPS routes on other ports unchanged.
Use the HTTPS URL printed by Funnel. Visitors need no Tailscale installation.

Run fixtures with `node --test scripts/public_gateway/gateway.test.mjs`.
The six fixtures cover routing, private boundaries, streaming cancellation,
concurrency, body limits and browser preflight. They do not contact Framer.
