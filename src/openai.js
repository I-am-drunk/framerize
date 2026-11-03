export function responseEventError(event) {
  if (event.type !== 'response.failed') return null;
  const error = new Error(event.response?.error?.message || 'upstream response failed');
  error.status = 502;
  return error;
}

function invalid(message) {
  const error = new Error(message);
  error.status = 400;
  return error;
}

export function validateChat(body) {
  if (!body || typeof body !== 'object' || Array.isArray(body)) throw invalid('body must be an object');
  if ((body.n ?? 1) !== 1) throw invalid('n must be 1');
  if (!Array.isArray(body.messages) || !body.messages.length) throw invalid('messages is required');
  for (const message of body.messages) {
    if (!message || typeof message !== 'object') throw invalid('each message must be an object');
    if (!['system', 'developer', 'user', 'assistant', 'tool'].includes(message.role)) {
      throw invalid('unsupported message role');
    }
  }
  if (body.response_format?.type === 'json_schema') {
    const format = body.response_format.json_schema;
    if (!format || typeof format !== 'object' || Array.isArray(format)) {
      throw invalid('response_format.json_schema must be an object');
    }
  }
}

