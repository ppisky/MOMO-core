param(
    [ValidateRange(1, 100000)][int[]]$Documents = @(100, 1000),
    [ValidateRange(5, 10000)][int]$Trials = 25,
    [ValidateRange(1, 1000)][int]$LifecycleTrials = 5,
    [ValidateRange(1, 100000)][int]$VectorCount = 5000
)

$ErrorActionPreference = 'Stop'
$taskSavedEnvironment = @{}
foreach ($name in @('HOTPATH_METRICS_SERVER_OFF', 'HOTPATH_OUTPUT_FORMAT', 'HOTPATH_OUTPUT_PATH')) {
    $taskSavedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
Push-Location (Join-Path $PSScriptRoot '..')
try {
    # Compile everything before timing; never run workloads concurrently.
    cargo build --release -p momo-memory -p momo-storage --examples --all-features --locked
    if ($LASTEXITCODE -ne 0) { throw 'Performance examples failed to build' }
    $taskOutput = Join-Path (Get-Location) ('target/core-profile-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $taskOutput | Out-Null
    @(
        "revision=$(git rev-parse HEAD)"
        "working_tree=$(git status --porcelain | Out-String)"
        "rustc=$(rustc --version)"
        "cargo=$(cargo --version)"
        "os=$([Environment]::OSVersion.VersionString)"
        "cpu=$((Get-CimInstance Win32_Processor).Name)"
        "documents=$($Documents -join ',') trials=$Trials lifecycle_trials=$LifecycleTrials vectors=$VectorCount"
        'Mode: release; hotpath function timing only; synthetic data; no model calls'
    ) | Set-Content (Join-Path $taskOutput 'environment.txt')

    $env:HOTPATH_METRICS_SERVER_OFF = '1'
    $env:HOTPATH_OUTPUT_FORMAT = 'json-pretty'
    foreach ($count in $Documents) {
        $env:HOTPATH_OUTPUT_PATH = Join-Path $taskOutput "memory-$count.json"
        & target/release/examples/retrieval_benchmark.exe $count $Trials $LifecycleTrials |
            Tee-Object -FilePath (Join-Path $taskOutput "memory-$count.txt")
        if ($LASTEXITCODE -ne 0) { throw "Memory benchmark failed for $count documents" }
    }
    $env:HOTPATH_OUTPUT_PATH = Join-Path $taskOutput 'vectors.json'
    & target/release/examples/vector_benchmark.exe $VectorCount 384 64 $Trials |
        Tee-Object -FilePath (Join-Path $taskOutput 'vectors.txt')
    if ($LASTEXITCODE -ne 0) { throw 'Vector benchmark failed' }
    Write-Output "Performance reports: $taskOutput"
} finally {
    foreach ($name in $taskSavedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $taskSavedEnvironment[$name], 'Process')
    }
    Pop-Location
}
