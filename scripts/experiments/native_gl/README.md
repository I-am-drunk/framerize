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
