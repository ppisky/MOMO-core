param([switch]$SkipAudit)

$ErrorActionPreference = 'Stop'
$taskRepo = Split-Path -Parent $PSScriptRoot
Push-Location $taskRepo

function Invoke-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo failed: $args" }
}

try {
    Get-Content -LiteralPath 'contracts/1.0/SHA256SUMS' | ForEach-Object {
        $parts = $_ -split '\s+', 2
        $contractPath = Join-Path 'contracts/1.0' $parts[1].Trim()
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath $contractPath).Hash.ToLowerInvariant() -ne $parts[0]) {
            throw "Contract checksum mismatch: $contractPath"
        }
    }
    Invoke-Cargo @('fmt', '--all', '--', '--check')
    Invoke-Cargo @('clippy', '--workspace', '--all-targets', '--all-features', '--locked', '--', '-D', 'warnings')
    Invoke-Cargo test --workspace --all-features --locked
    & "$PSScriptRoot/test-morp.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'MORP offline verification failed' }
    Invoke-Cargo doc --workspace --all-features --no-deps --locked
    Invoke-Cargo build --release --workspace --all-features --locked
    if ($SkipAudit) { Write-Warning 'RustSec audit explicitly skipped; this is not a complete release gate.' }
    else { Invoke-Cargo audit }

    $serverPath = Join-Path $taskRepo 'target/release/momo-server.exe'
    $binaryText = [Text.Encoding]::Latin1.GetString([IO.File]::ReadAllBytes($serverPath))
    foreach ($removed in @('MOMO_SCOPE_ID', '01900000-0000-7000-8000-000000000101', 'character_catalogue', 'character-catalogue')) {
        if ($binaryText.Contains($removed)) { throw "Removed identity remains in release binary: $removed" }
    }
    $smokeRoot = Join-Path $taskRepo ('target/release-smoke-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $smokeRoot | Out-Null
    $portProbe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $portProbe.Start()
    $smokePort = $portProbe.LocalEndpoint.Port
    $portProbe.Stop()
    $previousData = $env:MOMO_DATA_DIR
    $previousBind = $env:MOMO_SERVER_BIND
    $smokeProcess = $null
    try {
        $env:MOMO_DATA_DIR = Join-Path $smokeRoot 'data'
        $env:MOMO_SERVER_BIND = "127.0.0.1:$smokePort"
        $smokeProcess = Start-Process -FilePath $serverPath -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $smokeRoot 'stdout.log') -RedirectStandardError (Join-Path $smokeRoot 'stderr.log')
        $healthy = $false
        for ($attempt = 0; $attempt -lt 100; $attempt++) {
            if ($smokeProcess.HasExited) { break }
            try {
                $health = Invoke-RestMethod "http://127.0.0.1:$smokePort/health" -TimeoutSec 1
                if ($health.ok -eq $true -and $health.service -eq 'momo-server') { $healthy = $true; break }
            } catch { Start-Sleep -Milliseconds 100 }
        }
        if (-not $healthy) { throw "Release health smoke failed; logs: $smokeRoot" }
        $recovery = Invoke-RestMethod "http://127.0.0.1:$smokePort/v1/memory/recovery" -TimeoutSec 3
        if (($recovery | ConvertTo-Json -Compress) -ne '{}') { throw 'Fresh runtime reported a recovery conflict' }
        Write-Output "Release smoke passed: $smokeRoot"
        Get-FileHash -Algorithm SHA256 -LiteralPath $serverPath
    } finally {
        if ($null -ne $smokeProcess -and -not $smokeProcess.HasExited) { Stop-Process -Id $smokeProcess.Id; $smokeProcess.WaitForExit() }
        $env:MOMO_DATA_DIR = $previousData
        $env:MOMO_SERVER_BIND = $previousBind
    }
} finally { Pop-Location }
