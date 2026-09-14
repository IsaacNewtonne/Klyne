param([switch]$Browser)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$stamp = [DateTime]::UtcNow.ToString('yyyy-MM-dd-HHmmss')
$log = Join-Path $root ("workspace/test-baseline-$stamp.log")
New-Item -ItemType Directory -Force (Split-Path -Parent $log) | Out-Null

function Write-Both($text) {
  $text | Tee-Object -FilePath $log -Append | Out-Null
  Write-Host $text
}

"=== Klyne test baseline $stamp (UTC) ===" | Set-Content -LiteralPath $log
Write-Both "repo: $root"
try { Write-Both ("rustc: " + (rustc -V)) } catch { Write-Both "rustc: missing ($($_.Exception.Message))" }
try { Write-Both ("cargo: " + (cargo -V)) } catch { Write-Both "cargo: missing ($($_.Exception.Message))" }
try { Write-Both ("toolchain: " + ((rustup show active-toolchain) -join ' ')) } catch { Write-Both "toolchain: rustup unavailable" }
try { Write-Both ("os: " + ((Get-CimInstance Win32_OperatingSystem).Caption + ' ' + (Get-CimInstance Win32_OperatingSystem).Version)) } catch { Write-Both "os: unknown" }
$chrome = @('C:\Program Files\Google\Chrome\Application\chrome.exe', 'C:\Program Files (x86)\Google\Chrome\Application\chrome.exe') | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($chrome) { Write-Both ("chrome: " + (Get-Item -LiteralPath $chrome).VersionInfo.ProductVersion) } else { Write-Both "chrome: not found (browser tests require Chrome)" }

Write-Both "--- Mixed workspace baseline (includes Studio Chrome and desktop tests) ---"
Set-Location -LiteralPath $root
# Windows PowerShell treats native stderr as ErrorRecords. Cargo writes normal
# progress there; record it without terminating, and use the native exit code.
Get-Command cargo -ErrorAction Stop | Out-Null
$ErrorActionPreference = 'Continue'
cargo test --workspace --exclude harness-browser --locked --offline --no-fail-fast 2>&1 | Tee-Object -FilePath $log -Append
$baselineExit = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($baselineExit -eq 0) { $tier0 = 'PASS' } else { $tier0 = 'FAIL' }
Write-Both "Mixed baseline result: $tier0 (cargo exit $baselineExit)"
$browserExit = 0

if ($Browser) {
  Write-Both "--- Tier 1: browser (Chrome required) ---"
  $ErrorActionPreference = 'Continue'
  cargo test -p harness-browser --test controlled --locked --offline -- --test-threads=1 2>&1 | Tee-Object -FilePath $log -Append
  $browserExit = $LASTEXITCODE
  $ErrorActionPreference = 'Stop'
  if ($browserExit -eq 0) { Write-Both "Tier 1 result: PASS" } else { Write-Both "Tier 1 result: FAIL (cargo exit $browserExit)" }
} else {
  Write-Both "Tier 1 skipped (pass -Browser to include Chrome suite). Tier 2 (live MCP, --ignored) and Tier 3 (desktop GUI) never run here; see docs/test-tiers.md."
}

Write-Both "log: $log"
if ($tier0 -ne 'PASS' -or $browserExit -ne 0) { exit 1 }
