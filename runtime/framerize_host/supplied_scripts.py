"""Browser code for the explicit supplied-token operation."""
from pathlib import Path

ROOT = Path(__file__).parent
START = (ROOT / 'browser' / 'supplied_start.js').read_text().replace(
    '__READ_RESPONSE__', (ROOT / 'browser' / 'response_reader.js').read_text())
POLL = """([id, ack]) => {
  const contentType = window.__aiJobs?.[id]?.contentType ?? null;
  const value = (%s)([id, ack]);
  return {...value, content_type: contentType};
}""" % (ROOT / 'browser' / 'response_poll.js').read_text()
