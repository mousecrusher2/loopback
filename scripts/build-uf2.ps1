param(
    [ValidateSet("debug", "release")]
    [string]$BuildProfile = "release",

    [string]$Output = "",

    [switch]$VerboseUf2
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Target = "thumbv8m.main-none-eabihf"
$BinName = "pico2-uac1-loopback"

$Picotool = Get-Command picotool -ErrorAction SilentlyContinue
$Elf2Uf2 = Get-Command elf2uf2-rs -ErrorAction SilentlyContinue
if (-not $Picotool -and -not $Elf2Uf2) {
    throw "Neither picotool nor elf2uf2-rs was found. Install picotool, or install the fallback with: cargo install elf2uf2-rs"
}

$BuildArgs = @("build", "--bin", $BinName, "--target", $Target)
if ($BuildProfile -eq "release") {
    $BuildArgs += "--release"
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
    if ($Picotool) {
        $Uf2Tool = $Picotool
        $Uf2Args = @("uf2", "convert")
    } else {
        $Uf2Tool = $Elf2Uf2
    }
    if ($VerboseUf2) {
        $Uf2Args += "--verbose"
    }
    if ($Picotool) {
        $Uf2Args += @($ElfPath, "-t", "elf", $OutputPath)
    } else {
        $Uf2Args += @($ElfPath, $OutputPath)
    }

    & $Uf2Tool.Source @Uf2Args
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    Write-Host "UF2 written to $OutputPath"
} finally {
    Pop-Location
}
