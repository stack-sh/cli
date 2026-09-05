# Aqua registry maintenance

[`registry.yaml`](./registry.yaml) maps the `stack-sh/cli` package to the canonical Stack CLI GitHub Release archives. It does not rebuild or repack the executable. Aqua downloads the release checksum inventory, requires the selected archive to have a SHA-256 entry, and verifies the keyless Sigstore bundle against the tagged release workflow identity.

The registry supports only the release contract's tier-1 environments: macOS and glibc-based Linux on arm64 and x86_64. The package name is `stack-sh/cli`, and the installed command remains `stack`.

## Verify a registry change

Install the pinned Aqua version used by CI, then test every supported mapping without executing a foreign-architecture binary:

```sh
aqua update-checksum
aqua update
for environment in darwin/amd64 darwin/arm64 linux/amd64 linux/arm64; do
  AQUA_CONFIG=tests/aqua/aqua.yaml \
    AQUA_POLICY_CONFIG=tests/aqua/aqua-policy.yaml \
    AQUA_GOOS="${environment%/*}" \
    AQUA_GOARCH="${environment#*/}" \
    aqua install --test
done
```

`aqua update-checksum` must reproduce `tests/aqua/aqua-checksums.json` exactly. The file locks all four release archives to the SHA-256 values obtained from the release checksum asset after Aqua verifies its Sigstore bundle. `aqua update` must leave the pinned fixture unchanged until a newer stable release exists.

On the native host, repeat without `--test` in an isolated `AQUA_ROOT_DIR`, then run `stack --version`, `stack init`, `stack check`, and `stack render`.

## Publish an update

Release archives and checksums remain owned by the immutable GitHub Release. If the artifact naming contract changes, update this registry and all four environment tests in one pull request. After merge, pin the owner registry in user documentation to the resulting full commit SHA. Never use a branch name as a registry `ref`, replace release assets, or point Aqua at repacked bytes.
