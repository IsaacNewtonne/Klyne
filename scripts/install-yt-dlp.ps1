# Installs (or updates) yt-dlp.exe into Klyne's tools folder for the
# video_transcript tool. No global install, no pip, no login or cookies.
# The download is verified against the release's SHA2-256SUMS file.
#   powershell -ExecutionPolicy Bypass -File scripts\install-yt-dlp.ps1
param([string]$Destination = (Join-Path (Split-Path -Parent $PSScriptRoot) 'tools\yt-dlp'))
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$base = 'https://github.com/yt-dlp/yt-dlp/releases/latest/download'
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$exe = Join-Path $Destination 'yt-dlp.exe'
$tmp = Join-Path $Destination 'yt-dlp.exe.download'
$sums = Join-Path $Destination 'SHA2-256SUMS'
Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA2-256SUMS" -OutFile $sums
Invoke-WebRequest -UseBasicParsing -Uri "$base/yt-dlp.exe" -OutFile $tmp
$line = Get-Content $sums | Where-Object { $_ -match '\s\*?yt-dlp\.exe$' } | Select-Object -First 1
if (-not $line) { Remove-Item $tmp; throw 'yt-dlp.exe is missing from SHA2-256SUMS' }
$expected = ($line -split '\s+')[0].ToLowerInvariant()
$actual = (Get-FileHash -Algorithm SHA256 $tmp).Hash.ToLowerInvariant()
if ($expected -ne $actual) { Remove-Item $tmp; throw "SHA-256 mismatch: expected $expected, got $actual" }
Move-Item -Force $tmp $exe
$version = & $exe --ignore-config --version
Write-Output "yt-dlp $version installed at $exe (sha256 $actual)"
