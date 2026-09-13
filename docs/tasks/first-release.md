---
title: First release
description: What stands between this checkout and a 0.1.0 leaf can pin — the repo, the token, the number
author: adammharris
status: open
created: 2026-09-12
updated: 2026-09-12
part_of: '[Tasks](tasks.md)'
---
# First release

What stands between this checkout and a `0.1.0` that leaf can pin:

- The GitHub repository `diaryx-org/resvg-swift`, and this checkout pushed to
  it. Listed in `~/diaryx/repos.figl` already; `dx clone --check` reports it
  missing until it exists.
- The `CARGO_REGISTRY_TOKEN` secret on the repo, with `publish-new` scope —
  both crates are new to crates.io. `publish.yml` names it.
- The version. `dx release` with no spec proposes; the number is Adam's.
- Then `dx release <spec>` cuts bump, changelog, commit, and tag, and the tag
  push runs `publish.yml`.

After that, leaf's task (`leaf/docs/tasks/svg-through-resvg-swift.md`) can
take the pin.
