# ============================================================================
# ZOROWARRIOR HORIZON BRIDGE - build the Forge 1.8.9 jar on Windows
#
#   powershell -ExecutionPolicy Bypass -File tools\build-forge189.ps1
#
# Two obstacles, both real, both handled here:
#
#  1. Forge 1.8.9 requires ForgeGradle 2.1.2, which only runs on Gradle 4.x and
#     needs a JDK 8. (ForgeGradle 2.3 rejects 1.8.9: "MIN: 1.12".)
#
#  2. ForgeGradle 2.1.2 fetches its mapping list from export.mcpbot.bspk.rs at
#     configuration time. That service is retired, so every current guide fails
#     with "Unable to obtain url ... with etag!".
#
# The mappings themselves still exist on Forge's maven, and so does a versions.json
# in the schema ForgeGradle expects. This script serves that genuine file on
# http://127.0.0.1 and points the retired hostname at loopback in the hosts file
# for the duration of the build, then removes the entry again. No mapping data is
# invented.
#
# Requires: an elevated PowerShell (to edit hosts) and a JDK 8.
# ============================================================================
[CmdletBinding()]
param([string] $Jdk8, [string] $OutDir = 'dist\java-1.8.9')

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$hostsPath = "$env:SystemRoot\System32\drivers\etc\hosts"
$marker = '# zoro-horizon-bridge temporary mcpbot stand-in'

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
        ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'This script edits the hosts file; re-run PowerShell as Administrator.'
}

# --- locate a JDK 8 ---
if (-not $Jdk8) {
    $Jdk8 = @(
        $env:JAVA_HOME,
        'C:\Program Files\Eclipse Adoptium\jdk-8*',
        'C:\Program Files\Java\jdk1.8*'
    ) | Where-Object { $_ } | ForEach-Object { Get-Item $_ -ErrorAction SilentlyContinue } |
        Where-Object { Test-Path (Join-Path $_.FullName 'bin\javac.exe') } |
        Where-Object { (& (Join-Path $_.FullName 'bin\javac.exe') -version 2>&1) -match '1\.8' } |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $Jdk8) { throw 'No JDK 8 found. Install one or pass -Jdk8 <path>.' }

# --- genuine versions.json, served locally ---
$srv = Join-Path $env:TEMP 'zoro-mcp-srv'
New-Item -ItemType Directory -Force -Path $srv | Out-Null
Invoke-WebRequest -UseBasicParsing `
    -Uri 'https://maven.minecraftforge.net/de/oceanlabs/mcp/versions.json' `
    -OutFile (Join-Path $srv 'versions.json')

$listener = Start-Process python -ArgumentList '-m', 'http.server', '80' -WorkingDirectory $srv `
    -PassThru -WindowStyle Hidden
Add-Content $hostsPath "127.0.0.1 export.mcpbot.bspk.rs $marker"

try {
    # --- Gradle 4.10.3 ---
    $gradleDir = Join-Path $root '.tools\gradle-4.10.3'
    if (-not (Test-Path (Join-Path $gradleDir 'bin\gradle.bat'))) {
        $zip = Join-Path $env:TEMP 'gradle-4.10.3-bin.zip'
        Invoke-WebRequest -UseBasicParsing `
            -Uri 'https://services.gradle.org/distributions/gradle-4.10.3-bin.zip' -OutFile $zip
        Expand-Archive -Path $zip -DestinationPath (Join-Path $root '.tools') -Force
    }

    $env:JAVA_HOME = $Jdk8
    Push-Location (Join-Path $root 'java\forge189')
    & (Join-Path $gradleDir 'bin\gradle.bat') --no-daemon build
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { throw "Forge 1.8.9 build failed (exit $code)" }

    New-Item -ItemType Directory -Force -Path (Join-Path $root $OutDir) | Out-Null
    Get-ChildItem (Join-Path $root 'java\forge189\build\libs\*.jar') |
        Where-Object { $_.Name -notlike '*-sources.jar' } |
        ForEach-Object {
            Copy-Item $_.FullName (Join-Path $root $OutDir) -Force
            Write-Host "built: $(Join-Path $OutDir $_.Name)" -ForegroundColor Green
        }

    python (Join-Path $root 'tools\verify_jar.py') `
        (Get-ChildItem (Join-Path $root "$OutDir\*.jar")).FullName
} finally {
    # leave the machine as found, always
    (Get-Content $hostsPath) | Where-Object { $_ -notmatch [regex]::Escape($marker) } |
        Set-Content $hostsPath
    if ($listener -and -not $listener.HasExited) { Stop-Process -Id $listener.Id -Force }
}
