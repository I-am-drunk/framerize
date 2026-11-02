# Public inference on IRIS

Branch: `ops/public-inference-20261101`.

The persistent public URL is `https://iris-1.javanese-mercat.ts.net:10000/`.
Funnel forwards to the separate proxy on `127.0.0.1:8790`.
Existing port 443 access remains private and still forwards to port 8080.
No Framerize restart or release change was needed for this endpoint.

The public DNS address returned HTTP 200 for `/` and `/health`, and HTTP 404
for `/api/accounts`. These checks explicitly used the public relay address.
The user service is enabled. All six proxy fixtures passed; the checked-in
source and tests match the exercised files byte for byte.

The recurring-error binary completed its separate qualification and activation.
The live service runs alpha.9 from `132af22691dc04680f2ecadaace8e92a4d2f706e`.
Real requests through the public DNS address, with no authentication sent,
returned HTTP 200 and the expected generated text in both dialects:

| Route | Model | Seconds |
|---|---|---:|
| `/openai/v1/responses` | `gpt-4.1` | 1.871 |
| `/anthropic/v1/messages` | `claude-haiku-4-5` | 1.138 |

OpenAI clients can use `https://iris-1.javanese-mercat.ts.net:10000/openai/v1`.
Anthropic clients can use `https://iris-1.javanese-mercat.ts.net:10000/anthropic`.
The shared root also supports both dialects. Visitors need no Tailscale or
password. The proxy and inference services are enabled and running.
Next: use either base URL in a compatible client.
