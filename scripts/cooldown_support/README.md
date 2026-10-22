# Browser runtime restoration sources

These five Python modules and the startup template preserve the deployed
browser host's one-use cooldown restoration. They contain source code only.
Private transfer requests, captured ledgers and account records stay outside
the repository.

Prepare a package without importing the host, touching account state or
starting a process:

```sh
python3 -B scripts/prepare_browser_runtime.py runtime/browser-package
```

The output directory must be new. It contains the generated host and a
hash-named support directory. The normal Rust build still embeds the base
`scripts/camoufox_host.py`; the generated package is a separate deployment
input. Keep both pieces together when preparing a coordinated release.

The initial published sources reproduce sidecar SHA-256
`7d76f06bdf7f03da87eef6d54fc14cef45d0e6e5b0557fe6fae88fa6143f73eb`
and support-manifest SHA-256
`cec55f5d28e2155f6db5c4217d7853620d5d4c46020db8a46a59f4c0aea09f8b`.

Package generation does not install, restart, capture, drain or restore a
live service. Those actions still require the deployment's existing ownership
and recovery procedure.
