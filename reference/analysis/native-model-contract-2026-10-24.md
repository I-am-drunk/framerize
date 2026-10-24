# Current model selection and the unresolved security refusal

The current public application does not provide an exact browser comparison
for our GPT-4.1 Responses trial. Its ordinary model chooser sends that model
through Chat. This is source evidence, not proof that Responses is rejected.

The saved editor module `chunk-OEY74USB.mjs` imports model helpers from
`chunk-MR53IUCO.mjs` and `chunk-O5RQDDZJ.mjs`. Those two public files were
retrieved without authentication on October 24. Their SHA-256 digests are
`2ef6c4b6ede30929cc2a12005597ca6869f6b0e6f6c1508b6594b78aeaf0cd60`
and `ba515c0cf73d59fc765d4833c5bb54e540abbdc3fb4305d9268c23c4e6400133`.

The source closes three questions:

- `Cn`, imported as `Ll`, returns no provider override for `openai/gpt-4.1`.
  Adding an invented provider restriction is not supported by this source.
- `zr` tests membership in MR53IUCO's explicit Responses model map. GPT-4.1
  is absent. With no custom model override, chooser `c1` reaches `po`, imported
  from the shared module's `tx` export. That client uses `/ai/v3/chat/`.
  Our isolated trial instead uses `/ai/v3/responses/`.
- The reverse model mapper leaves `openai/gpt-4.1` unchanged. Our normalizer
  also produces that ID. There is no prefix mismatch for this model.

The Responses serializer remains valid evidence for that endpoint's format,
including the already corrected nested session ID. It does not establish a
successful current GPT-4.1 browser request to that endpoint. No route, model,
feature category, billing setting or runtime configuration changed here.

Four public assets and ten exact byte ranges are bound in scratch
`native-model-contract-r1/source-spans.json` (SHA-256
`9783c9193269ab529c3dafeb890905c3859c5562ab6555e6dad60d7c34a19796`).
REA parent `ev_d8f46e021edcd6d84300807773551aeeb8ed1fe15800dd3b4a62fce22c376798`
parsed both new modules without executing them. Its graph remains partial,
with 538 retained semantic unknowns and explicit omitted unknown records.
The findings above also follow the exact import/export and literal branches.
