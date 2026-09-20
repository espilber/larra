# NSIS installer. Authenticode-signs when Azure Artifact Signing env is
# present and SignTool succeeds; otherwise ships unsigned.
# Must run on Windows. From macOS: gh workflow run release-windows.yml
#
#   pwsh scripts/release-windows.ps1
#   pwsh scripts/release-windows.ps1 -Target aarch64-pc-windows-msvc

param(
    [string]$Target = "",
    # ARM runners cannot run Azure SignTool (x64). Build unsigned there,
    # then sign on windows-latest with scripts/sign-windows-nsis.ps1.
    [switch]$SkipAuthenticode
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [System.Runtime.InteropServices.OSPlatform]::Windows)) {
    Write-Error "Windows NSIS needs Windows. From macOS: gh workflow run release-windows.yml"
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

function Test-AzureSigningEnv {
    foreach ($name in $script:azureNames) {
        $val = [Environment]::GetEnvironmentVariable($name)
        if ([string]::IsNullOrWhiteSpace($val)) {
            return $false
        }
    }
    return $true
}

if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY) -and
    [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
    Write-Error "Set TAURI_SIGNING_PRIVATE_KEY in .env.signing or the GitHub secret (in-app updater)."
}

if ($Target) {
    rustup target add $Target
    Write-Host "Staging engine pin for $Target…"
    node scripts/fetch-engine.mjs --triple=$Target
    $prefix = Join-Path $Root "src-tauri/target/$Target/release/bundle/nsis"
} else {
    Write-Host "Staging engine pin for this Windows host…"
    node scripts/fetch-engine.mjs
    $prefix = Join-Path $Root "src-tauri/target/release/bundle/nsis"
}

function Invoke-NsisBuild {
    param([bool]$Sign)

    $config = @{
        bundle = @{
            createUpdaterArtifacts = $true
        }
    }
    if ($Sign) {
        $signCommand = @(
            "artifact-signing-cli",
            "-e", $env:AZURE_ARTIFACT_SIGNING_ENDPOINT,
            "-a", $env:AZURE_ARTIFACT_SIGNING_ACCOUNT,
            "-c", $env:AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE,
            "-d", "Larra",
            "%1"
        ) -join " "
        $config.bundle.windows = @{
            signCommand = $signCommand
        }
    }
    $configPath = Join-Path $env:TEMP "larra-tauri-windows-sign.json"
    $json = $config | ConvertTo-Json -Depth 6 -Compress
    [System.IO.File]::WriteAllText($configPath, $json)

    if ($Target) {
        pnpm tauri build --target $Target --bundles nsis --config $configPath
    } else {
        pnpm tauri build --bundles nsis --config $configPath
    }
    if ($LASTEXITCODE -ne 0) {
        throw "tauri build exited $LASTEXITCODE"
    }
}

function Test-ExesAuthenticodeValid {
    param([string]$Dir)
    $ok = $true
    Get-ChildItem $Dir -Filter *.exe -ErrorAction Stop | ForEach-Object {
        $sig = Get-AuthenticodeSignature $_.FullName
        Write-Host "$($_.Name): $($sig.Status) $($sig.SignerCertificate.Subject)"
        if ($sig.Status -ne "Valid") {
            $ok = $false
        }
    }
    return $ok
}

$wantSign = -not $SkipAuthenticode -and (Test-AzureSigningEnv)
if ($SkipAuthenticode) {
    Write-Host "Skipping Authenticode on this host."
}
if ($wantSign -and -not (Get-Command artifact-signing-cli -ErrorAction SilentlyContinue)) {
    Write-Warning "artifact-signing-cli is not installed; building unsigned NSIS."
    $wantSign = $false
}

$signed = $false
if ($wantSign) {
    Write-Host "Azure signing env is set; building signed NSIS…"
    try {
        Invoke-NsisBuild -Sign $true
        if (Test-ExesAuthenticodeValid -Dir $prefix) {
            $signed = $true
        } else {
            throw "Authenticode status is not Valid"
        }
    } catch {
        Write-Warning "Signing did not work ($($_.Exception.Message)). Building unsigned NSIS."
    }
}

if (-not $signed) {
    if (-not $wantSign) {
        Write-Host "Azure signing env is missing; building unsigned NSIS…"
    }
    Invoke-NsisBuild -Sign $false
    Get-ChildItem $prefix -Filter *.exe -ErrorAction Stop | ForEach-Object {
        $sig = Get-AuthenticodeSignature $_.FullName
        Write-Host "$($_.Name): $($sig.Status) $($sig.SignerCertificate.Subject)"
    }
}

Write-Host ""
Write-Host "Artifacts in $prefix"
Get-ChildItem $prefix -ErrorAction Stop | Format-Table Name, Length

$hostTriple = ((rustc -vV) | Where-Object { $_ -match '^host: ' }) -replace '^host:\s+', ''
$triple = if ($Target) { $Target } else { $hostTriple.Trim() }
Write-Host ""
node scripts/latest-json.mjs --bundle-dir $prefix --triple $triple
Write-Host "Attach the NSIS exe, .sig, and dist/latest.json to the GitHub Release."
Write-Host "Merge fragments from every platform: node scripts/latest-json.mjs --combine"
