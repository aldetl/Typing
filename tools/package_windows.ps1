param([string]$InputSimManifest = '../inputsim/Cargo.toml',
    [ValidateSet('x64','x86')][string]$Architecture = 'x64')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'windows_package_common.ps1')
Push-Location -LiteralPath $projectRoot
try {
    & cargo test -p typing-host languages::tests::shipped_profiles_cover_actual_message_ids_and_keep_english_sources_aligned --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Language resource verification failed' }
    $runtime = Build-TypingWindowsRuntime -ProjectRoot $projectRoot -InputSimManifest $InputSimManifest -Architecture $Architecture -OutputRoot (Join-Path $projectRoot "target/portable-build/$Architecture")
    $legacy = $runtime.Legacy
    $triple = $runtime.Target
    $manifest = $runtime.InputSimManifest
    $application = $runtime.Application
    $inputSim = $runtime.InputSim
    $dependencies = $runtime.Imports
    $metadata = cargo metadata --no-deps --format-version 1 --locked --offline | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read package metadata' }
    $version = ($metadata.packages | Where-Object name -eq 'typing-windows').version
    $inputSimMetadata = cargo metadata --manifest-path $manifest --no-deps --format-version 1 --locked --offline | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read InputSim package metadata' }
    $inputSimVersion = ($inputSimMetadata.packages | Where-Object name -eq 'inputsim').version
    if (-not $inputSimVersion) { throw 'InputSim package version is missing' }
    $dist = Join-Path $projectRoot 'target/dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $stage = Join-Path $dist ("stage-" + [guid]::NewGuid().ToString('N'))
    $packageName = "TypingVE-$version-win-$Architecture"
    $package = Join-Path $stage $packageName
    $licenses = Join-Path $package 'licenses'
    New-Item -ItemType Directory -Path $licenses -Force | Out-Null
    Copy-Item -LiteralPath $application -Destination (Join-Path $package 'TypingVE.exe')
    Copy-Item -LiteralPath $inputSim -Destination (Join-Path $package 'inputsim.dll')
    Copy-Item -LiteralPath resources/configuration.ini -Destination (Join-Path $package 'configuration.ini')
    New-Item -ItemType Directory -Path (Join-Path $package 'profiles') | Out-Null
    Copy-Item -LiteralPath resources/profiles/default.typing.ini -Destination (Join-Path $package 'profiles/default.typing.ini')
    Copy-Item -LiteralPath docs/USER_GUIDE.md -Destination (Join-Path $package 'USER_GUIDE.md')
    New-Item -ItemType Directory -Path (Join-Path $package 'languages') | Out-Null
    foreach ($languageFile in @('en.ini', 'zh-CN.ini', 'zh-CN.md')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "resources/languages/$languageFile") -Destination (Join-Path $package "languages/$languageFile")
    }
    Copy-Item -LiteralPath docs/USER_GUIDE.md -Destination (Join-Path $package 'languages/en.md')
    foreach ($guideAssetDirectory in @('assets', 'languages/assets')) {
        $guideAssets = Join-Path $package $guideAssetDirectory
        New-Item -ItemType Directory -Path $guideAssets | Out-Null
        Copy-Item -LiteralPath docs/assets/command-mode-movement.png -Destination (Join-Path $guideAssets 'command-mode-movement.png')
    }
    @"
TypingVE $version - portable Windows $Architecture build

Extract this entire folder, then run TypingVE.exe. Keep inputsim.dll,
configuration.ini, and the profiles and languages folders together. No separate runtime or framework installation is
required by this build. Both binaries use the statically linked C runtime.
Targets: $(if ($legacy) { 'Windows 7 SP1/10 x86 and Windows 11 via WOW64' } else { 'Windows 11 x64' }).
Windows 11 x64 is the available verification platform. Clean-install deployment,
Windows 7/10 execution, actual sleep/session changes and physical mixed-DPI
monitors remain pending; no clean-machine or legacy-OS verification is claimed.
The x86 build uses Rust's Windows 7 target with a rebuilt standard library.
Before Windows 10 1607, text mapping is stateless and has weaker validation for
unusual custom layouts and shifted dead keys. Newer Windows retains stronger
state-preserving character verification.

TypingVE starts enabled in the notification area. Right-click its tray icon for
Show Window, one Enable/Disable toggle, and Exit, in that order.
Double-click toggles enabled state.
Closing or minimizing the main window hides it; use tray Exit to quit.
Mapping Profile, Event diagnostics, Settings, and Help use native controls, in that tab order.
In Settings, check Start with Windows to launch TypingVE in the tray at sign-in for
the current user. Uncheck it to cancel automatic startup. Changes apply immediately.
After moving the portable folder, enable this option again from the new location.
Choose System default, English, or 简体中文 in Settings to change language immediately.
The language choice is per user in %LOCALAPPDATA%\Typing\language.txt. Language
profiles and translated guides are in languages, separately from mapping profiles.
After editing a language file, switch away and back or restart to reload it.
See USER_GUIDE.md for setup, everyday use, profiles, and troubleshooting.

Default configuration: Ctrl+U toggles TypingVE, Caps Lock or Left Alt alone toggles
command mode. In command mode I/J/K/L move up/left/down/right; see the
Mapping Profile page for other commands. Quit an older Aldetl/TypingVE instance using the same
global hotkey before running this copy. Edit and save independent mapping
profiles in Mapping Profile, or import legacy INI mappings. The Profile dropdown opens a saved profile for editing. Save only writes it; Set as active saves pending edits and explicitly applies the displayed profile.
Only names from this app's profiles directory are listed, without recursion.
New/Copy/Import drafts save there. Save As can save anywhere; external files stay
out of the list. Rename changes a display name; Delete confirms an inactive
profile chosen from its own list and protects the active profile. Unsaved edits
prompt Save / Don't Save / Cancel before closing or replacing the draft.
Application preferences in
configuration.ini require a restart; the selected profile is resolved relative
to that configuration, independently of the working directory. Missing or
invalid application configuration reports its path and exits without changing
the file or starting capture. An invalid selected profile opens recovery with
capture stopped. mode_mouse_tip is accepted as an alias for mode_cursor_tip;
mode_cursor_tip wins when both are present. Both values must be on or off.

Preferences and the diagnostic log are per user under
%LOCALAPPDATA%\Typing (window-size.txt and typing.log). User-profile and temporary
directory fallbacks are used if LOCALAPPDATA is unavailable. The program does
not store these files in this extracted folder. Diagnostic file logging runs
automatically in the background, appending across launches with 1 MiB files
and up to three archives. Logging failures are silently ignored; messages may
be dropped while busy or at exit. Event diagnostics shows keyboard events in a
table that fills the available space.

This build contains Rust shared host logic, native Win32 controls, and InputSim.
There is no Slint, webview, managed runtime, installer, or updater. The app and
enabled tray use the square-eye pixel mole; disabled uses the narrow-eye mole.
Command mode uses the same enabled mole lowered slightly, preserving its stepped
bottom edge and thin slate-gray outline. Default mappings retain Aldetl's resources.
Help shows a short introduction and examples, followed by Open User Guide… and
About. The full formatted guide opens in a separate resizable, read-only dialog.
TypingVE's software license, icon reuse terms, and project NOTICE are in
licenses/Typing. The third-party overview is licenses/Typing/THIRD_PARTY_NOTICES.md;
component license files and Rust notices are under licenses.
"@ | Set-Content -LiteralPath (Join-Path $package 'README.txt') -Encoding utf8

    Copy-TypingWindowsLicenses -ProjectRoot $projectRoot -Destination $licenses -InputSimManifest $manifest -InputSimVersion $inputSimVersion -Target $triple -Legacy:$legacy
    $inventory = @(Get-ChildItem -LiteralPath $package -Recurse -File | Sort-Object FullName | ForEach-Object {
        [ordered]@{ path = $_.FullName.Substring($package.Length + 1).Replace('\', '/');
            bytes = $_.Length; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    })
    $zip = Join-Path $dist "$packageName.zip"
    Compress-Archive -LiteralPath $package -DestinationPath $zip -CompressionLevel Optimal -Force
    $extracted = Join-Path $stage 'extracted'
    Expand-Archive -LiteralPath $zip -DestinationPath $extracted
    $extractedPackage = Join-Path $extracted $packageName
    foreach ($entry in $inventory) {
        $file = Join-Path $extractedPackage $entry.path
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $entry.sha256) {
            throw "Archive round-trip mismatch: $($entry.path)"
        }
    }
    $report = [ordered]@{
        version = $version; target = $triple; builtUtc = [DateTime]::UtcNow.ToString('o')
        rust = $(if ($legacy) { rustup run nightly-2026-04-15 rustc --version } else { rustc --version }); crt = 'static'; imports = $dependencies
        archive = $zip; archiveBytes = (Get-Item -LiteralPath $zip).Length
        archiveSha256 = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash
        unpackedBytes = ($inventory | ForEach-Object { $_.bytes } | Measure-Object -Sum).Sum
        extractedPackage = $extractedPackage; files = $inventory
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $dist "$packageName.manifest.json") -Encoding utf8
    Write-Output ($report | ConvertTo-Json -Depth 3)
} finally {
    Pop-Location
}
