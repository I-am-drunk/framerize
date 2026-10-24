"""Real, offscreen EGL/GLES component probe. Not a WebGL implementation."""
import ctypes as C

I, U, P, F = C.c_int, C.c_uint, C.c_void_p, C.c_float


def function(library, name, result, *arguments):
    value = getattr(library, name)
    value.restype, value.argtypes = result, list(arguments)
    return value


def attributes(*values):
    return (I * (len(values) + 1))(*values, 0x3038)


class Offscreen:
    def __init__(self):
        self.egl = C.CDLL('libEGL.so.1')
        self.gl = C.CDLL('libGLESv2.so.2')
        self.display = self.surface = self.context = None
        self.initialized = False
        self.closed = False

    def call(self, name, result, *arguments):
        return function(self.egl, name, result, *arguments)

    def api(self, name, result, *arguments):
        return function(self.gl, name, result, *arguments)

    def require(self, result, phase):
        if not result:
            code = self.call('eglGetError', U)()
            raise RuntimeError(f'{phase}: EGL error {code:#x}')
        return result

    def __enter__(self):
        try:
            self.display = self.require(self.call(
                'eglGetPlatformDisplay', P, U, P, P)(0x31DD, None, None), 'display')
            major, minor = I(), I()
            self.require(self.call('eglInitialize', U, P, P, P)(
                self.display, C.byref(major), C.byref(minor)), 'initialize')
            self.initialized = True
            self.require(self.call('eglBindAPI', U, U)(0x30A0), 'bind GLES')
            config, count = P(), I()
            attrs = attributes(0x3033, 1, 0x3040, 4,
                               0x3024, 8, 0x3023, 8, 0x3022, 8, 0x3021, 8)
            self.require(self.call('eglChooseConfig', U, P, P, P, I, P)(
                self.display, attrs, C.byref(config), 1, C.byref(count)), 'config')
            if count.value != 1:
                raise RuntimeError('no RGBA8 pbuffer configuration')
            self.surface = self.require(self.call('eglCreatePbufferSurface', P, P, P, P)(
                self.display, config, attributes(0x3057, 64, 0x3056, 64)), 'surface')
            self.context = self.require(self.call('eglCreateContext', P, P, P, P, P)(
                self.display, config, None, attributes(0x3098, 2)), 'context')
            self.require(self.call('eglMakeCurrent', U, P, P, P, P)(
                self.display, self.surface, self.surface, self.context), 'current')
            self.version = [major.value, minor.value]
            return self
        except BaseException:
            self.close()
            raise

    def __exit__(self, *error):
        self.close()

    def close(self):
        failures = []
        if self.initialized:
            if not self.call('eglMakeCurrent', U, P, P, P, P)(
                    self.display, None, None, None):
                failures.append('release current')
            for name, resource in [('eglDestroyContext', self.context),
                                   ('eglDestroySurface', self.surface)]:
                if resource and not self.call(name, U, P, P)(self.display, resource):
                    failures.append(name)
            if not self.call('eglTerminate', U, P)(self.display):
                failures.append('terminate display')
        if not self.call('eglReleaseThread', U)():
            failures.append('release thread')
        self.initialized = False
        self.display = self.surface = self.context = None
        self.closed = not failures
        if failures:
            raise RuntimeError('EGL cleanup failed: ' + ', '.join(failures))
