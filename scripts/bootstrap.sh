#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$root"

for tool in sha256sum unzip; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'Missing bootstrap dependency: %s\n' "$tool" >&2
        exit 1
    fi
done

sha256sum --check <<'CHECKSUMS'
A0C0CDF1CF226DC6252020CE9A87A939A8B615E65269DB40AC9123B28FA20A9D  vendor/scintilla566.zip
2092B1DD18355321717E3BDE25148E4C87E691723CA2B06A65E29A307C5462A6  vendor/lexilla553.zip
84D7DBE9D9CEB34961AD6FBDCF91A24F7CC1A2FD59D1851C0F5F4A0CDBFDE2F9  vendor/scite566.zip
CHECKSUMS

if [[ ! -d vendor/scintilla ]]; then
    unzip -q vendor/scintilla566.zip -d vendor
fi
if [[ ! -d vendor/lexilla ]]; then
    unzip -q vendor/lexilla553.zip -d vendor
fi
mkdir -p vendor/language-data
unzip -oqj vendor/scite566.zip 'scite/src/*.properties' 'scite/License.txt' \
    -d vendor/language-data

printf 'Pinned Scintilla, Lexilla, and SciTE language data are ready. Run cargo build --release.\n'
