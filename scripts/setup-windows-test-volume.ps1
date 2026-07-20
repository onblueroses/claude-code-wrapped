[CmdletBinding()]
param(
    [switch]$Detach
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-DiskPart {
    param(
        [Parameter(Mandatory)]
        [string[]]$Commands
    )

    $scriptPath = Join-Path $env:RUNNER_TEMP (
        "ccwrapped-diskpart-" + [Guid]::NewGuid().ToString("N") + ".txt"
    )
    try {
        [System.IO.File]::WriteAllLines(
            $scriptPath,
            $Commands,
            [System.Text.Encoding]::ASCII
        )
        $output = & diskpart.exe /s $scriptPath
        $exitCode = $LASTEXITCODE
        $output | Write-Host
        if ($exitCode -ne 0) {
            throw "diskpart exited with status $exitCode"
        }
    }
    finally {
        Remove-Item -LiteralPath $scriptPath -Force -ErrorAction SilentlyContinue
    }
}

if ($Detach) {
    $vhdPath = $env:CCWRAPPED_WINDOWS_TEST_VHD
    if ([string]::IsNullOrWhiteSpace($vhdPath)) {
        return
    }
    if (Test-Path -LiteralPath $vhdPath) {
        Invoke-DiskPart -Commands @(
            "select vdisk file=`"$vhdPath`"",
            "detach vdisk"
        )
        Remove-Item -LiteralPath $vhdPath -Force -ErrorAction SilentlyContinue
    }
    return
}

if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or
    [string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
    throw "the Windows test volume requires RUNNER_TEMP and GITHUB_ENV"
}

$usedLetters = @(
    Get-PSDrive -PSProvider FileSystem | ForEach-Object { $_.Name }
)
$driveLetter = @("W", "V", "U", "T", "S", "R") |
    Where-Object { $usedLetters -notcontains $_ } |
    Select-Object -First 1
if ([string]::IsNullOrWhiteSpace($driveLetter)) {
    throw "no drive letter is available for the Windows test volume"
}

$vhdPath = Join-Path $env:RUNNER_TEMP (
    "ccwrapped-tests-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT.vhdx"
)
"CCWRAPPED_WINDOWS_TEST_VHD=$vhdPath" | Add-Content -Path $env:GITHUB_ENV

Invoke-DiskPart -Commands @(
    "create vdisk file=`"$vhdPath`" maximum=256 type=expandable",
    "select vdisk file=`"$vhdPath`"",
    "attach vdisk",
    "create partition primary",
    "format fs=ntfs quick label=CCWRAPPED_TEST",
    "assign letter=$driveLetter"
)

$volumeRoot = "${driveLetter}:\"
if (-not (Test-Path -LiteralPath $volumeRoot)) {
    throw "diskpart did not mount the Windows test volume at $volumeRoot"
}

$currentSid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$grant = "*" + $currentSid + ":(OI)(CI)F"
& icacls.exe $volumeRoot /inheritance:r /grant:r $grant
if ($LASTEXITCODE -ne 0) {
    throw "failed to protect the Windows test volume root"
}

$testRoot = Join-Path $volumeRoot "fixtures"
New-Item -ItemType Directory -Path $testRoot | Out-Null
"CCWRAPPED_WINDOWS_TEST_ROOT=$testRoot" | Add-Content -Path $env:GITHUB_ENV
