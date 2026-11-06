import assert from 'node:assert/strict';

// Synthetic request shapes for the custom Responses and Messages routes. Model labels are
// opaque forwarding values: an offline test proves preservation, not live model support.

/** @typedef {'responses' | 'messages'} CustomProtocol */
/** @typedef {'omitted' | 'false' | 'true'} ClientStreamMode */
/**
 * @typedef {object} CustomRequestCase
 * @property {string} name
 * @property {CustomProtocol} protocol
 * @property {string} path Client route.
 * @property {string} upstream Exact custom upstream path.
 * @property {Readonly<Record<string, unknown>>} body Client request without `stream`.
 */

export const customRoutes = Object.freeze({
  responses: Object.freeze({ path: '/openai/v1/responses', upstream: '/ai/v3/responses/' }),
  messages: Object.freeze({ path: '/anthropic/v1/messages', upstream: '/ai/v3/messages/' }),
});

/** @type {readonly ClientStreamMode[]} */
export const clientStreamModes = Object.freeze(['omitted', 'false', 'true']);

/** @param {Readonly<Record<string, unknown>>} body @param {ClientStreamMode} mode */
export function withStream(body, mode) {
  return mode === 'omitted' ? { ...body } : { ...body, stream: mode === 'true' };
}

export const unicodeText = 'Grüße, 世界! Ελληνικά, עברית, العربية, हिन्दी, 👩🏽‍💻🧪, e\u0301, \u2028, "quoted\\path"';

// Deterministic filler with multibyte characters so byte budgets cover UTF-8 encoding.
/** @param {string} label @param {number} bytes */
export function fillerText(label, bytes) {
  const unit = `${label} ❄ snow-δ `;
  return unit.repeat(Math.floor(bytes / Buffer.byteLength(unit)));
}

// Tiny synthetic attachments: a 1x1 PNG and a short text-only PDF-like document.
export const syntheticImage = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYPj/HwADAgH/eL9GtQAAAABJRU5ErkJggg==';
export const syntheticDocument = Buffer.from('%PDF-1.4\n% synthetic Framerize fixture\n%%EOF\n').toString('base64');

const lookupSchema = { type: 'object', additionalProperties: false, required: ['query', 'filters'],
  properties: { query: { type: 'string' }, filters: { type: 'object', properties: {
    units: { type: 'string', enum: ['metric', 'imperial'] },
    days: { type: 'array', items: { type: 'integer', minimum: 1, maximum: 7 } } } } } };

/** @param {CustomProtocol} protocol @param {number} turns @param {number} bytesPerTurn */
export function history(protocol, turns, bytesPerTurn) {
  return Array.from({ length: turns }, (_, index) => {
    const role = index % 2 === 0 ? 'user' : 'assistant';
    const text = `${index}: ${fillerText(role, bytesPerTurn)}`;
    if (protocol === 'messages') return { role, content: [{ type: 'text', text }] };
    return { type: 'message', role,
      content: [{ type: role === 'user' ? 'input_text' : 'output_text', text }] };
  });
}

const attachmentText = 'Describe both synthetic attachments.';

/** @type {readonly CustomRequestCase[]} */
export const customRequestCases = Object.freeze([
  { name: 'openai-alpha unicode with strict tools', protocol: 'responses', ...customRoutes.responses,
    body: { model: 'synthetic-gpt-alpha-2026-01', instructions: unicodeText,
      input: [{ role: 'user', content: [{ type: 'input_text', text: unicodeText }] }],
      tools: [{ type: 'function', name: 'lookup', description: 'Synthetic lookup ✓',
        parameters: lookupSchema, strict: true }],
      tool_choice: { type: 'function', name: 'lookup' }, parallel_tool_calls: false,
      reasoning: { effort: 'high', summary: 'auto' },
      text: { verbosity: 'low', format: { type: 'json_schema', name: 'answer', strict: true,
        schema: { type: 'object', properties: { answer: { type: 'string' } }, required: ['answer'] } } },
      include: ['reasoning.encrypted_content'], store: false, max_output_tokens: 512,
      temperature: 0, top_p: 1, metadata: { case: 'openai-alpha', note: unicodeText } } },
  { name: 'openai-beta history with image and file', protocol: 'responses', ...customRoutes.responses,
    body: { model: 'synthetic/gpt-beta:2026-02', input: [...history('responses', 24, 2048),
      { role: 'user', content: [{ type: 'input_text', text: attachmentText },
        { type: 'input_image', detail: 'low', image_url: `data:image/png;base64,${syntheticImage}` },
        { type: 'input_file', filename: 'fixture.pdf',
          file_data: `data:application/pdf;base64,${syntheticDocument}` }] }],
    service_tier: 'flex', truncation: 'auto', prompt_cache_key: 'synthetic-cache',
    max_output_tokens: 64, metadata: { case: 'openai-beta' } } },
  { name: 'claude-alpha unicode with tools and thinking', protocol: 'messages', ...customRoutes.messages,
    body: { model: 'synthetic-claude-alpha-20260101', max_tokens: 2048,
      system: [{ type: 'text', text: unicodeText, cache_control: { type: 'ephemeral' } }],
      messages: [{ role: 'user', content: unicodeText }],
      tools: [{ name: 'lookup', description: 'Synthetic lookup ✓', input_schema: lookupSchema,
        cache_control: { type: 'ephemeral' } }],
      tool_choice: { type: 'auto', disable_parallel_tool_use: true },
      thinking: { type: 'enabled', budget_tokens: 1024 },
      stop_sequences: ['\n\nSTOP', '終'], top_k: 5, metadata: { user_id: 'synthetic-user-α' } } },
  { name: 'claude-beta history with tool result, image and document', protocol: 'messages',
    ...customRoutes.messages,
    body: { model: 'synthetic.claude-beta@2026-02', max_tokens: 64, temperature: 0.5,
      messages: [...history('messages', 21, 2048),
        { role: 'assistant', content: [{ type: 'tool_use', id: 'toolu_synthetic', name: 'lookup',
          input: { query: 'snow ❄', filters: { units: 'metric', days: [1, 2] } } }] },
        { role: 'user', content: [
          { type: 'tool_result', tool_use_id: 'toolu_synthetic', content: [
            { type: 'text', text: 'Synthetic result ❄' },
            { type: 'image', source: { type: 'base64', media_type: 'image/png', data: syntheticImage } }] },
          { type: 'document', title: 'fixture.pdf', source: { type: 'base64',
            media_type: 'application/pdf', data: syntheticDocument } },
          { type: 'text', text: attachmentText }] }],
      service_tier: 'standard_only', metadata: { user_id: 'synthetic-user-β' } } },
]);

/** Copy a case body with a request identity in its protocol's metadata field. */
/** @param {CustomRequestCase} entry @param {string} id */
export function identify(entry, id) {
  const key = entry.protocol === 'responses' ? 'case' : 'user_id';
  return { ...entry.body, metadata: { ...(/** @type {object} */ (entry.body.metadata)), [key]: id } };
}

/** @param {Readonly<Record<string, unknown>>} request */
export function identityOf(request) {
  const metadata = /** @type {{ case?: unknown, user_id?: unknown } | undefined} */ (request.metadata);
  const id = metadata?.case ?? metadata?.user_id;
  assert.equal(typeof id, 'string', 'Synthetic request must carry an identity');
  return /** @type {string} */ (id);
}
