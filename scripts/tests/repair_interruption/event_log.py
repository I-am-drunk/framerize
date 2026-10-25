import json
from evidence import Evidence
from support import require

class EventLog(Evidence):
    def __init__(self, path):
        super().__init__()
        self.path = path
        self.file = path.open('rb')
        self.file.seek(0, 2)
        self.identity = (path.stat().st_dev, path.stat().st_ino)
        self.pending = b''
        self.bytes = 0

    def poll(self):
        st = self.path.stat()
        require((st.st_dev, st.st_ino) == self.identity and
                st.st_size >= self.file.tell(), 'event_log_changed')
        data = self.file.read(65536)
        self.bytes += len(data)
        require(self.bytes <= 4 * 1024**2, 'event_log_limit')
        self.pending += data
        lines = self.pending.split(b'\n')
        self.pending = lines.pop()
        require(len(self.pending) <= 262144, 'event_line_limit')
        for line in lines:
            self.add(json.loads(line))
        require(not self.count('operation.admission.refused'), 'admission_refused')
        require(not self.count('operation.unresolved'), 'unresolved_operation')
        require(not self.count('ai.chain.repick'), 'unexpected_repick')
        require(not any(r.get('metered_chunks', 0) for r in self.rows), 'metered_call')

    def close(self):
        self.file.close()
