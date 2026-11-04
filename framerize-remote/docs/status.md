# Remote notes

Use the [Remote guide](../README.md) for setup, saved profiles and CLI commands.
The [development status](../../docs/wip-status.md) lists project-wide verification
limits and release prerequisites.

Remote exposes a local client API and forwards requests to the server. It does
not perform provider inference, load provider accounts or include Castle.
Its local client token is separate from the token used to reach the server.

## Historical evidence

The [retained release rehearsal](release-evidence/0.1.9-671ed7df/README.md)
records an earlier package name, source revision and artifact hashes. It does
not qualify or publish the current Remote package.

The previous development and deployment log remains in Git history. Its
recorded services, live requests and package names do not describe the current
installation. Use the [publication guide](../../docs/remote-publication.md)
for the current release procedure. Publication and live provider verification
remain separate from synthetic source and installed-package checks.
