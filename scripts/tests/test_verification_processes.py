import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from verification_processes import descendants

WORKER = '''
import subprocess, threading, time

def worker():
    child = subprocess.Popen(['sleep', '30'])
    print(child.pid, flush=True)
    child.wait()

threading.Thread(target=worker).start()
time.sleep(30)
'''

class DescendantTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform.startswith('linux'), 'Linux process ownership')
    def test_worker_thread_children_are_included(self):
        process = subprocess.Popen([sys.executable, '-c', WORKER],
            stdout=subprocess.PIPE, text=True, start_new_session=True)
        try:
            self.assertTrue(select.select([process.stdout], [], [], 3)[0])
            child = int(process.stdout.readline())
            main_children = Path(f'/proc/{process.pid}/task/{process.pid}/children').read_text()
            self.assertNotIn(str(child), main_children.split())
            self.assertEqual(descendants(process.pid), {process.pid, child})
        finally:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            process.wait(timeout=3)
            process.stdout.close()

if __name__ == '__main__':
    unittest.main()
