"""Read one stream while retaining a socket that can always be interrupted."""
import socket
import threading
import time
from .frames import Frames
from .http_error import response_details
from .support import error_code, require

class Stream:
    def __init__(self, connection, *, request_started=None):
        self.connection = connection
        started, started_at_unix = request_started or (time.monotonic(), time.time())
        self.started_monotonic = started
        self.control = connection.sock.dup()
        self.lock = threading.Lock()
        self.frames = Frames()
        self.state = {'status': None, 'headers_at_unix': None, 'eof': False,
                      'response_error_code': None,
                      'response_failure': None,
                      'finished': False, 'read_error': None,
                      'request_started_at_unix': started_at_unix,
                      'headers_elapsed_ms': None, 'first_text_elapsed_ms': None,
                      'terminal_elapsed_ms': None, 'reader_finished_elapsed_ms': None}
        self.cancelling = False
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _elapsed_ms(self):
        return (time.monotonic() - self.started_monotonic) * 1000

    def _read(self):
        response = None
        try:
            response = self.connection.getresponse()
            with self.lock:
                self.state['status'] = response.status
                self.state['headers_at_unix'] = time.time()
                self.state['headers_elapsed_ms'] = self._elapsed_ms()
            if response.status != 200:
                details = response_details(response)
                with self.lock:
                    self.state['response_error_code'] = details['native_error_code']
                    self.state['response_failure'] = details
                require(False, 'stream_http_status')
            require(response.getheader('Content-Type', '').split(';')[0].strip()
                    .lower() == 'text/event-stream', 'stream_content_type')
            while True:
                data = response.read1(4096)
                with self.lock:
                    if self.cancelling:
                        break
                    if not data:
                        require(response.length in (None, 0) and response.isclosed(),
                                'stream_http_framing')
                        self.frames.finish()
                        self.state['eof'] = True
                        break
                    self.frames.feed(data)
                    elapsed = self._elapsed_ms()
                    if (self.state['first_text_elapsed_ms'] is None and
                            self.frames.public()['text_characters'] > 0):
                        self.state['first_text_elapsed_ms'] = elapsed
                    if self.state['terminal_elapsed_ms'] is None and self.frames.completed:
                        self.state['terminal_elapsed_ms'] = elapsed
        except Exception as error:
            with self.lock:
                if not self.cancelling:
                    self.state['read_error'] = error_code(error)
        finally:
            if response is not None:
                response.close()
            self.connection.close()
            with self.lock:
                self.state['finished'] = True
                self.state['reader_finished_elapsed_ms'] = self._elapsed_ms()

    def snapshot(self):
        with self.lock:
            return dict(self.state, **self.frames.public())

    def close(self, cancel=False):
        eligible = True
        try:
            if cancel or self.reader.is_alive():
                with self.lock:
                    if cancel:
                        eligible = (not self.state['finished'] and
                                    not self.state['eof'] and
                                    self.state['read_error'] is None and
                                    self.frames.completed == 0)
                    self.cancelling = True
                    try:
                        self.control.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        if cancel:
                            raise
                    if cancel and eligible:
                        self.state['disconnected_at_unix'] = time.time()
                        self.state['disconnected_elapsed_ms'] = self._elapsed_ms()
        finally:
            self.reader.join(timeout=3)
            self.control.close()
        require(not self.reader.is_alive(), 'stream_reader_not_joined')
        require(eligible, 'stream_finished_before_disconnect')
