#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
./scripts/bootstrap.sh

if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    cargo test --locked --tests
elif command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a cargo test --locked --tests
else
    printf '%s\n' 'GUI tests require a display or xvfb-run. Install Xvfb or run from a desktop session.' >&2
    exit 1
fi

cargo build --locked --release
printf '%s\n' 'Built target/release/rstpd (requires the GTK3 system libraries).'
