# afterglow — agent notes

The HDMI screensavers for the `talos-pi5` cluster, published as
`ghcr.io/northisup/afterglow` (public). Deployed from the private
NorthIsUp/homelab-gitops (`k8s/apps/screensaver/`), which pins the image by
digest; `homelab-gitops#N` in old commit messages points at its PRs. This repo
is public: no tailnet names, LAN IPs or secrets in code, docs or commits.

- The frame loop is the product. No liveness probe, no read-backs, no
  observability in the render path — restarting or slowing it is the outage.
- `src/surface.rs`'s module doc is the contract every saver is held to: any
  pixel written but not reported as damaged never reaches the panel.
- `src/font.rs` is generated (`mise run font`); CI fails if it drifts.
- Commit messages: end with the Co-Authored-By trailer.
