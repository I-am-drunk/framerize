"""Measure context and renderer queries separately from shader execution."""
import ctypes as C
import hashlib
import json
from pathlib import Path

from egl import U, Offscreen


def main():
    cycles = []
    for _ in range(2):
        with Offscreen() as context:
            query = context.api('glGetString', C.c_char_p, U)
            fields = {name: query(value) for name, value in {
                'vendor': 0x1F00, 'renderer': 0x1F01,
                'version': 0x1F02, 'extensions': 0x1F03}.items()}
            assert all(fields.values()), 'empty native graphics query'
            assert b'llvmpipe' in fields['renderer']
            assert b'OpenGL ES' in fields['version']
            assert context.api('glGetError', U)() == 0
            cycles.append(dict(egl_version=context.version, software_renderer=True,
                fields_sha256={k: hashlib.sha256(v).hexdigest() for k, v in fields.items()},
                extension_count=len(fields['extensions'].split())))
        assert context.closed
        cycles[-1]['context_closed'] = True
    Path('/out/probe.json').write_text(json.dumps(dict(passed=True, cycles=cycles,
        sdk_executed=False, webgl_implemented=False, shader_executed=False), indent=2) + '\n')


if __name__ == '__main__':
    main()
