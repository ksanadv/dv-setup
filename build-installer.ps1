# Builds dv-setup.exe (suite/) into dist\dv-setup-<version>-windows-x64.exe, with a .sha256 next to it.
# 'Continue' so Windows PowerShell 5.1 does not treat cargo's progress on stderr as a failure; exit codes are checked
$ErrorActionPreference = 'Continue'
Set-Location $PSScriptRoot

$pattern = '^version\s*=\s*"([^"]+)"'
$libVersion = (Select-String -Path Cargo.toml -Pattern $pattern | Select-Object -First 1).Matches.Groups[1].Value
$suiteVersion = (Select-String -Path suite/Cargo.toml -Pattern $pattern | Select-Object -First 1).Matches.Groups[1].Value
if ($libVersion -ne $suiteVersion) { throw "Cargo.toml is $libVersion but suite/Cargo.toml is $suiteVersion; keep them equal" }

# a target folder of its own beside the configured one: the static CRT flag below would otherwise rebuild its contents
$configured = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$target = Join-Path (Split-Path $configured) 'dv-setup'
# no Visual C++ runtime needed on the target machine
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo test --features net --target-dir $target
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cargo build --release --manifest-path suite/Cargo.toml --target-dir $target
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$name = "dv-setup-$libVersion-windows-x64.exe"
New-Item -ItemType Directory -Force dist | Out-Null
Remove-Item dist\dv-setup-* -ErrorAction SilentlyContinue
Copy-Item (Join-Path $target 'release\dv-setup.exe') "dist\$name"
$text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes("$PSScriptRoot\dist\$name"))
if ($text -match '(?i)vcruntime\d*\.dll') { throw "$name imports the Visual C++ runtime" }
$hash = (Get-FileHash "dist\$name" -Algorithm SHA256).Hash.ToLower()
[IO.File]::WriteAllText("$PSScriptRoot\dist\$name.sha256", "$hash  $name`n")
Get-Item dist\$name | ForEach-Object { '{0}  {1:N1} MB' -f $_.FullName, ($_.Length / 1MB) }
