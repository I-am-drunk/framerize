function own(object, key) {
  if (object === null || !['object', 'function'].includes(typeof object)) return;
  const field = Object.getOwnPropertyDescriptor(object, key);
  return field && Object.hasOwn(field, 'value') ? field.value : undefined;
}

export function observeBranch(entries) {
  const length = own(entries, 'length');
  if (!Array.isArray(entries) || !Number.isInteger(length) || length > 1000)
    throw new Error('logger-shape');
  const counts = { v3: 0, v2: 0, none: 0, errors: 0 };
  for (let index = 0; index < length; index++) {
    const entry = own(entries, String(index));
    if (own(own(entry, 'logger'), 'id') !== 'castle') continue;
    const level = own(entry, 'level');
    if (level === 4) counts.errors++;
    if (level !== 1) continue;
    const parts = own(entry, 'parts');
    if (!Array.isArray(parts) || own(parts, 'length') !== 1) continue;
    const message = own(parts, '0');
    if (message === 'Using Castle v3 token') counts.v3++;
    else if (message === 'Using Castle v2 token') counts.v2++;
    else if (message === 'No Castle token available') counts.none++;
  }
  return counts;
}

export function exactV3(counts) {
  return counts.v3 === 1 && counts.v2 === 0 && counts.none === 0 && counts.errors === 0;
}
