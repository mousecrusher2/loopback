param(
    [ValidateSet("debug", "release")]
    [Alias("Profile")]
    [string]$BuildProfile = "release",

    [string[]]$Features = @(),

    [string]$Output = "",

    [switch]$NoDefaultFeatures,

    [switch]$VerboseUf2
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Target = "thumbv8m.main-none-eabihf"
$BinName = "pico2-uac1-loopback"

$Elf2Uf2 = Get-Command elf2uf2-rs -ErrorAction SilentlyContinue
if (-not $Elf2Uf2) {
    throw "elf2uf2-rs was not found. Install it with: cargo install elf2uf2-rs"
}

$BuildArgs = @("build", "--bin", $BinName, "--target", $Target)
if ($BuildProfile -eq "release") {
    $BuildArgs += "--release"
}
if ($NoDefaultFeatures) {
    $BuildArgs += "--no-default-features"
}
if ($Features.Count -gt 0) {
    $BuildArgs += @("--features", ($Features -join ","))
}

Push-Location $RepoRoot
try {
    & cargo @BuildArgs
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    $ProfileDir = if ($BuildProfile -eq "release") { "release" } else { "debug" }
    $ElfPath = Join-Path $RepoRoot "target\$Target\$ProfileDir\$BinName"
    if (-not (Test-Path $ElfPath)) {
        throw "ELF output was not found: $ElfPath"
    }

    if ([string]::IsNullOrWhiteSpace($Output)) {
        $Suffix = if ($BuildProfile -eq "release") { "" } else { "-$BuildProfile" }
        $Output = "target\uf2\$BinName$Suffix.uf2"
    }

    $OutputPath = if ([System.IO.Path]::IsPathRooted($Output)) {
        $Output
    } else {
        Join-Path $RepoRoot $Output
    }
    $OutputDir = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force $OutputDir | Out-Null

    $Uf2Args = @()
    if ($VerboseUf2) {
        $Uf2Args += "--verbose"
    }
    $Uf2Args += @($ElfPath, $OutputPath)

    & $Elf2Uf2.Source @Uf2Args
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    Write-Host "UF2 written to $OutputPath"
} finally {
    Pop-Location
}
