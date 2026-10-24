"""Exercise installed GLES with synthetic pixels only; no SDK or account data."""
import ctypes as C
import json
from pathlib import Path

from egl import F, I, P, U, Offscreen


def shader(context, kind, source, expected=True):
    create = context.api('glCreateShader', U, U)
    handle = create(kind)
    assert handle, 'shader allocation'
    text = C.c_char_p(source)
    context.api('glShaderSource', None, U, I, P, P)(handle, 1, C.byref(text), None)
    context.api('glCompileShader', None, U)(handle)
    status = I()
    context.api('glGetShaderiv', None, U, U, P)(handle, 0x8B81, C.byref(status))
    if bool(status.value) != expected:
        context.api('glDeleteShader', None, U)(handle)
        raise AssertionError('unexpected shader compilation status')
    return handle


def draw(context):
    api = context.api
    shaders, program, buffer = [], 0, U()
    try:
        shaders.append(shader(context, 0x8B31,
            b'attribute vec2 position;void main(){gl_Position=vec4(position,0.,1.);}'))
        shaders.append(shader(context, 0x8B30,
            b'precision mediump float;void main(){gl_FragColor=vec4(1.,0.,0.,1.);}'))
        shaders.append(shader(context, 0x8B30, b'this is not GLSL', expected=False))
        program = api('glCreateProgram', U)()
        assert program, 'program allocation'
        for item in shaders[:2]:
            api('glAttachShader', None, U, U)(program, item)
        api('glBindAttribLocation', None, U, U, C.c_char_p)(program, 0, b'position')
        api('glLinkProgram', None, U)(program)
        linked = I()
        api('glGetProgramiv', None, U, U, P)(program, 0x8B82, C.byref(linked))
        assert linked.value, 'program linking'
        api('glUseProgram', None, U)(program)
        api('glGenBuffers', None, I, P)(1, C.byref(buffer))
        assert buffer.value, 'buffer allocation'
        api('glBindBuffer', None, U, U)(0x8892, buffer)
        vertices = (F * 6)(-1, -1, 3, -1, -1, 3)
        api('glBufferData', None, U, C.c_ssize_t, P, U)(
            0x8892, C.sizeof(vertices), vertices, 0x88E4)
        api('glVertexAttribPointer', None, U, I, U, C.c_ubyte, I, P)(
            0, 2, 0x1406, 0, 0, None)
        api('glEnableVertexAttribArray', None, U)(0)
        api('glViewport', None, I, I, I, I)(0, 0, 64, 64)
        api('glDrawArrays', None, U, I, I)(4, 0, 3)
        pixel = (C.c_ubyte * 4)()
        api('glReadPixels', None, I, I, I, I, U, U, P)(
            32, 32, 1, 1, 0x1908, 0x1401, pixel)
        assert list(pixel) == [255, 0, 0, 255], 'shader pixel readback'
        assert api('glGetError', U)() == 0, 'GLES error'
    finally:
        api('glUseProgram', None, U)(0)
        if buffer.value:
            api('glDeleteBuffers', None, I, P)(1, C.byref(buffer))
        if program:
            api('glDeleteProgram', None, U)(program)
        for item in shaders:
            api('glDeleteShader', None, U)(item)
    assert api('glGetError', U)() == 0, 'GLES object cleanup'


def main():
    cycles = []
    for _ in range(2):
        with Offscreen() as context:
            api = context.api
            renderer = api('glGetString', C.c_char_p, U)(0x1F01)
            version = api('glGetString', C.c_char_p, U)(0x1F02)
            assert renderer and version and b'OpenGL ES' in version
            assert b'llvmpipe' in renderer or b'softpipe' in renderer
            api('glClearColor', None, F, F, F, F)(0, 0, 1, 1)
            api('glClear', None, U)(0x4000)
            pixel = (C.c_ubyte * 4)()
            api('glReadPixels', None, I, I, I, I, U, U, P)(
                32, 32, 1, 1, 0x1908, 0x1401, pixel)
            assert list(pixel) == [0, 0, 255, 255], 'clear pixel readback'
            draw(context)
            cycles.append(dict(egl_version=context.version, software_renderer=True,
                clear_readback=True, shader_readback=True, bad_shader_rejected=True))
        assert context.closed, 'context cleanup'
        cycles[-1]['context_closed'] = True
    result = dict(passed=True, cycles=cycles, sdk_executed=False, webgl_implemented=False)
    Path('/out/probe.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
