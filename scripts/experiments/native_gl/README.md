# Native graphics feasibility

These probes call the installed EGL/OpenGL ES libraries through Python ctypes.
They create a private, surfaceless 64 by 64 pixel buffer, use real software
rendering, and explicitly destroy contexts and surfaces. They run with no
display, GPU device, browser, network, account state or SDK. This is not a
WebGL implementation and is not selected by the application.

`probe.py` checks clear/readback, compiled triangle rendering, invalid shader
rejection, object cleanup and two consecutive context lifetimes. The llvmpipe
run completed those checks but FAILED resource qualification: its 128 MiB
limit recorded 60 maximum events. The softpipe alternative was OOM-killed at
the same limit before producing a result. Both failures remain recorded.

`query_probe.py` measures context creation, actual renderer/version/extension
queries and cleanup without compiling or drawing. That separate run passed at
55,771,136 bytes, six tasks, zero swap/events and verified unit/group absence.
It does not convert the full rendering failures into passes or establish the
combined Node/SDK memory requirement. The runner retains the existing host
memory reserve, 128 MiB group limit, no swap, 32 tasks and bounded lifetime.

`qualification.json` binds the exact sources and saved outcomes. Renderer data
is hashed, not replaced with a claimed browser or GPU identity. The next step
is a direct Node binding measurement with genuine driver results and explicit
cleanup. No new live request is justified by these component results alone.

The direct Node binding (`egl.mjs`) now passes a separate bounded probe with
Node 24, JIT disabled and Koffi 3.3.3. It peaks at 47,398,912 bytes with zero
swap/events and verified teardown. Driver field hashes match the independent
Python query. The probe also checks exclusive context ownership, refusal after
close and repeated close. Dependencies were integrity-checked and unpacked in
the isolated fixture without running package installation scripts.
This remains a native query component, with no DOM, SDK or WebGL integration.
`node-qualification.json` binds its sources, dependencies and saved results.

The Mesa port in `mesa.patch` applies to upstream `gl` 8.1.6. It replaces the
ANGLE instancing entry points with the equivalent supported EXT entry points,
requires both native extensions, and returns the driver's actual vendor and
renderer. It fixes surface cleanup, propagates cleanup results through the
JavaScript wrapper, and reclaims unreachable JavaScript before native context
allocation. It does not advertise an empty shader-translation implementation.

Build inputs are `gl` 8.1.6, NAN 2.24.0 and the Node 24.13.0 headers. The three
native source files compile as a shared C++20 addon with `USE_OZONE` for the
header's window-system-independent handle types, linked to installed EGL/GLES.
No X server or browser is used. Runtime selects surfaceless llvmpipe with
`LP_NUM_THREADS=0` and `LP_NATIVE_VECTOR_WIDTH=128`; shader disk caching is off.

The final addon passes real shader/instanced rendering, invalid-shader rejection,
native-identity queries, simultaneous contexts and confirmed teardown at
93,655,040 bytes. The HTML canvas bridge passes six additional type, resizing,
resource and ownership checks with real JSDOM/canvas at 103,636,992 bytes.
Both used the unchanged 128 MiB cap, zero swap/events and verified group absence.
Earlier setup, resource and cleanup-wrapper failures remain in the qualification.

Apply `lifetime.patch` after `mesa.patch` for the subsequent SDK integration.
It removes a redundant native allocation in the public JavaScript wrapper,
exposes the actual live-context count, returns free native allocator pages,
and preserves cleanup failures even when disposal occurs during collection.

`canvas_webgl.mjs` bounds each window to two live native contexts, eight total
creations and one million cumulative pixels. It preserves 2D/WebGL exclusivity,
does not retain discarded canvases, rejects a second owner and restores the
original method after verified native cleanup. The updated rendering and eight
canvas checks cover exact native counts and retained/discarded canvas lifetimes.

`worker-qualification.json` retains the failed setup, memory and integration
runs. The final instrumented SDK run created five contexts, refused none and
passed at 131,796,992 bytes. The deployable version omits diagnostic files and
passes at 120,934,400 bytes. Both used the existing owner, discarded the token,
recorded zero swap/memory events and verified group absence. Graphics status
does not add fields to the strict worker metadata protocol. These are synthetic
results; neither service nor an account was touched and no live delivery is proven.
