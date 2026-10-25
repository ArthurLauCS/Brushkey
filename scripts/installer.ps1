# Builds both DLLs, the settings app and the dictionary, then the installer:
# target\installer\Brushkey-Setup-<version>.exe. Needs Inno Setup 6 and the i686-pc-windows-msvc target.
$root = "$PSScriptRoot\.."
$out = "$root\target\installer"
# keep only this version's installer
if (Test-Path $out) { [IO.Directory]::Delete($out, $true) }
$manifest = "$root\Cargo.toml"
cargo build --release -p ime -p settings --manifest-path $manifest
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cargo build --release -p ime --target i686-pc-windows-msvc --manifest-path $manifest
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cargo run --release -p engine --bin dictgen --manifest-path $manifest -- "$root\data" "$root\target\dict.bin"
if ($LASTEXITCODE) { exit $LASTEXITCODE }
$version = (Select-String -Path "$root\crates\ime\Cargo.toml" -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
# Inno Setup may be installed per machine or per user
$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
& $iscc /Q "/DVersion=$version" "$root\installer\brushkey.iss"
exit $LASTEXITCODE
