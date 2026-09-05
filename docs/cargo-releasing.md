# Initial crates.io publication

The initial publication creates `stack-diagram-cli` version `0.5.1`. The workflow is deliberately limited to this bootstrap operation; it is not the recurring release mechanism.

1. Merge the release preparation through a reviewed pull request and wait for both main CI jobs to succeed.
2. Create a short-lived crates.io token limited to `publish-new` and the exact crate name `stack-diagram-cli`. Store it only as the repository Actions secret `CARGO_INITIAL_PUBLISH_TOKEN`; never paste it into an issue, pull request, workflow input, or source file.
3. Dispatch `initial-publish.yaml` on `main`, with `expected_sha` equal to the full successful main commit. The workflow rejects a different ref, commit, package identity, initial version, or CI state. It requires the crate name to be absent and performs a credential-free packaging dry run before publishing.
4. Verify the registry version, checksum, and downloaded `.cargo_vcs_info.json`. After publishing, the workflow installs the exact registry package with an empty Cargo home on all four supported native targets, using both Rust 1.85.0 and stable, and checks commands, generated assets, and JSON output. If the upload or verification times out, inspect the registry before retrying: a failure after upload does not undo publication. Re-run only failed verification jobs, not the successful publish job.
5. Remove the GitHub bootstrap secret and revoke the crates.io token. Configure a crates.io trusted publisher for the ongoing release workflow before any later publication. Do not reuse this initial workflow for updates or broaden the bootstrap token.

The token is supplied only to the publication step through `CARGO_REGISTRY_TOKEN`; the workflow never runs `cargo login` or writes a credentials file. It cannot configure trusted publishing on behalf of a crate owner. See the [Cargo publication reference](https://doc.rust-lang.org/cargo/commands/cargo-publish.html) for upload and timeout behavior.
