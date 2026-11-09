#!/bin/sh
# The hook build-release runs for an apt-only release: build, then package.
# The last line of output is the path of the .deb.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
cargo build --release --locked >&2
exec ./packaging/debian/build-deb.sh
