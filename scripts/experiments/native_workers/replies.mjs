// Classify the two pinned SDK scripts' replies without retaining values.
export function classifyReply(value) {
  if (!Array.isArray(value)) return 'other';
  if (value.length === 10 && value[0] === 1 && value[1] === true
      && value[2] === 'Framerize/0.1'
      && [3, 4, 5, 6, 8, 9].every(i => value[i] === undefined)
      && Array.isArray(value[7]) && value[7].length === 0) return 'feature';
  if (value.length === 3 && value[0] === 3 && value[1] === false
      && value[2] === null) return 'entropy';
  if (value.length === 3 && value[0] === 4 && value[1] === false
      && Number.isFinite(value[2]) && value[2] >= 0) return 'timing';
  if (value.length === 3 && value[0] === 5) return 'error';
  return 'other';
}
