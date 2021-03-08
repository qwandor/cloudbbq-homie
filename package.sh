#!/bin/bash
#
# Copyright 2020 the cloudbbq-homie authors.
# This project is dual-licensed under Apache 2.0 and MIT terms.
# See LICENSE-APACHE and LICENSE-MIT for details.

# Build a Debian package for the given target, or the default host target if none is set.

set -euo pipefail

TARGET=${TARGET:-}

if [ -z "$TARGET" ]; then
  cargo deb
else
  cross build --release --target "$TARGET" --bin cloudbbq-homie
  cargo deb --target "$TARGET" --no-build
fi
