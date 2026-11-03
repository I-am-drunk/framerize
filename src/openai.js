import { randomBytes } from 'node:crypto';
import { parseSse } from './sse.js';

const textOf = (content) => typeof content === 'string' ? content
  : Array.isArray(content) ? content.map((part) => part?.text || '').join('') : '';

function invalid(message) {
  const error = new Error(message);
  error.status = 400;
  return error;
}

function inputMessage(role, content, output = false) {
  const type = output ? 'output_text' : 'input_text';
  return { type: 'message', role, content: [{ type, text: textOf(content) }] };
}

export function chatToResponses(body) {
  if (!body || typeof body !== 'object' || Array.isArray(body)) throw invalid('body must be an object');
  if ((body.n ?? 1) !== 1) throw invalid('n must be 1');
  if (!Array.isArray(body.messages) || !body.messages.length) throw invalid('messages is required');
  const input = [];
  for (const message of body.messages) {
    if (!message || typeof message !== 'object') throw invalid('each message must be an object');
    if (['system', 'developer'].includes(message.role)) {
      input.push(inputMessage('developer', message.content));
    } else if (message.role === 'user') {
      input.push(inputMessage('user', message.content));
    } else if (message.role === 'assistant') {
      if (textOf(message.content)) input.push(inputMessage('assistant', message.content, true));
      for (const call of message.tool_calls || []) {
        input.push({ type: 'function_call', call_id: call.id,
          name: call.function?.name, arguments: call.function?.arguments || '' });
      }
    } else if (message.role === 'tool') {
      input.push({ type: 'function_call_output', call_id: message.tool_call_id,
        output: textOf(message.content) });
    }
  }
  if (!input.length) throw invalid('messages must contain content or tool calls');
  const result = { model: body.model, input, stream: body.stream === true };
  if (Array.isArray(body.tools)) result.tools = body.tools.filter((tool) => tool?.type === 'function')
    .map(({ function: fn }) => ({ type: 'function', name: fn?.name,
      description: fn?.description, parameters: fn?.parameters, ...(fn && 'strict' in fn ? { strict: fn.strict } : {}) }));
  if (typeof body.tool_choice === 'string') result.tool_choice = body.tool_choice;
  if (body.tool_choice?.type === 'function') result.tool_choice = {
    type: 'function', name: body.tool_choice.function?.name,
  };
  const limit = body.max_completion_tokens ?? body.max_tokens;
  if (limit !== undefined) result.max_output_tokens = limit;
  for (const key of ['temperature', 'top_p', 'stop', 'user', 'service_tier', 'parallel_tool_calls']) {
    if (body[key] !== undefined) result[key] = body[key];
  }
  if (body.response_format?.type === 'json_object') {
    result.text = { format: { type: 'json_object' } };
  } else if (body.response_format?.type === 'json_schema') {
    const format = body.response_format.json_schema;
    if (!format || typeof format !== 'object' || Array.isArray(format)) {
      throw invalid('response_format.json_schema must be an object');
    }
    result.text = { format: { ...format, type: 'json_schema' } };
  }
  return result;
}

export function responseToChat(response = {}, model) {
  let content = '';
  const toolCalls = [];
  for (const item of response.output || []) {
    if (item.type === 'message') {
      for (const part of item.content || []) {
        if (part.type === 'output_text') content += part.text || '';
      }
    } else if (['function_call', 'custom_tool_call'].includes(item.type)) {
      toolCalls.push({ id: item.call_id, type: 'function', function: {
        name: item.name, arguments: item.arguments ?? item.input ?? '',
      } });
    }
  }
  return {
    id: `chatcmpl-${String(response.id || '').replace(/^resp_/, '')}`,
    object: 'chat.completion', created: response.created_at || Math.floor(Date.now() / 1000),
    model, choices: [{ index: 0, message: {
      role: 'assistant', content: content || (toolCalls.length ? null : ''),
      ...(toolCalls.length ? { tool_calls: toolCalls } : {}),
    }, finish_reason: toolCalls.length ? 'tool_calls'
      : response.status === 'incomplete' ? 'length' : 'stop' }],
    usage: {
      prompt_tokens: response.usage?.input_tokens || 0,
      completion_tokens: response.usage?.output_tokens || 0,
      total_tokens: response.usage?.total_tokens || 0,
    },
  };
}

function chatChunk(model, delta, finishReason = null) {
  return { id: 'chatcmpl-local', object: 'chat.completion.chunk',
    created: Math.floor(Date.now() / 1000), model,
    choices: [{ index: 0, delta, finish_reason: finishReason }] };
}

export async function* responsesToChatStream(body, model) {
  const id = `chatcmpl-${randomBytes(12).toString('hex')}`;
  const created = Math.floor(Date.now() / 1000);
  const toolCalls = new Map();
  let finalResponse;
  for await (const event of body) {
      if (event.type === 'response.output_text.delta' && event.delta) {
        yield { ...chatChunk(model, { content: event.delta }), id, created };
      } else if (event.type === 'response.output_item.added'
          && ['function_call', 'custom_tool_call'].includes(event.item?.type)) {
        const index = toolCalls.size;
        toolCalls.set(event.output_index, index);
        yield { ...chatChunk(model, { tool_calls: [{ index, id: event.item.call_id,
          type: 'function', function: { name: event.item.name, arguments: '' } }] }), id, created };
      } else if (['response.function_call_arguments.delta',
        'response.custom_tool_call_input.delta'].includes(event.type)) {
        const index = toolCalls.get(event.output_index);
        if (index !== undefined && event.delta) yield { ...chatChunk(model, {
          tool_calls: [{ index, function: { arguments: event.delta } }],
        }), id, created };
      } else if (['response.completed', 'response.incomplete'].includes(event.type)) {
        finalResponse = event.response;
      }
  }
  const chat = responseToChat(finalResponse || { output: [] }, model);
  yield { ...chatChunk(model, {}, chat.choices[0].finish_reason), id, created };
  yield '[DONE]';
}

export async function readResponse(body) {
  const contentType = body.headers.get('content-type') || '';
  if (contentType.includes('application/json')) return body.json();
  let finalResponse;
  for await (const event of parseSse(body.body)) {
    if (event.data === '[DONE]') continue;
    const value = JSON.parse(event.data);
    if (['response.completed', 'response.incomplete'].includes(value.type)) finalResponse = value.response;
  }
  if (!finalResponse) throw new Error('upstream response did not complete');
  return finalResponse;
}

