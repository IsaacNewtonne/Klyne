param([switch]$SkipModels)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$voiceHome = Join-Path $repo '.voice'
$python = Join-Path $voiceHome 'runtime\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $python)) {
    & python -m venv (Join-Path $voiceHome 'runtime')
    if ($LASTEXITCODE -ne 0) { throw 'Could not create the voice environment.' }
}
& $python -m pip install -r (Join-Path $PSScriptRoot 'voice-requirements.txt')
if ($LASTEXITCODE -ne 0) { throw 'Voice dependency installation failed.' }
if (-not $SkipModels) {
    & $python (Join-Path $PSScriptRoot 'voice-worker.py') --setup --home $voiceHome
    if ($LASTEXITCODE -ne 0) { throw 'Voice model preparation failed.' }
}
if ($SkipModels) { Write-Host 'Voice runtime installed. Run again without -SkipModels to prepare and test models.' }
else { Write-Host 'Local voice is ready. Open Klyne Settings > Voice, then Check voice.' }
