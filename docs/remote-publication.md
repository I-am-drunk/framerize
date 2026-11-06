# Publish an approved Remote package

Publication is separate from candidate creation. The manual `publish-remote`
workflow runs only from `main`, behind the `npm-release` environment. It copies
the original candidate tarball after checking its hashes. It never rebuilds or
repacks the product. Publication defaults to off.

No approval record is supplied by this change. Local tests and the
single-endpoint validator do not establish live browserless acceptance.

## Record the live review

After the approved live integration passes, retain a redacted report at
`release-approvals/framerize-remote-live.md`. Record the exact candidate run,
commit, Remote archive hash, paired server artifact and its hash, commands,
results and limitations. The reviewer must check installed Remote against the
actual approved server and account. Keep credentials and private account details
outside the repository. A generic endpoint response alone is insufficient.

Commit the matching record at `release-approvals/framerize-remote.json` through
review. This template deliberately fails approval:

```json
{
  "schemaVersion": 1,
  "decision": "pending",
  "candidateRunId": "REVIEWED_RUN_ID",
  "candidateManifestSha256": "SHA256_OF_CANDIDATE_RELEASE_JSON",
  "sourceSha": "CANDIDATE_SOURCE_COMMIT",
  "reviewedBy": "REVIEWER_GITHUB_USERNAME",
  "evidenceFile": "framerize-remote-live.md",
  "evidenceSha256": "SHA256_OF_REVIEWED_LIVE_REPORT"
}
```

Only the completed review permits `decision: "approve-npm-publication"`.
The verifier checks the record's bindings, not the truth of a human report.
Environment reviewers must inspect that report before allowing publication.

## Prepare the exact archive

1. Configure required reviewers on the repository's `npm-release` environment.
   A missing reviewer rule fails verification, even if GitHub starts the job.
2. Run `publish-remote` from `main`. Supply the successful candidate run ID and
   the reviewed SHA-256 of its `release.json`. Leave `publish` set to false.
3. Review `remote-promotion-<run ID>`. It contains the unchanged tarball and
   `promotion.json`, with the archive hash, source commit and approval commit.

The verifier requires the successful `release.yml` run from this repository,
the exact run attempt, all nine platform/runtime reports, their retained logs,
the candidate checksum list and the committed live review. It checks every
retained byte against the pinned manifest. It does not execute candidate code.
Installed test logs keep assembly's 16 MiB limit during promotion. Other
candidate evidence keeps its 2 MiB limit.

The package must be `framerize-remote`, with no runtime dependencies or
`publishConfig` overrides. Prerelease versions use the `next` dist-tag;
stable versions use `latest`. Neither path changes the package bytes.

## Configure npm and publish

The workflow uses npm trusted publishing with OIDC, not a stored npm token.
Configure the package's trusted publisher for this repository,
`publish-remote.yml`, the `npm-release` environment, and direct `npm publish`
permission. See [npm's trusted publisher guide](https://docs.npmjs.com/trusted-publishers/).
The job pins Node 24.13.0 and requires its npm 11.6.2.

The registry returned 404 for `framerize-remote` at this development checkpoint.
That does not establish ownership or name availability. The package owner must
complete npm's initial ownership/publication setup before configuring this
package's trusted publisher. Any first upload must use the approved archive and
dist-tag from `promotion.json`, with private credentials. Do not repack it.

If the initial setup already published this version, do not publish it again.
For subsequent approved versions, run this workflow with that version's
candidate inputs and `publish: true`. Environment approval is still required.
The workflow publishes the prepared tarball, checks the
registry's SHA-512 integrity and runs its installed CLI help through npm exec.

The npm command disables lifecycle scripts, uses an empty user/global
configuration, and fixes the public registry. It has no npm-token fallback.
The registry check uses a fresh cache outside the checkout.

Automatic npm build provenance is disabled deliberately. This workflow promotes
bytes built by an earlier candidate run; its current commit is the approval
commit, not the package source commit. Claiming it built the package would be
misleading. `promotion.json` records both identities and the exact archive hash.
This does not claim an npm build-provenance attestation.

Do not blindly retry a failed publication job. A failure after the upload may
mean the version already exists. Inspect the registry integrity and retained
logs first. npm versions are immutable; this workflow never unpublishes or
overwrites them. Registry CLI help checks installation only, not live inference.

This publishing path still needs hosted qualification and configured ownership.
No live approval, registry publication or account-setting change has occurred
as part of its local synthetic tests.
