<#
.SYNOPSIS
Build and inventory an unsigned x64 MSIX. Does not install, sign, or publish.
.DESCRIPTION
Copy packaging/microsoft-store/identity.example.json to a local file and fill
the exact Partner Center values plus the Windows version actually tested.
Set ProductIdentityConfirmed=true only after checking that product's identity.
Example: ./tools/package_store.ps1 -IdentityFile target/typing-store-identity.json
Development accepts an explicitly supplied development identity and uses a
separate artifact name/directory. Such artifacts are never Store candidates.
StageOnly leaves an unpacked layout suitable for separately authorized local
development registration; it does not register or launch the application.
#>
[CmdletBinding()]
param([string]$IdentityFile, [string]$InputSimManifest = '../inputsim/Cargo.toml',
    [string]$OutputRoot, [switch]$Development, [switch]$StageOnly)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'windows_package_common.ps1')

function Read-StoreIdentity {
    param([string]$Path, [bool]$IsDevelopment)
    if (-not $Path -or -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw 'Provide -IdentityFile with the confirmed Partner Center product values. See packaging/microsoft-store/identity.example.json.'
    }
    $identity = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    foreach ($property in @('IdentityName','Publisher','PublisherDisplayName','DisplayName','MaxVersionTested')) {
        if (-not ($identity.PSObject.Properties.Name -contains $property) -or
            $identity.$property -isnot [string] -or [string]::IsNullOrWhiteSpace($identity.$property) -or
            $identity.$property -match 'REQUIRES-|PLACEHOLDER|CHANGE.?ME|<|>') {
            throw "Missing or placeholder identity field: $property"
        }
    }
    if ($identity.IdentityName -notmatch '^[A-Za-z0-9.-]{3,50}$' -or $identity.IdentityName -eq 'Aldetl.NoteRecall') {
        throw 'IdentityName must be TypingVE''s own package identity (3-50 ASCII letters, digits, periods or hyphens).'
    }
    if ($identity.Publisher -notmatch '^CN=\S' -or $identity.Publisher.Length -gt 8192) { throw 'Publisher must be the exact CN=... subject from Product identity.' }
    $testedVersion = $null
    if ($identity.MaxVersionTested -notmatch '^\d+\.\d+\.\d+\.\d+$' -or
        -not [version]::TryParse($identity.MaxVersionTested, [ref]$testedVersion) -or
        $testedVersion -lt [version]'10.0.19041.0') { throw 'MaxVersionTested must be an explicitly tested four-part Windows version at or above 10.0.19041.0.' }
    if (-not $IsDevelopment -and ($identity.ProductIdentityConfirmed -isnot [bool] -or -not $identity.ProductIdentityConfirmed)) {
        throw 'Confirm the product identity in Partner Center and set ProductIdentityConfirmed=true. Use -Development only for a separate local test package.'
    }
    return $identity
}

function Get-SourceIdentity {
    param([string]$Root, [string[]]$Paths)
    $revision = & git -C $Root rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw "Cannot identify source repository: $Root" }
    $status = @(& git -C $Root status --porcelain -- @Paths)
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect source state: $Root" }
    $pathsToHash = @(& git -C $Root -c core.quotepath=false ls-files -c -o --exclude-standard -- @Paths | Sort-Object -Unique)
    if ($LASTEXITCODE -ne 0 -or -not $pathsToHash.Count) { throw "Cannot enumerate source inputs: $Root" }
    $files = @($pathsToHash | ForEach-Object {
        $path = Join-Path $Root $_
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            [ordered]@{ path=$_; sha256=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
        } else { [ordered]@{ path=$_; missing=$true } }
    })
    [ordered]@{ revision=$revision.Trim(); status=$status; files=$files }
}

function Assert-PackageLayout {
    param([string]$Root, $Identity, [string]$Version)
    [xml]$manifest = Get-Content -LiteralPath (Join-Path $Root 'AppxManifest.xml') -Raw
    $names = [Xml.XmlNamespaceManager]::new($manifest.NameTable)
    $names.AddNamespace('f', $manifest.DocumentElement.NamespaceURI)
    $names.AddNamespace('uap','http://schemas.microsoft.com/appx/manifest/uap/windows10')
    $names.AddNamespace('uap10','http://schemas.microsoft.com/appx/manifest/uap/windows10/10')
    $names.AddNamespace('desktop','http://schemas.microsoft.com/appx/manifest/desktop/windows10')
    $names.AddNamespace('rescap','http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities')
    $packageIdentity = $manifest.Package.Identity
    if ($packageIdentity.Name -ne $Identity.IdentityName -or $packageIdentity.Publisher -ne $Identity.Publisher -or
        $packageIdentity.Version -ne $Version -or $packageIdentity.ProcessorArchitecture -ne 'x64' -or
        $manifest.Package.Properties.DisplayName -ne $Identity.DisplayName -or
        $manifest.Package.Properties.PublisherDisplayName -ne $Identity.PublisherDisplayName) { throw 'Generated manifest identity/version mismatch.' }
    $languages = @($manifest.Package.Resources.Resource | ForEach-Object Language | Sort-Object)
    if (($languages -join ',') -ne 'en-US,zh-CN') { throw 'MSIX must declare English and Simplified Chinese.' }
    $targetFamily = $manifest.Package.Dependencies.TargetDeviceFamily
    if ($targetFamily.Name -ne 'Windows.Desktop' -or $targetFamily.MinVersion -ne '10.0.19041.0' -or
        $targetFamily.MaxVersionTested -ne $Identity.MaxVersionTested) { throw 'Incorrect Windows desktop compatibility declaration.' }
    $application = $manifest.SelectSingleNode('/f:Package/f:Applications/f:Application', $names)
    if ($application.Id -ne 'Typing' -or $application.Executable -ne 'TypingVE.exe' -or $application.EntryPoint -ne 'Windows.FullTrustApplication' -or
        $application.GetAttribute('RuntimeBehavior',$names.LookupNamespace('uap10')) -ne 'packagedClassicApp' -or
        $application.GetAttribute('TrustLevel',$names.LookupNamespace('uap10')) -ne 'mediumIL') { throw 'Incorrect full-trust application declaration.' }
    $extension = $application.SelectSingleNode('f:Extensions/desktop:Extension', $names)
    $task = $extension.SelectSingleNode('desktop:StartupTask', $names)
    if ($extension.Category -ne 'windows.startupTask' -or $extension.Executable -ne 'TypingVE.exe' -or $extension.EntryPoint -ne 'Windows.FullTrustApplication' -or
        $extension.GetAttribute('Parameters',$names.LookupNamespace('uap10')) -ne '--startup' -or
        $task.TaskId -ne 'TypingStartup' -or $task.Enabled -ne 'false') { throw 'Incorrect opt-in startup-task declaration.' }
    $capabilities = @($manifest.SelectNodes('/f:Package/f:Capabilities/*', $names) | ForEach-Object { $_.GetAttribute('Name') })
    if (($capabilities -join ',') -ne 'runFullTrust') { throw 'Unexpected package capabilities.' }
    foreach ($file in @('TypingVE.exe','inputsim.dll','Defaults/configuration.ini','Defaults/profiles/default.typing.ini','languages/en.ini','languages/en.md','languages/zh-CN.ini','languages/zh-CN.md','USER_GUIDE.md','README.txt','Assets/command-mode-movement.png','languages/assets/command-mode-movement.png','licenses/Typing/LICENSE','licenses/Typing/NOTICE')) {
        if (-not (Test-Path -LiteralPath (Join-Path $Root $file) -PathType Leaf)) { throw "Required package resource missing: $file" }
    }
    Add-Type -AssemblyName System.Drawing
    foreach ($asset in @(@('Square44x44Logo.png',44),@('Square150x150Logo.png',150),@('StoreLogo.png',50))) {
        $path = Join-Path $Root "Assets/$($asset[0])"
        $bytes = [IO.File]::ReadAllBytes($path)
        if ($bytes.Length -lt 24 -or [BitConverter]::ToString($bytes,0,8) -ne '89-50-4E-47-0D-0A-1A-0A') { throw "Not a PNG: $path" }
        $picture = [Drawing.Image]::FromFile($path)
        try { if ($picture.Width -ne $asset[1] -or $picture.Height -ne $asset[1]) { throw "Incorrect image dimensions: $path" } }
        finally { $picture.Dispose() }
    }
    if (Test-Path -LiteralPath (Join-Path $Root 'AppxSignature.p7x')) { throw 'The Store upload artifact must have no development signature.' }
}

# Preflight validates external inputs before building or creating output directories.
$identity = Read-StoreIdentity -Path $IdentityFile -IsDevelopment ([bool]$Development)
Push-Location -LiteralPath $projectRoot
try {
    $metadata = & cargo metadata --no-deps --format-version 1 --locked --offline | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read application version.' }
    $version = ($metadata.packages | Where-Object name -eq 'typing-windows').version
    if ($version -notmatch '^(\d+)\.(\d+)\.(\d+)$' -or [int]$Matches[1] -lt 1 -or
        [long]$Matches[1] -gt 65535 -or [long]$Matches[2] -gt 65535 -or [long]$Matches[3] -gt 65535) { throw 'Cargo version cannot map to a Store package version.' }
    $packageVersion = "$version.0"
    if (-not $OutputRoot) { $OutputRoot = if ($Development) { 'target/dist/store-development' } else { 'target/dist/store' } }
    $output = [IO.Path]::GetFullPath((Join-Path $projectRoot $OutputRoot))
    if (-not $output.StartsWith($projectRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Store outputs must stay within the TypingVE workspace.' }
    $channel = if ($Development) { 'development' } else { 'store' }
    $name = "TypingVE-$version-$channel-x64"
    $archive = Join-Path $output "$name.msix"
    $reportPath = Join-Path $output "$name.manifest.json"
    if ((Test-Path -LiteralPath $archive) -or (Test-Path -LiteralPath $reportPath)) { throw 'This candidate already exists. Preserve it and use another output directory or a newer release version.' }
    $inputManifest = (Resolve-Path -LiteralPath $InputSimManifest).Path
    $inputRoot = Split-Path $inputManifest -Parent
    $typingPaths = @('Cargo.toml','Cargo.lock','typing-core','typing-host','typing-windows','resources','tools','licenses','packaging/microsoft-store','LICENSE','NOTICE','ARTWORK-LICENSE.md','THIRD_PARTY_NOTICES.md','docs/USER_GUIDE.md','docs/assets/command-mode-movement.png')
    $inputPaths = @('Cargo.toml','Cargo.lock','src','include','build.rs','LICENSE')
    $sources = [ordered]@{ typing=(Get-SourceIdentity $projectRoot $typingPaths); inputsim=(Get-SourceIdentity $inputRoot $inputPaths) }
    & cargo test -p typing-host languages::tests::shipped_profiles_cover_actual_message_ids_and_keep_english_sources_aligned --locked --offline | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Language resource verification failed.' }
    & cargo test -p typing-windows distribution::tests --locked --offline | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Distribution paths and first-use seed verification failed.' }
    $runtime = Build-TypingWindowsRuntime -ProjectRoot $projectRoot -InputSimManifest $inputManifest -Architecture x64 -OutputRoot (Join-Path $projectRoot 'target/store-build/x64')
    $makeappx = Join-Path $runtime.SdkBin 'x64/makeappx.exe'
    if (-not (Test-Path -LiteralPath $makeappx)) { throw 'Windows SDK MakeAppx.exe is required.' }
    $inputMetadata = & cargo metadata --manifest-path $inputManifest --no-deps --format-version 1 --locked --offline | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read InputSim package version.' }
    $inputVersion = ($inputMetadata.packages | Where-Object name -eq 'inputsim').version
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    $stage = Join-Path $output ("stage-" + [guid]::NewGuid().ToString('N'))
    foreach ($directory in @('Defaults/profiles','Assets','languages/assets','licenses')) { New-Item -ItemType Directory -Path (Join-Path $stage $directory) -Force | Out-Null }
    Copy-Item -LiteralPath $runtime.Application -Destination (Join-Path $stage 'TypingVE.exe')
    Copy-Item -LiteralPath $runtime.InputSim -Destination (Join-Path $stage 'inputsim.dll')
    Copy-Item -LiteralPath resources/configuration.ini -Destination (Join-Path $stage 'Defaults/configuration.ini')
    Copy-Item -LiteralPath resources/profiles/default.typing.ini -Destination (Join-Path $stage 'Defaults/profiles/default.typing.ini')
    foreach ($file in @('en.ini','zh-CN.ini','zh-CN.md')) { Copy-Item -LiteralPath "resources/languages/$file" -Destination (Join-Path $stage "languages/$file") }
    foreach ($file in @('USER_GUIDE.md','languages/en.md')) { Copy-Item -LiteralPath docs/USER_GUIDE.md -Destination (Join-Path $stage $file) }
    foreach ($directory in @('Assets','languages/assets')) { Copy-Item -LiteralPath docs/assets/command-mode-movement.png -Destination (Join-Path $stage "$directory/command-mode-movement.png") }
    Copy-Item -LiteralPath packaging/microsoft-store/Assets/StoreLogo.png,packaging/microsoft-store/Assets/Square44x44Logo.png,packaging/microsoft-store/Assets/Square150x150Logo.png -Destination (Join-Path $stage 'Assets')
    Copy-TypingWindowsLicenses -ProjectRoot $projectRoot -Destination (Join-Path $stage 'licenses') -InputSimManifest $inputManifest -InputSimVersion $inputVersion -Target $runtime.Target
    [xml]$manifest = Get-Content -LiteralPath packaging/microsoft-store/AppxManifest.xml -Raw
    $manifest.Package.Identity.SetAttribute('Name',$identity.IdentityName)
    $manifest.Package.Identity.SetAttribute('Publisher',$identity.Publisher)
    $manifest.Package.Identity.SetAttribute('Version',$packageVersion)
    $manifest.Package.Properties.DisplayName = $identity.DisplayName
    $manifest.Package.Properties.PublisherDisplayName = $identity.PublisherDisplayName
    $manifest.Package.Dependencies.TargetDeviceFamily.SetAttribute('MaxVersionTested',$identity.MaxVersionTested)
    $manifest.Package.Applications.Application.VisualElements.SetAttribute('DisplayName',$identity.DisplayName)
    $manifest.Save((Join-Path $stage 'AppxManifest.xml'))
    @"
TypingVE $version - Windows x64 MSIX ($channel)

Install through the Microsoft Store for a Store release. This unsigned package
is not a sideload installer. Development layouts require separate registration.
Explicit launch opens the existing Mapping Profile window. Closing/minimizing
hides it; use the tray's Exit action to quit. Only one installed instance runs
per Windows user/package family. Quit the portable copy before switching.

Configuration, editable profiles, language/window preferences and bounded local
logs use the Windows package LocalCache folder's typing directory, normally:
%LOCALAPPDATA%\Packages\<package-family>\LocalCache\typing\
First launch copies only packaged defaults; updates preserve existing edits.
LocalCache is excluded from Windows app-data backup/restore. Copy configuration
and profiles outside package app data before uninstall/reset if needed.
Use Import for portable mappings; no portable preferences or startup entries
are automatically imported or changed. Optional file logging omits raw input
and typed text; its failures do not stop the app. Event diagnostics is in memory.

Start with Windows is opt-in in Settings and uses Windows' startup task. Windows
Settings > Apps > Startup can disable it; TypingVE respects user/policy choices.
The installed edition accepts ordinary launch or --startup, without portable
path overrides. Read-only package files must not be edited.

Windows Desktop x64 compatibility target: Windows 10 build 19041 or newer.
Manifest MaxVersionTested: $($identity.MaxVersionTested). A declared target is
not clean-machine or certification evidence; consult the candidate inventory.
Required software/artwork/component notices are in licenses/.
"@ | Set-Content -LiteralPath (Join-Path $stage 'README.txt') -Encoding utf8
    Assert-PackageLayout -Root $stage -Identity $identity -Version $packageVersion
    $after = [ordered]@{ typing=(Get-SourceIdentity $projectRoot $typingPaths); inputsim=(Get-SourceIdentity $inputRoot $inputPaths) }
    if (($sources | ConvertTo-Json -Depth 10 -Compress) -ne ($after | ConvertTo-Json -Depth 10 -Compress)) { throw 'Source inputs changed while producing the candidate.' }
    $inventory = @(Get-ChildItem -LiteralPath $stage -Recurse -File | Sort-Object FullName | ForEach-Object { [ordered]@{path=$_.FullName.Substring($stage.Length+1).Replace('\','/'); bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash} })
    if (-not $StageOnly) {
        & $makeappx pack /d $stage /p $archive | Out-Host
        if ($LASTEXITCODE -ne 0) { throw 'MakeAppx validation/packaging failed. No candidate report was written.' }
        $extracted = Join-Path $output ("unpacked-" + [guid]::NewGuid().ToString('N'))
        & $makeappx unpack /p $archive /d $extracted | Out-Host
        if ($LASTEXITCODE -ne 0) { throw 'MSIX extraction failed.' }
        Assert-PackageLayout -Root $extracted -Identity $identity -Version $packageVersion
        foreach ($entry in $inventory) { if ((Get-FileHash -LiteralPath (Join-Path $extracted $entry.path) -Algorithm SHA256).Hash -ne $entry.sha256) { throw "MSIX round-trip mismatch: $($entry.path)" } }
    }
    $report = [ordered]@{
        channel=$channel; applicationVersion=$version; packageVersion=$packageVersion; target=$runtime.Target
        identity=$identity; builtUtc=[DateTime]::UtcNow.ToString('o'); sources=$sources
        rust=$runtime.Rust; msvcToolset=$runtime.MsvcToolset; sdkBin=$runtime.SdkBin; crt='static'; imports=$runtime.Imports
        layout=$stage; stageOnly=[bool]$StageOnly; files=$inventory
        unpackedBytes=($inventory | ForEach-Object { $_.bytes } | Measure-Object -Sum).Sum
        archive=$(if (-not $StageOnly) {$archive} else {$null})
        archiveBytes=$(if (-not $StageOnly) {(Get-Item -LiteralPath $archive).Length} else {$null})
        archiveSha256=$(if (-not $StageOnly) {(Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash} else {$null})
        verification=[ordered]@{languageResources='passed'; distributionAndSeeds='passed'; binaryInspection='passed'; layout='passed'; archiveRoundTrip=$(if ($StageOnly) {'not run'} else {'passed'}); installedExecution='not run'; cleanMachines='not run'; accessibility='not run'; certificationKit='not run'; storeSubmission='not performed'}
    }
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $reportPath -Encoding utf8
    [pscustomobject]@{Report=$reportPath; Layout=$stage; Archive=$report.archive; Sha256=$report.archiveSha256; Channel=$channel}
} finally { Pop-Location }
