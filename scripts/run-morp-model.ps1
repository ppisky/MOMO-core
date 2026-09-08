[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Config,
    [ValidateSet('all', 'memory', 'acgn', 'momo')]
    [string]$Suite = 'memory',
    [ValidateSet('dev', 'eval', 'all')]
    [string]$Split = 'dev',
    [ValidateRange(1, 20)]
    [int]$Repeats = 1,
    [ValidateRange(1, 1000)]
    [int[]]$Horizons = @(50),
    [ValidateRange(1, 100)]
    [int]$Variants = 1,
    [string[]]$Dimensions = @(),
    [string[]]$Families = @(),
    [string[]]$Characters = @(),
    [ValidateSet('label_free', 'labeled', 'labels_only')]
    [string[]]$Arms = @(),
    [string[]]$CaseIds = @(),
    [ValidateSet('context', 'extracted')]
    [string[]]$Dependencies = @(),
    [string]$OutputRoot,
    [switch]$AllowAI,
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
$resolvedConfig = (Resolve-Path -LiteralPath $Config -ErrorAction Stop).Path
$repositoryRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path

Push-Location $repositoryRoot
try {
    if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
        $OutputRoot = Join-Path 'target' ('morp-model-' + [guid]::NewGuid().ToString('N'))
    }
    if (Test-Path -LiteralPath $OutputRoot) {
        throw "OutputRoot already exists; use a new directory: $OutputRoot"
    }
    New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null

    $dataset = Join-Path $OutputRoot 'dataset'
    $plan = Join-Path $OutputRoot 'plan.json'
    $run = Join-Path $OutputRoot 'run'
    $report = Join-Path $OutputRoot 'objective-report.json'
    $horizonArguments = @($Horizons | ForEach-Object { $_.ToString() })

    $buildArguments = @(
        '-m', 'benchmarks.morp', 'build', '--out', $dataset,
        '--suite', $Suite, '--horizons'
    ) + $horizonArguments + @('--variants', $Variants.ToString())
    & $Python @buildArguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & $Python -m benchmarks.morp validate $dataset
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $planArguments = @(
        '-m', 'benchmarks.morp', 'plan', $dataset, '--config', $resolvedConfig,
        '--split', $Split, '--repeats', $Repeats.ToString(), '--out', $plan
    )
    foreach ($dimension in $Dimensions) { $planArguments += @('--dimension', $dimension) }
    foreach ($family in $Families) { $planArguments += @('--family', $family) }
    foreach ($character in $Characters) { $planArguments += @('--character', $character) }
    foreach ($arm in $Arms) { $planArguments += @('--arm', $arm) }
    foreach ($caseId in $CaseIds) { $planArguments += @('--case-id', $caseId) }
    foreach ($dependency in $Dependencies) { $planArguments += @('--dependency', $dependency) }
    & $Python @planArguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $planDocument = Get-Content -Raw -LiteralPath $plan | ConvertFrom-Json
    Write-Output ("MORP plan: protocol={0}; candidate_calls={1}; maintenance_calls={2}" -f `
        $planDocument.protocol, $planDocument.candidate_calls, $planDocument.maintenance_calls)
    Write-Output "Plan: $plan"

    if (-not $AllowAI) {
        Write-Output 'Plan-only mode complete (AI calls: 0). Pass -AllowAI to execute this exact plan.'
        exit 0
    }

    Write-Warning ("AI execution enabled: up to {0} candidate calls, plus any provider-dependent maintenance calls." -f `
        $planDocument.candidate_calls)
    & $Python -m benchmarks.morp run $dataset --plan $plan --out $run --allow-ai
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & $Python -m benchmarks.morp score $dataset --plan $plan `
        --predictions (Join-Path $run 'predictions.jsonl') --out $report
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Output "Candidate predictions: $(Join-Path $run 'predictions.jsonl')"
    Write-Output "Objective-only report: $report"
    $scoreDocument = Get-Content -Raw -LiteralPath $report | ConvertFrom-Json
    Write-Output ("Coverage: {0}/100" -f $scoreDocument.score_summary.coverage)
    Write-Output ("Objective score: {0}/100" -f $(if ($null -eq $scoreDocument.score_summary.objective_score) { 'pending' } else { $scoreDocument.score_summary.objective_score }))
    Write-Output ("Selected score: {0}/100 ({1})" -f $(if ($null -eq $scoreDocument.score_summary.selected_score) { 'pending' } else { $scoreDocument.score_summary.selected_score }), $scoreDocument.score_summary.status)
    Write-Output 'Subjective dimensions remain pending until two independent judge result files or one auditable human adjudication are supplied.'
} finally {
    Pop-Location
}
