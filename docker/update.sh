#!/usr/bin/env bash
#
# Copyright 2021 the cloudbbq-homie authors.
# This project is dual-licensed under Apache 2.0 and MIT terms.
# See LICENSE-APACHE and LICENSE-MIT for details.

set -euo pipefail

VERSION=0.2.1-1

docker build ./docker -f docker/Dockerfile.debian-buster-aarch64 \
    -t ghcr.io/qwandor/cross-dbus-debian-buster-aarch64:$VERSION
docker build ./docker -f docker/Dockerfile.debian-buster-armv7 \
    -t ghcr.io/qwandor/cross-dbus-debian-buster-armv7:$VERSION
docker build ./docker -f docker/Dockerfile.debian-buster-x86_64 \
    -t ghcr.io/qwandor/cross-dbus-debian-buster-x86_64:$VERSION

docker push ghcr.io/qwandor/cross-dbus-debian-buster-aarch64:$VERSION
docker push ghcr.io/qwandor/cross-dbus-debian-buster-armv7:$VERSION
docker push ghcr.io/qwandor/cross-dbus-debian-buster-x86_64:$VERSION
