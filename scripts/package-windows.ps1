$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Invoke-Exact {
    param(
        [Parameter(Mandatory = $true)] [string] $Program,
        [Parameter(ValueFromRemainingArguments = $true)] [string[]] $Arguments
    )
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with $LASTEXITCODE"
    }
}

if (-not $IsWindows) {
    throw 'Windows package refused: host is not Windows'
}
if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
    throw 'Windows package refused: cargo-packager 0.11.8 is required'
}
$PackagerVersion = (& cargo packager --version).Trim()
if ($LASTEXITCODE -ne 0 -or $PackagerVersion -ne 'cargo-packager 0.11.8') {
    throw "Windows package refused: expected cargo-packager 0.11.8, found $PackagerVersion"
}

$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$Target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Out = if ($env:ABV_PACKAGE_DIR) {
    $env:ABV_PACKAGE_DIR
}
elseif ($env:FOUNDRY_ARTIFACT_DIR) {
    $env:FOUNDRY_ARTIFACT_DIR
}
else {
    Join-Path $Root 'dist'
}
$Raw = Join-Path $Target "abv-windows-package-$PID"
$Triple = 'x86_64-pc-windows-msvc'
$Artifact = Join-Path $Out 'adequate-booru-viewer-windows-x86_64-setup.exe'

if (Test-Path -LiteralPath $Artifact) {
    throw "Windows package refused: $Artifact already exists"
}
New-Item -ItemType Directory -Force -Path $Out, $Raw | Out-Null
Push-Location $Root
try {
    Invoke-Exact cargo build --release --locked --package adequate_booru_viewer --bin abv `
        --features egui-test --target $Triple
    Invoke-Exact cargo packager --release --formats nsis --target $Triple `
        --packages adequate_booru_viewer --out-dir $Raw

    $Installers = @(Get-ChildItem -LiteralPath $Raw -Filter '*.exe' -File)
    if ($Installers.Count -ne 1) {
        throw "packager emitted $($Installers.Count) installers instead of one"
    }
    Move-Item -LiteralPath $Installers[0].FullName -Destination $Artifact

    $Install = Start-Process -FilePath $Artifact -ArgumentList '/S' -Wait -PassThru
    if ($Install.ExitCode -ne 0) {
        throw "installer exited with $($Install.ExitCode)"
    }
    $InstallRoot = Join-Path $env:LOCALAPPDATA 'Adequate Booru Viewer'
    $Binary = Join-Path $InstallRoot 'abv.exe'
    $License = Join-Path $InstallRoot 'LICENSE-MIT'
    $Uninstaller = Join-Path $InstallRoot 'uninstall.exe'
    if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
        throw "installer did not place $Binary"
    }
    if (-not (Test-Path -LiteralPath $Uninstaller -PathType Leaf)) {
        throw "installer did not place $Uninstaller"
    }
    if (-not (Test-Path -LiteralPath $License -PathType Leaf)) {
        throw "installer did not place $License"
    }
    Invoke-Exact $Binary --version

    $env:ABV_PORTABILITY_BINARY = $Binary
    if (-not $env:ABV_PORTABILITY_ARTIFACTS) {
        $env:ABV_PORTABILITY_ARTIFACTS = Join-Path $Target 'abv-packaged-portability'
    }
    Invoke-Exact cargo run --release --locked --package abv-portability

    $Data = Join-Path $env:APPDATA 'swarm\adequate_booru_viewer'
    $Sentinel = Join-Path $Data 'uninstall-preserves-user-data'
    New-Item -ItemType Directory -Force -Path $Data | Out-Null
    Set-Content -LiteralPath $Sentinel -Value 'user-owned' -NoNewline
    $Uninstall = Start-Process -FilePath $Uninstaller -ArgumentList '/S' -Wait -PassThru
    if ($Uninstall.ExitCode -ne 0) {
        throw "uninstaller exited with $($Uninstall.ExitCode)"
    }
    if (Test-Path -LiteralPath $Binary) {
        throw "uninstaller left $Binary"
    }
    if (Test-Path -LiteralPath $License) {
        throw "uninstaller left $License"
    }
    if (-not (Test-Path -LiteralPath $Sentinel -PathType Leaf)) {
        throw 'uninstaller destroyed user data'
    }

    $Hash = (Get-FileHash -LiteralPath $Artifact -Algorithm SHA256).Hash.ToLowerInvariant()
    $Checksum = "$Hash  $(Split-Path -Leaf $Artifact)`n"
    [System.IO.File]::WriteAllText("$Artifact.sha256", $Checksum, [System.Text.Encoding]::ASCII)
    Write-Host "proved $Artifact"
}
finally {
    Pop-Location
}
