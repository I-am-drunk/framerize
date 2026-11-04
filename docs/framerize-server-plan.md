# Framerize server and Remote

The server owns provider accounts, session authentication and Castle. Remote
forwards client requests to the server through the shared HTTP core. It does
not contain Castle or provider credentials.

Castle is included in the server package. Normal builds and installed use do
not fetch the separate Castle repository.

- [TypeScript layout](typescript-layout.md) describes package boundaries.
- [Server setup](../framerize-server/README.md) covers configuration and embedding.
- [Remote setup](../framerize-remote/README.md) covers client-side forwarding.
- [Inference validation](inference-validation.md) distinguishes synthetic checks from live qualification.

Keep credentials in private configuration and use only authorized provider
accounts. Passing local tests does not establish live-provider compatibility
or qualify an untested package or executable for release.
