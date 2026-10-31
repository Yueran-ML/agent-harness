<#
.SYNOPSIS
Builds the static OpenColorIO that the crate columbina-ocio links, into the directory that
OCIO_DIR names.

.DESCRIPTION
The recipe of "Building OCIO" in the crate's documentation (src/lib.rs), the same for CI
(.github/workflows/ci.yml) and for a build by hand: the source tarball of the release, checked
against its SHA-256; OCIO's own CMake, which fetches its dependencies (expat, yaml-cpp, Imath,
pystring, minizip-ng and zlib) by their tags; Ninja and Release, with the static C runtime of
Microsoft (/MT) for OCIO and every dependency (cmake/msvc-static-crt.cmake), as the engine links it
(engine/.cargo/config.toml: target-feature=+crt-static); no apps, tests, Python, docs, OpenFX or
Java. The libraries of the dependencies are copied into the lib/ of the prefix beside OCIO's, so
that the prefix alone is OCIO_DIR.

The compiler is that of the latest Visual Studio with the x64 C++ tools, set up by its vcvars64;
CMake and Ninja are those that come with it, as on the machine where the recipe was made, or
those on PATH if it has none.

.PARAMETER Prefix
The directory to install into (CMAKE_INSTALL_PREFIX), which OCIO_DIR names afterwards.

.PARAMETER WorkDir
Where the source is unpacked (src/), built (build/) and, if it is not given, downloaded.

.PARAMETER Version
The version of OpenColorIO. Only versions whose tarball's SHA-256 is written below are built.

.PARAMETER Tarball
A tarball of the source already on disk, used in place of downloading one. It is checked against
the same SHA-256.

.PARAMETER Jobs
The number of jobs that CMake builds with.

.EXAMPLE
build-ocio.ps1 -Prefix D:\ocio\install -WorkDir D:\ocio -Tarball D:\ocio\OpenColorIO-2.6.0.tar.gz
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Prefix,
    [string] $WorkDir = (Join-Path ([IO.Path]::GetTempPath()) 'ocio-build'),
    [string] $Version = '2.6.0',
    [string] $Tarball,
    [int] $Jobs = 6
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# The "Source code (tar.gz)" of each release on GitHub: a git archive of its tag.
$Sha256 = @{
    '2.6.0' = '784ac37bde5b9c6e5dc15f23d1235bd1452d376cdc03627d6d6dd310f375a735'
}
if (-not $Sha256.ContainsKey($Version)) {
    throw "No SHA-256 is known for OpenColorIO $Version; add it to `$Sha256 in $PSCommandPath."
}

function Resolve-Full([string] $Path) {
    $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)
}

# A program, with its arguments, that has to succeed.
function Invoke-Checked([string] $Program, [string[]] $Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program $($Arguments -join ' ') failed with exit code $LASTEXITCODE."
    }
}

$Prefix = Resolve-Full $Prefix
$WorkDir = Resolve-Full $WorkDir
$CMakeDir = Join-Path $PSScriptRoot '..\cmake'
$Toolchain = ((Resolve-Path (Join-Path $CMakeDir 'msvc-static-crt.cmake')).Path) -replace '\\', '/'
$Source = Join-Path $WorkDir 'src'
$Build = Join-Path $WorkDir 'build'
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null

# The source.
if ($Tarball) {
    $Tarball = Resolve-Full $Tarball
} else {
    $Tarball = Join-Path $WorkDir "OpenColorIO-$Version.tar.gz"
    $Url = "https://github.com/AcademySoftwareFoundation/OpenColorIO/archive/refs/tags/v$Version.tar.gz"
    Write-Host "Downloading $Url"
    Invoke-Checked "$env:SystemRoot\System32\curl.exe" @('--fail', '--silent', '--show-error', '--location', '--retry', '3', '--output', $Tarball, $Url)
}
$Actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Tarball).Hash.ToLowerInvariant()
if ($Actual -ne $Sha256[$Version]) {
    throw "$Tarball has the SHA-256 $Actual, not $($Sha256[$Version]) of OpenColorIO $Version."
}
Write-Host "SHA-256 of $Tarball is $Actual, as expected"
if (Test-Path -LiteralPath $Source) { Remove-Item -Recurse -Force -LiteralPath $Source }
New-Item -ItemType Directory -Force -Path $Source | Out-Null
# The tar of Windows (bsdtar): a tar of Git or MSYS on PATH would take C: for a host.
Invoke-Checked "$env:SystemRoot\System32\tar.exe" @('-xzf', $Tarball, '-C', $Source)
$Tree = Join-Path $Source "OpenColorIO-$Version"

# The compiler, from vcvars64 of the latest Visual Studio with the x64 tools.
$VsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$Vs = & $VsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $Vs) { throw 'No Visual Studio with the x64 C++ tools was found.' }
$VcVars = Join-Path $Vs 'VC\Auxiliary\Build\vcvars64.bat'
$Environment = & cmd.exe /d /c call $VcVars '>nul' '&&' set
if ($LASTEXITCODE -ne 0) { throw "$VcVars failed with exit code $LASTEXITCODE." }
foreach ($Line in $Environment) {
    if ($Line -match '^([^=]+)=(.*)$') {
        [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
    }
}
$VsCMake = Join-Path $Vs 'Common7\IDE\CommonExtensions\Microsoft\CMake'
foreach ($Dir in @((Join-Path $VsCMake 'Ninja'), (Join-Path $VsCMake 'CMake\bin'))) {
    if (Test-Path -LiteralPath $Dir) { $env:PATH = "$Dir;$env:PATH" }
}
Write-Host "Visual Studio: $Vs (MSVC $env:VCToolsVersion)"
Invoke-Checked 'cmake' @('--version')
Invoke-Checked 'ninja' @('--version')

# OCIO and its dependencies.
Invoke-Checked 'cmake' @(
    '-S', $Tree, '-B', $Build, '-G', 'Ninja',
    '-DCMAKE_BUILD_TYPE=Release',
    "-DCMAKE_INSTALL_PREFIX=$($Prefix -replace '\\', '/')",
    '-DCMAKE_POLICY_DEFAULT_CMP0091=NEW',
    '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded',
    "-DCMAKE_TOOLCHAIN_FILE=$Toolchain",
    '-DBUILD_SHARED_LIBS=OFF',
    '-DOCIO_BUILD_APPS=OFF',
    '-DOCIO_BUILD_TESTS=OFF',
    '-DOCIO_BUILD_GPU_TESTS=OFF',
    '-DOCIO_BUILD_PYTHON=OFF',
    '-DOCIO_BUILD_DOCS=OFF',
    '-DOCIO_BUILD_OPENFX=OFF',
    '-DOCIO_BUILD_JAVA=OFF',
    '-DOCIO_INSTALL_EXT_PACKAGES=ALL'
)
Invoke-Checked 'cmake' @('--build', $Build, '--parallel', "$Jobs")
Invoke-Checked 'cmake' @('--install', $Build)

# The dependencies beside OCIO, so that the prefix has every library to link.
$Lib = Join-Path $Prefix 'lib'
Copy-Item -Force -Path (Join-Path $Build 'ext\dist\lib\*.lib') -Destination $Lib
if (-not (Test-Path -LiteralPath (Join-Path $Prefix 'include\OpenColorIO\OpenColorIO.h'))) {
    throw "$Prefix has no include\OpenColorIO\OpenColorIO.h after the install."
}
Get-ChildItem -LiteralPath $Lib -Filter '*.lib' | ForEach-Object { Write-Host "  $($_.Name)  $($_.Length) bytes" }
Write-Host "OpenColorIO $Version is in $Prefix (OCIO_DIR)"
