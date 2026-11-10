param([string]$InputSimManifest = '../inputsim/Cargo.toml',
    [ValidateSet('x64','x86')][string]$Architecture = 'x64')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
$savedEnvironment = @{}
$settings = @{
    RUSTFLAGS = $null
    CARGO_ENCODED_RUSTFLAGS = '-C' + [char]31 + 'target-feature=+crt-static'
    CARGO_PROFILE_RELEASE_OPT_LEVEL = 's'
    CARGO_PROFILE_RELEASE_LTO = 'true'
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '1'
    CARGO_PROFILE_RELEASE_STRIP = 'symbols'
    CARGO_TARGET_DIR = $null
}
try {
    foreach ($name in $settings.Keys) {
        $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -eq $settings[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $settings[$name], 'Process') }
    }
    $legacy = $Architecture -eq 'x86'
    $triple = if ($legacy) { 'i686-win7-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
    [string[]]$cargoPrefix = if ($legacy) { @('+nightly-2026-04-15') } else { @() }
    [string[]]$standardLibrary = if ($legacy) { @('-Z','build-std=std,panic_unwind') } else { @() }
    $manifest = (Resolve-Path -LiteralPath $InputSimManifest).Path
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot "target/portable-build/$Architecture/typing"
    & cargo @cargoPrefix build -p typing-windows --release --target $triple @standardLibrary --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Portable Typing build failed' }
    $application = Join-Path $env:CARGO_TARGET_DIR "$triple/release/Typing.exe"
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot "target/portable-build/$Architecture/inputsim"
    & cargo @cargoPrefix build --manifest-path $manifest --release --target $triple @standardLibrary --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Portable InputSim build failed' }
    $inputSim = Join-Path $env:CARGO_TARGET_DIR "$triple/release/inputsim.dll"

    $locator = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $visualStudio = & $locator -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $toolset = Get-ChildItem -LiteralPath (Join-Path $visualStudio 'VC/Tools/MSVC') -Directory |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $dumpbin = Join-Path $toolset.FullName 'bin/Hostx64/x64/dumpbin.exe'
    # Every accepted import is supplied by Windows. Unknown imports stop packaging.
    $systemDlls = @('kernel32.dll', 'ntdll.dll', 'user32.dll', 'gdi32.dll', 'advapi32.dll',
        'shell32.dll', 'comctl32.dll', 'ole32.dll', 'oleaut32.dll', 'bcrypt.dll',
        'ws2_32.dll', 'userenv.dll', 'wtsapi32.dll', 'uxtheme.dll', 'dwmapi.dll', 'imm32.dll',
        'msimg32.dll', 'comdlg32.dll', 'version.dll', 'propsys.dll', 'setupapi.dll', 'cfgmgr32.dll',
        'dwrite.dll', 'shlwapi.dll', 'secur32.dll', 'crypt32.dll', 'normaliz.dll')
    $dependencies = @{}
    foreach ($binary in @($application, $inputSim)) {
        $headers = & $dumpbin /headers $binary
        $machine = if ($legacy) { '14C machine \(x86\)' } else { '8664 machine \(x64\)' }
        if ($LASTEXITCODE -ne 0 -or -not ($headers -match $machine)) { throw "Incorrect binary architecture: $binary" }
        $imports = & $dumpbin /imports $binary
        if ($LASTEXITCODE -ne 0) { throw "Cannot inspect $binary" }
        $names = @($imports | ForEach-Object {
            if ($_ -match '^\s+([a-zA-Z0-9_.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
        } | Sort-Object -Unique)
        if ($names.Count -eq 0) { throw "No import evidence for $binary" }
        foreach ($name in $names) {
            if ($name -match '^(vcruntime|msvcp|msvcr|ucrtbase|api-ms-win-crt-)') {
                throw "Portable static-CRT package imports an external C runtime: $name ($binary)"
            }
            if ($name -notin $systemDlls -and $name -notmatch '^(api|ext)-ms-win-.*\.dll$') {
                throw "Unshipped non-OS dependency in ${binary}: $name"
            }
        }
        if ($legacy -and ($imports -match '\b(GetDpiForWindow|AdjustWindowRectExForDpi|SetProcessDpiAwarenessContext|RegisterSuspendResumeNotification|UnregisterSuspendResumeNotification|WaitOnAddress|WakeByAddressSingle|WakeByAddressAll|GetSystemTimePreciseAsFileTime|GetTempPath2W|SetThreadDescription)\s*$')) {
            throw "Post-Windows-7 API is statically imported: $binary"
        }
        $dependencies[(Split-Path -Leaf $binary)] = $names
    }
    $exports = & $dumpbin /exports $inputSim
    foreach ($symbol in @('resolve_key_v1','resolve_char_v1','submit_v1')) {
        if (-not ($exports -match "\s$symbol(?:\s+=\s+_$symbol)?\s*$")) { throw "Missing undecorated InputSim export: $symbol" }
    }
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
    $packageName = "Typing-$version-win-$Architecture"
    $package = Join-Path $stage $packageName
    $licenses = Join-Path $package 'licenses'
    New-Item -ItemType Directory -Path $licenses -Force | Out-Null
    Copy-Item -LiteralPath $application -Destination (Join-Path $package 'Typing.exe')
    Copy-Item -LiteralPath $inputSim -Destination (Join-Path $package 'inputsim.dll')
    Copy-Item -LiteralPath resources/configuration.ini -Destination (Join-Path $package 'configuration.ini')
    New-Item -ItemType Directory -Path (Join-Path $package 'profiles') | Out-Null
    Copy-Item -LiteralPath resources/profiles/default.typing.ini -Destination (Join-Path $package 'profiles/default.typing.ini')
    Copy-Item -LiteralPath docs/USER_GUIDE.md -Destination (Join-Path $package 'USER_GUIDE.md')
    # Preserve the source license documents' relative links in their own folder.
    $typingNotices = Join-Path $licenses 'Typing'
    $sharedNotices = Join-Path $typingNotices 'licenses'
    New-Item -ItemType Directory -Path $sharedNotices -Force | Out-Null
    foreach ($notice in @('LICENSE','NOTICE','ARTWORK-LICENSE.md','THIRD_PARTY_NOTICES.md')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $notice) -Destination (Join-Path $typingNotices $notice)
    }
    foreach ($notice in @('windows-rs-LICENSE-APACHE.txt','windows-rs-LICENSE-MIT.txt')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "licenses/$notice") -Destination (Join-Path $sharedNotices $notice)
    }
    Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $manifest) 'LICENSE') -Destination (Join-Path $sharedNotices 'InputSim-LICENSE.txt')
    @"
Typing $version - portable Windows $Architecture build

Extract this entire folder, then run Typing.exe. Keep inputsim.dll,
configuration.ini, and the profiles folder together. No separate runtime or framework installation is
required by this build. Both binaries use the statically linked C runtime.
Targets: $(if ($legacy) { 'Windows 7 SP1/10 x86 and Windows 11 via WOW64' } else { 'Windows 11 x64' }).
Windows 11 x64 is the available verification platform. Clean-install deployment,
Windows 7/10 execution, actual sleep/session changes and physical mixed-DPI
monitors remain pending; no clean-machine or legacy-OS verification is claimed.
The x86 build uses Rust's Windows 7 target with a rebuilt standard library.
Before Windows 10 1607, text mapping is stateless and has weaker validation for
unusual custom layouts and shifted dead keys. Newer Windows retains stronger
state-preserving character verification.

Typing starts enabled in the notification area. Right-click its tray icon for
Show Window, one Enable/Disable toggle, and Exit, in that order.
Double-click toggles enabled state.
Closing or minimizing the main window hides it; use tray Exit to quit.
Mapping Profile, Event diagnostics, Settings, and Help use native controls, in that tab order.
In Settings, check Start with Windows to launch Typing in the tray at sign-in for
the current user. Uncheck it to cancel automatic startup. Changes apply immediately.
After moving the portable folder, enable this option again from the new location.
See USER_GUIDE.md for setup, everyday use, profiles, and troubleshooting.

Default configuration: Ctrl+U toggles Typing, Caps Lock or Left Alt alone toggles
command mode. In command mode I/J/K/L move up/left/down/right; see the
Mapping Profile page for other commands. Quit an older Aldetl/Typing instance using the same
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
The distinct command-mode icon and default mappings retain Aldetl's resources.
Help shows a short introduction and examples, followed by Open User Guide… and
About. The full formatted guide opens in a separate resizable, read-only dialog.
Typing's software license, icon reuse terms, and project NOTICE are in
licenses/Typing. The third-party overview is licenses/Typing/THIRD_PARTY_NOTICES.md;
component license files and Rust notices are under licenses.
"@ | Set-Content -LiteralPath (Join-Path $package 'README.txt') -Encoding utf8

    # Use Cargo's resolved source paths rather than assuming a registry cache layout.
    $allMetadata = cargo metadata --format-version 1 --locked --offline --filter-platform $triple | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve dependency notices' }
    foreach ($dependency in $allMetadata.packages | Where-Object { $_.name -like 'windows*' }) {
        $source = Split-Path -Parent $dependency.manifest_path
        foreach ($license in Get-ChildItem -LiteralPath $source -File | Where-Object Name -Match '^license') {
            Copy-Item -LiteralPath $license.FullName -Destination (Join-Path $licenses "$($dependency.name)-$($dependency.version)-$($license.Name).txt")
        }
    }
    $sysroot = if ($legacy) { rustup run nightly-2026-04-15 rustc --print sysroot } else { rustc --print sysroot }
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate Rust notices' }
    Copy-Item -LiteralPath (Join-Path $sysroot 'share/doc/rust/COPYRIGHT-library.html') -Destination (Join-Path $licenses 'Rust-standard-library.html')
    @"
Third-party components

Rust standard library: https://github.com/rust-lang/rust
See Rust-standard-library.html for copyright and license notices.

windows-sys / windows-targets / architecture-specific Windows import libraries:
https://github.com/microsoft/windows-rs
See the component license files in this folder (MIT OR Apache-2.0).

InputSim $inputSimVersion is built from the selected InputSim source through its
public C interface. https://github.com/aldetl/InputSim
Its license is in Typing/licenses/InputSim-LICENSE.txt (Apache License 2.0 alone).
Windows system DLLs are provided by the operating system.
"@ | Set-Content -LiteralPath (Join-Path $licenses 'NOTICES.txt') -Encoding utf8
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
    foreach ($name in $savedEnvironment.Keys) {
        if ($null -eq $savedEnvironment[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
    }
    Pop-Location
}
