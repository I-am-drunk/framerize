export function event(type: string, fields: Readonly<Record<string, unknown>>): Buffer {
  return Buffer.from(`event: ${type}\ndata: ${JSON.stringify({ ...fields, type })}\n\n`);
}
