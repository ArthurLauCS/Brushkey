#!/bin/sh
# Development install (admin shell): builds both DLLs, the settings app and the
# dictionary, copies them to C:\Program Files\Brushkey and registers the DLLs.
# `-u` unregisters.
set -e
cd "$(dirname "$0")/.."
dst="/c/Program Files/Brushkey"
reg64() { regsvr32 //s "$@"; }
reg32() { /c/Windows/SysWOW64/regsvr32 //s "$@"; }
if [ "$1" = -u ]; then
    reg64 //u "$(cygpath -w "$dst/brushkey.dll")"
    reg32 //u "$(cygpath -w "$dst/brushkey32.dll")"
    exit
fi
cargo build --release -p ime -p settings
cargo build --release -p ime --target i686-pc-windows-msvc
[ -f target/dict.bin ] || cargo run --release -p engine --bin dictgen -- data target/dict.bin
mkdir -p "$dst"
# Running apps keep the old files loaded: those can be renamed, not overwritten.
for f in brushkey.dll brushkey32.dll brushkey-settings.exe dict.bin; do
    if [ -f "$dst/$f" ]; then mv "$dst/$f" "$dst/$f.$(date +%s%N).old"; fi
done
rm -f "$dst"/*.old 2>/dev/null || true # the ones still loaded go next time
cp target/release/brushkey.dll "$dst/brushkey.dll"
cp target/i686-pc-windows-msvc/release/brushkey.dll "$dst/brushkey32.dll"
cp target/release/brushkey-settings.exe "$dst/brushkey-settings.exe"
cp target/dict.bin "$dst/dict.bin"
reg64 "$(cygpath -w "$dst/brushkey.dll")"
reg32 "$(cygpath -w "$dst/brushkey32.dll")"
