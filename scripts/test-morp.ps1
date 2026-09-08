param([string]$Python = 'python')
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    & $Python -m unittest discover -s benchmarks/morp/tests -v
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    cargo build -p momo_core --example morp_contract_probe --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $morpOutput = Join-Path 'target' ('morp-offline-' + [guid]::NewGuid().ToString('N'))
    & $Python -m benchmarks.morp build --out (Join-Path $morpOutput 'dataset')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $Python -m benchmarks.morp plan (Join-Path $morpOutput 'dataset') --config benchmarks/morp/configs/momo.example.json --out (Join-Path $morpOutput 'momo-plan.json')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & $Python -m benchmarks.morp offline --probe target/debug/examples/morp_contract_probe.exe --out (Join-Path $morpOutput 'contracts.json')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Output "Offline artifacts: $morpOutput (AI calls: 0)"
} finally { Pop-Location }
