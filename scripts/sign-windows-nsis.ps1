# Authenticode-sign an existing Windows NSIS installer, then refresh the
# in-app updater signature. Must run on x64 Windows (Azure SignTool is x64).
#
#   pwsh scripts/sign-windows-nsis.ps1 -SearchDir path\to\artifact
#   pwsh scripts/sign-windows-nsis.ps1 -SearchDir path\to\artifact -Triple aarch64-pc-windows-msvc

param(
    [Parameter(Mandatory = $true)]
    [string]$SearchDir,
    [string]$Triple = "aarch64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [System.Runtime.InteropServices.OSPlatform]::Windows)) {
    Write-Error "Authenticode signing needs Windows."
}

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($arch -eq [System.Runtime.InteropServices.Architecture]::Arm64) {
    Write-Error "Azure SignTool does not run on Windows ARM. Sign on x64."
}

$envFile = Join-Path $Root ".env.signing"
if (Test-Path $envFile) {
    Get-Content $envFile | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#")) { return }
        if ($line -match '^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$') {
            $name = $Matches[1]
            $value = $Matches[2].Trim()
            if ($value.Length -ge 2) {
                $q = $value[0]
                if (($q -eq '"' -or $q -eq "'") -and $value[-1] -eq $q) {
                    $value = $value.Substring(1, $value.Length - 2)
                }
            }
            Set-Item -Path "Env:$name" -Value $value
        }
    }
}

$azureNames = @(
    "AZURE_CLIENT_ID",
    "AZURE_CLIENT_SECRET",
    "AZURE_TENANT_ID",
    "AZURE_ARTIFACT_SIGNING_ENDPOINT",
    "AZURE_ARTIFACT_SIGNING_ACCOUNT",
    "AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE"
)
foreach ($name in $azureNames) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        Write-Error "Missing $name"
    }
}

if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY) -and
    [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
    Write-Error "Set TAURI_SIGNING_PRIVATE_KEY (in-app updater)."
}

if (-not (Get-Command artifact-signing-cli -ErrorAction SilentlyContinue)) {
    Write-Error "artifact-signing-cli is not installed."
}

$resolved = Resolve-Path -LiteralPath $SearchDir
$exe = Get-ChildItem -LiteralPath $resolved -Recurse -File -ErrorAction Stop |
    Where-Object { $_.Name -match '_(arm64|aarch64)-setup\.exe$' } |
    Select-Object -First 1
if (-not $exe) {
    $exe = Get-ChildItem -LiteralPath $resolved -Recurse -Filter "*.exe" -File |
        Where-Object { $_.Name -notlike "*.sig" } |
        Select-Object -First 1
}
if (-not $exe) {
    Write-Error "No NSIS installer under $resolved"
}

Write-Host "Signing $($exe.FullName)"
& artifact-signing-cli `
    -e $env:AZURE_ARTIFACT_SIGNING_ENDPOINT `
    -a $env:AZURE_ARTIFACT_SIGNING_ACCOUNT `
    -c $env:AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE `
    -d "Larra" `
    $exe.FullName
if ($LASTEXITCODE -ne 0) {
    Write-Error "artifact-signing-cli exited $LASTEXITCODE"
}

$sig = Get-AuthenticodeSignature $exe.FullName
Write-Host "$($exe.Name): $($sig.Status) $($sig.SignerCertificate.Subject)"
if ($sig.Status -ne "Valid") {
    Write-Error "Authenticode status is $($sig.Status), not Valid"
}

Write-Host "Refreshing updater signature…"
pnpm exec tauri signer sign $exe.FullName
if ($LASTEXITCODE -ne 0) {
    Write-Error "tauri signer sign exited $LASTEXITCODE"
}

$canonical = Join-Path $Root "src-tauri/target/$Triple/release/bundle/nsis"
New-Item -ItemType Directory -Force -Path $canonical | Out-Null
$destExe = Join-Path $canonical $exe.Name
if ($exe.FullName -ne $destExe) {
    Copy-Item -Force $exe.FullName $destExe
    Copy-Item -Force "$($exe.FullName).sig" "$destExe.sig"
}

node scripts/latest-json.mjs --bundle-dir $canonical --triple $Triple
Write-Host "Signed $($exe.Name)"
