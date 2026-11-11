param([ValidateSet('x64','x86')][string]$Architecture = 'x64')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    $locator = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $visualStudio = & $locator -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $toolset = Get-ChildItem -LiteralPath (Join-Path $visualStudio 'VC/Tools/MSVC') -Directory |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10'
    $sdk = Get-ChildItem -LiteralPath (Join-Path $sdkRoot 'Include') -Directory |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'um/Windows.h') } |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $compiler = Join-Path $toolset.FullName "bin/Hostx64/$Architecture/cl.exe"
    $includes = @("/I$($toolset.FullName)/include", "/I$($sdk.FullName)/ucrt", "/I$($sdk.FullName)/shared", "/I$($sdk.FullName)/um")
    $libraries = @("/LIBPATH:$($toolset.FullName)/lib/$Architecture", "/LIBPATH:$sdkRoot/Lib/$($sdk.Name)/ucrt/$Architecture", "/LIBPATH:$sdkRoot/Lib/$($sdk.Name)/um/$Architecture")
    $output = Join-Path $projectRoot "target/tests-$Architecture"
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    foreach ($name in @('windows_receiver','native_ui','startup_errors','profiles_ui','language_ui')) {
        & $compiler /nologo /W4 /WX /TC /std:c11 /MT /X @includes "typing-windows/tests/$name.c" "/Fo$output/$name.obj" "/Fe$output/$name.exe" /link @libraries user32.lib gdi32.lib
        if ($LASTEXITCODE -ne 0) { throw "Native test build failed: $name ($Architecture)" }
    }
    & (Join-Path $output 'profiles_ui.exe') --check-paths
    if ($LASTEXITCODE -ne 0) { throw "Profile test path checks failed ($Architecture)" }
    & $compiler /nologo /W4 /WX /TC /std:c11 /MT /LD /X @includes typing-windows/tests/inputsim_failure_proxy.c "/Fo$output/inputsim_failure_proxy.obj" "/Fe$output/inputsim_failure_proxy.dll" /link @libraries "/IMPLIB:$output/inputsim_failure_proxy.lib" /EXPORT:resolve_key_v1 /EXPORT:resolve_char_v1 /EXPORT:submit_v1
    if ($LASTEXITCODE -ne 0) { throw "Failure proxy build failed ($Architecture)" }
} finally { Pop-Location }
