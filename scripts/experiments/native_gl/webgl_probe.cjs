// Runs against an explicitly mounted experimental implementation, never a service.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const create = require('/work/implementation');
const { NativeWebGL } = require('/work/implementation/src/javascript/native-gl');
const checks = [];
const digest = value => crypto.createHash('sha256').update(value).digest('hex');

function pixel(gl, rgba) {
  const value = new Uint8Array(4);
  gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, value);
  assert.deepEqual([...value], rgba);
  assert.equal(gl.getError(), gl.NO_ERROR);
}

function compile(gl, type, source, expected) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  assert.equal(gl.getShaderParameter(shader, gl.COMPILE_STATUS), expected);
  return shader;
}

function draw(gl) {
  const vertex = compile(gl, gl.VERTEX_SHADER,
    'attribute vec2 position;void main(){gl_Position=vec4(position,0.,1.);}', true);
  const fragment = compile(gl, gl.FRAGMENT_SHADER,
    'precision mediump float;void main(){gl_FragColor=vec4(1.,0.,0.,1.);}', true);
  const bad = compile(gl, gl.FRAGMENT_SHADER, 'this is not GLSL', false);
  const program = gl.createProgram();
  gl.attachShader(program, vertex); gl.attachShader(program, fragment);
  gl.bindAttribLocation(program, 0, 'position'); gl.linkProgram(program);
  assert.equal(gl.getProgramParameter(program, gl.LINK_STATUS), true);
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  gl.enableVertexAttribArray(0); gl.useProgram(program);
  gl.drawArrays(gl.TRIANGLES, 0, 3); pixel(gl, [255, 0, 0, 255]);
  const instancing = gl.getExtension('ANGLE_instanced_arrays');
  assert.ok(instancing);
  gl.clearColor(0, 0, 1, 1); gl.clear(gl.COLOR_BUFFER_BIT);
  instancing.drawArraysInstancedANGLE(gl.TRIANGLES, 0, 3, 1);
  pixel(gl, [255, 0, 0, 255]);
  for (const shader of [vertex, fragment, bad]) gl.deleteShader(shader);
  gl.deleteProgram(program); gl.deleteBuffer(buffer);
  assert.equal(gl.getError(), gl.NO_ERROR);
  checks.push('shader_draw', 'invalid_shader', 'instanced_draw', 'object_cleanup');
}

assert.equal(create(0, 64), null);
assert.equal(create(64, 0), null);
checks.push('invalid_dimensions');
const queryOnly = process.argv.includes('--queries');
const contexts = [];
const native = [];
try {
  for (let i = 0; i < 2; i++) {
    const gl = create(64, 64, { depth: false, stencil: false });
    assert.ok(gl, 'surfaceless WebGL context');
    contexts.push(gl);
    assert.equal(NativeWebGL.activeContexts(), i + 1);
    const debug = gl.getExtension('WEBGL_debug_renderer_info');
    assert.ok(debug);
    assert.equal(gl.getExtension('WEBGL_debug_shaders'), null);
    assert.match(gl.getParameter(debug.UNMASKED_RENDERER_WEBGL), /llvmpipe/);
    assert.equal(gl.getParameter(debug.UNMASKED_VENDOR_WEBGL), gl.getParameter(gl.VENDOR));
    assert.equal(gl.getParameter(debug.UNMASKED_RENDERER_WEBGL), gl.getParameter(gl.RENDERER));
    native.push({ vendor: digest(gl.getParameter(gl.VENDOR)),
      renderer: digest(gl.getParameter(gl.RENDERER)) });
    assert.equal(gl.getError(), gl.NO_ERROR);
    if (!queryOnly) {
      gl.clearColor(0, 0, 1, 1); gl.clear(gl.COLOR_BUFFER_BIT);
      pixel(gl, [0, 0, 255, 255]); draw(gl);
    }
  }
  checks.push('two_contexts', 'native_identity', 'unsupported_extension');
  const destroyed = contexts[0].getExtension('STACKGL_destroy_context').destroy();
  contexts[0] = null;
  assert.equal(destroyed, true);
  assert.equal(NativeWebGL.activeContexts(), 1);
  assert.match(contexts[1].getParameter(contexts[1].RENDERER), /llvmpipe/);
  checks.push('other_context_survives_close');
} finally {
  for (const gl of contexts) if (gl) {
    assert.equal(gl.getExtension('STACKGL_destroy_context').destroy(), true);
  }
}
assert.equal(NativeWebGL.cleanup(), true);
const fresh = create(64, 64, { depth: false, stencil: false });
assert.ok(fresh);
assert.equal(fresh.getExtension('STACKGL_destroy_context').destroy(), true);
assert.equal(NativeWebGL.cleanup(), true);
assert.equal(NativeWebGL.cleanup(), true);
checks.push('fresh_context_after_close');
fs.writeFileSync('/out/probe.json', JSON.stringify({ passed: true, checks, native,
  shader_executed: !queryOnly, sdk_executed: false }, null, 2) + '\n', { flag: 'wx' });
