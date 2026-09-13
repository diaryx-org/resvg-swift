---
title: iOS type-check script
description: Nothing type-checks the Swift package against the iOS triple; leaf has the script to copy
author: adammharris
status: open
created: 2026-09-12
updated: 2026-09-12
part_of: '[Tasks](tasks.md)'
---
# iOS type-check script

`scripts/test-swift.sh` builds and runs the Swift tests on the macOS host. The
package declares iOS 16 and `ResvgCoreGraphics` uses only CoreGraphics and
ImageIO, so it should compile there unchanged — but nothing checks. leaf's
`scripts/check-swift.sh` type-checks its package against the iOS-simulator
triple without an Xcode project; the same script, renamed, belongs here, and
`ci.yml`'s `swift` job should run it after the tests.
