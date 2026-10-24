"""Run one real process-owner case per fresh delegated test service."""
import json
import os
from pathlib import Path
import struct
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
from browser_comparison_process import Child


def run(case, binary):
    with tempfile.TemporaryDirectory() as temporary:
        if case == 'descendant':
            binary = Path(temporary) / 'inert-child'
            binary.write_text('#!/usr/bin/python3\nimport os,signal,socket,sys\n'
                'sock=socket.socket(fileno=int(sys.argv[2]))\n'
                'pid=os.fork()\n'
                'if pid==0:\n os.setsid()\n sock.sendall(b"ready")\n signal.pause()\n'
                'signal.pause()\n')
            binary.chmod(0o700)
        owner = Child(binary, {'PATH': '/usr/bin:/bin', 'PYTHONDONTWRITEBYTECODE': '1'},
                      memory_bytes=128 * 1024**2, tasks=16)
        try:
            sock = owner.launch()
            sock.settimeout(3)
            if case == 'descendant':
                assert sock.recv(5) == b'ready'
                members = set(map(int, (owner.group / 'cgroup.procs').read_text().split()))
                assert len(members) == 2 and owner.process.pid in members
                descendant = (members - {owner.process.pid}).pop()
                fields = Path(f'/proc/{descendant}/stat').read_text().rsplit(') ', 1)[1].split()
                assert int(fields[2]) == descendant  # An independent process group.
            elif case == 'reject':
                sock.sendall(struct.pack('!I', 2) + b'{}')
            else:
                raise ValueError('unknown inert case')
        finally:
            result = owner.close()
        assert result['group_absent']
        assert result['forced'] == (case == 'descendant')
        assert result['exit_code'] == (-9 if case == 'descendant' else 2)
        usage = result['resources']
        assert usage['memory.peak'] <= 128 * 1024**2
        assert not any(usage['memory.events'].values())
        assert not any(usage['pids.events'].values())
        assert usage['memory.swap.current'] == 0
        print(json.dumps(dict(case=case, accepted=True, cleanup=result)))


if __name__ == '__main__':
    run(sys.argv[1], Path(sys.argv[2]).resolve(strict=True))
