<#
.SYNOPSIS
Verify an inventoried MSIX; optionally exercise its isolated development layout.
.DESCRIPTION
The default is read-only artifact verification. -DevelopmentRuntime requires
a development report whose identity ends in .Development and Developer Mode
already enabled. It registers that layout only if the identity is not installed,
runs native activation/startup/UI checks, and unregisters only its own package
in finally. It never enables Developer Mode or touches portable startup entries.
Run on an interactive desktop with other TypingVE copies closed.
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Report, [switch]$DevelopmentRuntime, [switch]$Inspect)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$candidate = Get-Content -LiteralPath $Report -Raw | ConvertFrom-Json
if ($candidate.archive -and (Get-FileHash -LiteralPath $candidate.archive -Algorithm SHA256).Hash -ne $candidate.archiveSha256) {
    throw 'MSIX hash differs from the candidate report.'
}
foreach ($entry in $candidate.files) {
    if ((Get-FileHash -LiteralPath (Join-Path $candidate.layout $entry.path) -Algorithm SHA256).Hash -ne $entry.sha256) {
        throw "Candidate layout changed: $($entry.path)"
    }
}
Write-Host 'PASS candidate archive and layout hashes'
if (-not $DevelopmentRuntime) { return }
if ($candidate.channel -ne 'development' -or $candidate.identity.IdentityName -notmatch '\.Development$' -or
    $candidate.identity.ProductIdentityConfirmed) { throw 'Runtime tests require a separate, unconfirmed .Development identity.' }
if (Get-AppxPackage -Name $candidate.identity.IdentityName) { throw 'This identity is already installed. Use a fresh development identity; existing data will not be reset.' }
if (Get-ChildItem -LiteralPath (Join-Path $env:LOCALAPPDATA 'Packages') -Directory -Filter ($candidate.identity.IdentityName + '_*') -ErrorAction SilentlyContinue) {
    throw 'Data for this development identity already exists. Use a fresh identity to preserve it.'
}
if (@(Get-Process -Name Typing,TypingVE -ErrorAction SilentlyContinue).Count) { throw 'Close other TypingVE or legacy Typing copies before the interactive runtime checks.' }
$developer = Get-ItemProperty -LiteralPath HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock -ErrorAction SilentlyContinue
if ($developer.AllowDevelopmentWithoutDevLicense -ne 1) { throw 'Developer Mode must already be enabled for development registration.' }
& (Join-Path $PSScriptRoot 'build_windows_tests.ps1') -Architecture x64
if ($LASTEXITCODE -ne 0) { throw 'Native test compilation failed.' }
$installed = $null
$resultPath = Join-Path (Split-Path -Parent (Resolve-Path -LiteralPath $Report).Path) ("runtime-" + [guid]::NewGuid().ToString('N') + '.json')
$result = [ordered]@{candidateSha256=$candidate.archiveSha256; startedUtc=[DateTime]::UtcNow.ToString('o'); osVersion=[Environment]::OSVersion.Version.ToString(); arguments='not run'; activation='not run'; nativeUi='not run'; data='not run'; cleanup='not run'; cleanMachines='not run'; certificationKit='not run'}
try {
    Add-AppxPackage -Register (Join-Path $candidate.layout 'AppxManifest.xml') -DisableDevelopmentMode:$false
    $installed = Get-AppxPackage -Name $candidate.identity.IdentityName
    if (-not $installed) { throw 'Development registration did not return the expected package.' }
    $aumid = $installed.PackageFamilyName + '!Typing'
    $data = Join-Path $env:LOCALAPPDATA "Packages/$($installed.PackageFamilyName)/LocalCache/typing"
    if (Test-Path -LiteralPath $data) { throw 'Development data already exists. Use a fresh identity; existing data will not be modified.' }
    & (Join-Path $projectRoot 'target/tests-x64/store_activation.exe') --expect-error $aumid '--config forbidden.ini'
    if ($LASTEXITCODE -ne 0 -or (Test-Path -LiteralPath $data)) { throw 'Rejected packaged arguments must show an error and leave managed data untouched.' }
    $result.arguments='passed: error dialog, exit 1, no managed data created'
    # The first launch must publish the managed directory itself. Initial
    # activation assertions require an English Windows display language.
    & (Join-Path $projectRoot 'target/tests-x64/store_activation.exe') $aumid
    if ($LASTEXITCODE -ne 0) { throw 'Packaged activation or startup-task checks failed.' }
    $result.activation='passed'
    foreach ($file in @('configuration.ini','profiles/default.typing.ini')) {
        if ((Get-FileHash -LiteralPath (Join-Path $data $file)).Hash -ne
            (Get-FileHash -LiteralPath (Join-Path $candidate.layout "Defaults/$file")).Hash) { throw "First-use default differs: $file" }
    }
    $configuration = Join-Path $data 'configuration.ini'
    [IO.File]::AppendAllText($configuration,"`r`n; Store persistence validation`r`n")
    $savedHash = (Get-FileHash -LiteralPath $configuration).Hash
    [IO.File]::WriteAllText((Join-Path $data 'language.txt'),"en`n")
    $launched = & (Join-Path $projectRoot 'target/tests-x64/store_activation.exe') --activate $aumid
    if ($LASTEXITCODE -ne 0) { throw 'Explicit packaged relaunch failed.' }
    $uiMode = if ($Inspect) { 'inspect' } else { 'test' }
    & (Join-Path $projectRoot 'target/tests-x64/native_ui.exe') --attach-store $launched $uiMode
    if ($LASTEXITCODE -ne 0) { throw 'Packaged native UI verification failed.' }
    $result.nativeUi='passed'
    if ((Get-FileHash -LiteralPath $configuration).Hash -ne $savedHash) { throw 'Relaunch overwrote edited configuration.' }
    foreach ($entry in $candidate.files) {
        if ((Get-FileHash -LiteralPath (Join-Path $installed.InstallLocation $entry.path)).Hash -ne $entry.sha256) { throw "Runtime changed package resource: $($entry.path)" }
    }
    $savedConfiguration = [IO.File]::ReadAllText($configuration)
    [IO.File]::WriteAllText($configuration,"[common]`r`nhotkey=invalid-key`r`n")
    $invalidHash = (Get-FileHash -LiteralPath $configuration).Hash
    & (Join-Path $projectRoot 'target/tests-x64/store_activation.exe') --expect-error $aumid ''
    if ($LASTEXITCODE -ne 0 -or (Get-FileHash -LiteralPath $configuration).Hash -ne $invalidHash) { throw 'Invalid existing configuration must fail without being repaired or overwritten.' }
    [IO.File]::WriteAllText($configuration,$savedConfiguration)
    $result.data='passed: first-use defaults, edit persistence, damaged configuration preserved, package resources unchanged'
    $result.packageFamily=$installed.PackageFamilyName
    $result.dataPath=$data
} finally {
    if ($installed) {
        if ($data -and (Test-Path -LiteralPath (Join-Path $data 'typing.log'))) {
            Copy-Item -LiteralPath (Join-Path $data 'typing.log') -Destination ($resultPath + '.log')
        }
        # Target only this invocation's uniquely checked development package.
        $processes = @(Get-Process TypingVE -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $candidate.layout 'TypingVE.exe') })
        foreach ($ownedProcess in $processes) {
            & (Join-Path $projectRoot 'target/tests-x64/native_ui.exe') --quit $ownedProcess.Id
        }
        Remove-AppxPackage -Package $installed.PackageFullName
        $result.cleanup='unregistered isolated development package'
    }
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $resultPath -Encoding utf8
    Write-Host "Runtime evidence: $resultPath"
}
