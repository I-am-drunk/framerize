"""Read one stream while retaining a socket that can always be interrupted."""
import socket
import threading
import time
from frames import Frames
from support import require

class Stream:
    def __init__(self, connection):
        self.connection = connection
        self.control = connection.sock.dup()
        self.lock = threading.Lock()
        self.frames = Frames()
        self.state = {'status': None, 'headers_at_unix': None, 'eof': False,
                      'finished': False, 'read_error': None}
        self.cancelling = False
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        response = None
        try:
            response = self.connection.getresponse()
            with self.lock:
                self.state['status'] = response.status
                self.state['operation_id'] = response.getheader('x-framerize-operation-id')
                self.state['headers_at_unix'] = time.time()
            require(response.status == 200, 'stream_http_status')
            require(response.getheader('Content-Type', '').split(';')[0].strip()
                    == 'text/event-stream', 'stream_content_type')
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
        except Exception as error:
            with self.lock:
                if not self.cancelling:
                    self.state['read_error'] = (str(error) if type(error) is
                                                RuntimeError else type(error).__name__)
        finally:
            if response is not None:
                response.close()
            self.connection.close()
            with self.lock:
                self.state['finished'] = True

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
        finally:
            self.reader.join(timeout=3)
            self.control.close()
        require(not self.reader.is_alive(), 'stream_reader_not_joined')
        require(eligible, 'stream_finished_before_disconnect')
