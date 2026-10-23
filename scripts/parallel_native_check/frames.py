"""Bounded Responses SSE evidence, retaining counts rather than response text."""
import json
from .support import require

class Frames:
    def __init__(self):
        self.pending = b''
        self.lines = []
        self.bytes = 0
        self.events = 0
        self.completed = 0
        self.text_characters = 0
        self.terminal_text_characters = 0
        self.done_marker = False

    def feed(self, data):
        self.bytes += len(data)
        require(self.bytes <= 65536, 'stream_byte_limit')
        self.pending += data
        while b'\n' in self.pending:
            line, self.pending = self.pending.split(b'\n', 1)
            line = line.removesuffix(b'\r')
            if line:
                self.lines.append(line)
            else:
                self.event()
                self.lines.clear()

    def event(self):
        data = [line[5:].lstrip(b' ') for line in self.lines
                if line.startswith(b'data:')]
        if not data:
            return
        payload = b'\n'.join(data)
        if payload == b'[DONE]':
            require(self.completed == 1, 'done_without_completion')
            require(not self.done_marker, 'duplicate_done_marker')
            self.done_marker = True
            return
        value = json.loads(payload)
        require(isinstance(value, dict), 'invalid_stream_event')
        kind = value.get('type')
        require(isinstance(kind, str), 'missing_stream_event_type')
        labels = [line[6:].strip().decode() for line in self.lines
                  if line.startswith(b'event:')]
        require(not labels or labels == [kind], 'stream_event_type_mismatch')
        require(not self.completed, 'event_after_completion')
        require(kind not in ('error', 'response.failed', 'response.incomplete')
                and not value.get('error'), 'stream_error')
        self.events += 1
        if kind == 'response.output_text.delta':
            require(isinstance(value.get('delta'), str), 'invalid_text_delta')
            self.text_characters += len(value['delta'])
        elif kind == 'response.completed':
            response = value.get('response')
            require(isinstance(response, dict) and response.get('status') ==
                    'completed' and not response.get('error') and
                    not response.get('incomplete_details'), 'invalid_completion')
            self.terminal_text_characters = sum(
                len(part['text']) for item in response.get('output', [])
                for part in item.get('content', [])
                if part.get('type') == 'output_text' and isinstance(part.get('text'), str))
            self.completed += 1

    def finish(self):
        require(not self.pending and not self.lines, 'partial_stream_frame')
        require(self.completed == 1, 'missing_terminal_completion')
        require(max(self.text_characters, self.terminal_text_characters) > 0,
                'empty_stream_output')
        if self.text_characters and self.terminal_text_characters:
            require(self.text_characters == self.terminal_text_characters,
                    'incomplete_stream_text')

    def public(self):
        return {'bytes': self.bytes, 'events': self.events,
                'terminal_completions': self.completed,
                'text_characters': max(self.text_characters,
                                       self.terminal_text_characters),
                'done_marker': self.done_marker}
