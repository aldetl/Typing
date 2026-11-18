# Shared build/inspection and shipped notices for portable ZIP and MSIX production.
function Build-TypingWindowsRuntime {
    param([string]$ProjectRoot, [string]$InputSimManifest,
        [ValidateSet('x64','x86')][string]$Architecture, [string]$OutputRoot)
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
    $env:CARGO_TARGET_DIR = Join-Path $OutputRoot 'typing'
    & cargo @cargoPrefix build -p typing-windows --release --target $triple @standardLibrary --locked --offline | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Portable TypingVE build failed' }
    $application = Join-Path $env:CARGO_TARGET_DIR "$triple/release/TypingVE.exe"
    $env:CARGO_TARGET_DIR = Join-Path $OutputRoot 'inputsim'
    & cargo @cargoPrefix build --manifest-path $manifest --release --target $triple @standardLibrary --locked --offline | Out-Host
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
        'dwrite.dll', 'shlwapi.dll', 'secur32.dll', 'crypt32.dll', 'normaliz.dll', 'gdiplus.dll')
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
        if ($legacy -and ($imports -match '\b(GetDpiForWindow|AdjustWindowRectExForDpi|SetProcessDpiAwarenessContext|RegisterSuspendResumeNotification|UnregisterSuspendResumeNotification|WaitOnAddress|WakeByAddressSingle|WakeByAddressAll|GetSystemTimePreciseAsFileTime|GetTempPath2W|SetThreadDescription|GetCurrentPackageFamilyName|RoInitialize|RoUninitialize|WindowsCreateString|WindowsDeleteString|WindowsGetStringRawBuffer)\s*$')) {
            throw "Post-Windows-7 API is statically imported: $binary"
        }
        $dependencies[(Split-Path -Leaf $binary)] = $names
    }
    $exports = & $dumpbin /exports $inputSim
    foreach ($symbol in @('resolve_key_v1','resolve_char_v1','submit_v1')) {
        if (-not ($exports -match "\s$symbol(?:\s+=\s+_$symbol)?\s*$")) { throw "Missing undecorated InputSim export: $symbol" }
    }

    $sdk = Get-ChildItem -LiteralPath (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin') -Directory |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'x64/rc.exe') } |
        Sort-Object Name | Select-Object -Last 1
    if (-not $sdk) { throw 'Windows SDK resource tools were not found' }
    [pscustomobject]@{
        Legacy=$legacy; Target=$triple; InputSimManifest=$manifest
        Application=$application; InputSim=$inputSim; Imports=$dependencies
        MsvcToolset=$toolset.FullName; InspectionTool=$dumpbin; SdkBin=$sdk.FullName
        Rust=$(if ($legacy) { & rustup run nightly-2026-04-15 rustc -vV } else { & rustc -vV })
    }
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        if ($null -eq $savedEnvironment[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
    }
}
}

function Copy-TypingWindowsLicenses {
    param([string]$ProjectRoot, [string]$Destination, [string]$InputSimManifest,
        [string]$InputSimVersion, [string]$Target, [switch]$Legacy)
    $licenses=$Destination
    $manifest=$InputSimManifest
    $triple=$Target
    New-Item -ItemType Directory -Path $licenses -Force | Out-Null
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

windows / windows-core (x64), windows-sys / windows-targets / Windows import libraries:
https://github.com/microsoft/windows-rs
See the component license files in this folder (MIT OR Apache-2.0).

InputSim $inputSimVersion is built from the selected InputSim source through its
public C interface. https://github.com/aldetl/InputSim
Its license is in Typing/licenses/InputSim-LICENSE.txt (Apache License 2.0 alone).
Windows system DLLs are provided by the operating system.
"@ | Set-Content -LiteralPath (Join-Path $licenses 'NOTICES.txt') -Encoding utf8

}
