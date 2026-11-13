param([switch]$BuildOnly, [switch]$Portable, [switch]$LiveOnly,
    [ValidateSet('x64','x86')][string]$Architecture = 'x64')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
$languageValidationEnvironmentChanged = $false
$originalLanguageValidationAppData = $null
try {
    if ($BuildOnly -and $LiveOnly) { throw 'BuildOnly and LiveOnly cannot be combined.' }
    if (-not $LiveOnly) {
    cargo fmt --all --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
    cargo test --workspace --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
    cargo clippy --workspace --locked --offline --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Lint checks failed' }
    cargo build --workspace --release --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    if ($Architecture -eq 'x86') {
        cargo +nightly-2026-04-15 test --workspace --target i686-win7-windows-msvc -Z build-std=std,test,panic_unwind --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Legacy x86 tests failed' }
        cargo +nightly-2026-04-15 clippy --workspace --target i686-win7-windows-msvc -Z build-std=std,test,panic_unwind --locked --offline --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'Legacy x86 lint checks failed' }
    }
    $savedCargoTarget = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target/inputsim'
        cargo build --manifest-path ../inputsim/Cargo.toml --release --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'InputSim build failed' }
    } finally { $env:CARGO_TARGET_DIR = $savedCargoTarget }
    & ./tools/build_windows_tests.ps1 -Architecture $Architecture
    }
    $testTools = Join-Path $projectRoot "target/tests-$Architecture"
    $legacyFixture = Get-Content -Raw -LiteralPath typing-core/tests/fixtures/aldetl-installed.ini
    # Retain legacy application settings and exercise the current shipped
    # mappings. The original fixture remains unchanged for import comparisons.
    $fixture = $legacyFixture.Substring(0, $legacyFixture.IndexOf('[modekeys]', [StringComparison]::Ordinal)) +
        (Get-Content -Raw -LiteralPath resources/profiles/default.typing.ini)
    $fixture += [Environment]::NewLine + "[common]" + [Environment]::NewLine + "hotkey=Control+Shift+F12" + [Environment]::NewLine + "[modekeys]" + [Environment]::NewLine +
        "Left=Right" + [Environment]::NewLine + "Z=Control+Shift+F12" + [Environment]::NewLine + "A=step1000000" + [Environment]::NewLine +
        "[hotkeys]" + [Environment]::NewLine + "F10=Control+Shift+F12" + [Environment]::NewLine
    Set-Content -LiteralPath target/integration.ini -Value $fixture -Encoding utf8
    if (-not $BuildOnly) {
        # The existing native assertions use English labels. Isolate their
        # language preference from the user's selection for the whole suite.
        $languageValidationAppData = Join-Path $projectRoot ("target/language-validation-user-" + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path (Join-Path $languageValidationAppData 'Typing') -Force | Out-Null
        [IO.File]::WriteAllText((Join-Path $languageValidationAppData 'Typing/language.txt'), "en`n")
        $originalLanguageValidationAppData = [Environment]::GetEnvironmentVariable('LOCALAPPDATA', 'Process')
        [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $languageValidationAppData, 'Process')
        $languageValidationEnvironmentChanged = $true
        $application = (Resolve-Path -LiteralPath target/release/Typing.exe).Path
        $dll = (Resolve-Path -LiteralPath target/inputsim/release/inputsim.dll).Path
        if ($Architecture -eq 'x86' -and -not $Portable) {
            throw 'Use -Portable with -Architecture x86 to verify the matched legacy executable and DLL.'
        }
        if ($Portable) {
            $metadata = cargo metadata --no-deps --format-version 1 --locked --offline | ConvertFrom-Json
            if ($LASTEXITCODE -ne 0) { throw 'Cannot read release version' }
            $version = ($metadata.packages | Where-Object name -eq 'typing-windows').version
            $release = Get-Content -LiteralPath "target/dist/Typing-$version-win-$Architecture.manifest.json" -Raw | ConvertFrom-Json
            if ((Get-FileHash -LiteralPath $release.archive -Algorithm SHA256).Hash -ne $release.archiveSha256) {
                throw 'Release archive changed after packaging'
            }
            foreach ($entry in $release.files) {
                if ((Get-FileHash -LiteralPath (Join-Path $release.extractedPackage $entry.path) -Algorithm SHA256).Hash -ne $entry.sha256) {
                    throw "Extracted release file changed: $($entry.path)"
                }
            }
            $application = Join-Path $release.extractedPackage 'Typing.exe'
            $dll = Join-Path $release.extractedPackage 'inputsim.dll'
        }
        $configuration = (Resolve-Path -LiteralPath target/integration.ini).Path
        & (Join-Path $testTools 'startup_errors.exe') $application (Join-Path $projectRoot 'target') | Tee-Object -FilePath "target/startup-errors-$Architecture-live.log"
        if ($LASTEXITCODE -ne 0) { throw 'Startup error verification failed' }
        $oldProxyPath = $env:TYPING_TEST_INPUTSIM
        try {
            $env:TYPING_TEST_INPUTSIM = $dll
            $proxy = Join-Path $testTools 'inputsim_failure_proxy.dll'
            $hostLog = Join-Path $projectRoot "target/integration"
            & (Join-Path $testTools 'windows_receiver.exe') $application $dll $configuration $hostLog all $proxy | Tee-Object -FilePath "target/integration-$Architecture-live.log"
            if ($LASTEXITCODE -ne 0) {
                Get-ChildItem -LiteralPath target -Filter 'integration-*.log' |
                    Where-Object { $_.Name -ne 'integration-build.log' -and $_.Name -ne 'integration-validation.log' } |
                    ForEach-Object { Get-Content -LiteralPath $_.FullName | Select-Object -Last 8 }
                throw 'Typing live integration failed'
            }
        } finally {
            $env:TYPING_TEST_INPUTSIM = $oldProxyPath
        }
        & (Join-Path $testTools 'native_ui.exe') $application $dll $configuration (Join-Path $projectRoot "target/native-ui-$Architecture.log") test |
            Tee-Object -FilePath "target/native-ui-$Architecture-live.log"
        if ($LASTEXITCODE -ne 0) { throw 'Native UI verification failed' }
        $profileTestRoot = Join-Path $projectRoot ("target/profile-native-$Architecture-" + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $profileTestRoot | Out-Null
        # Exercise argument normalization through the real Save As/Import dialogs.
        & (Join-Path $testTools 'profiles_ui.exe') $application $dll $profileTestRoot.Replace('\','/') |
            Tee-Object -FilePath "target/profiles-ui-$Architecture-live.log"
        if ($LASTEXITCODE -ne 0) { throw 'Profile UI verification failed' }
        $languageTestRoot = Join-Path $projectRoot ("target/language-native-$Architecture-" + [guid]::NewGuid().ToString('N'))
        & (Join-Path $testTools 'language_ui.exe') $application $dll $languageTestRoot (Join-Path $projectRoot 'resources/languages') |
            Tee-Object -FilePath "target/language-ui-$Architecture-live.log"
        if ($LASTEXITCODE -ne 0) { throw 'Language UI verification failed' }
        if ($Portable) {
            $nativeTest = Join-Path $testTools 'native_ui.exe'
            Push-Location -LiteralPath ([IO.Path]::GetTempPath())
            try {
                & $nativeTest $application '-' '-' (Join-Path $projectRoot "target/portable-default-$Architecture.log") test |
                    Tee-Object -FilePath (Join-Path $projectRoot "target/portable-default-$Architecture-live.log")
                if ($LASTEXITCODE -ne 0) { throw 'Portable default-resource launch or UI verification failed' }
            } finally { Pop-Location }
        }
    }
} finally {
    if ($languageValidationEnvironmentChanged) {
        [Environment]::SetEnvironmentVariable('LOCALAPPDATA', $originalLanguageValidationAppData, 'Process')
    }
    Pop-Location
}
