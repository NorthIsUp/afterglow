# Building and publishing the image

Built and pushed by CI (`.github/workflows/image.yml`) on any non-docs change:
arm64-native, running `cargo fmt --check`, `clippy -D warnings`, the tests and a
dump render, then publishing `ghcr.io/northisup/afterglow` as `latest` and
`sha-<commit>`. The image is public; pulling it needs no credentials.

Then bump the `image:` digest in homelab-gitops'
`k8s/apps/screensaver/deployment.yaml` — in its own commit, with no
env changes in it, so the new binary always runs against the old env block first.

Clippy runs `clippy::pedantic` from the `[lints]` table in `Cargo.toml`, so CI,
hk and `mise run clippy` share one lint set; each allowed lint carries its
reason there.
