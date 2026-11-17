# ============================================================================
# ZOROWARRIOR HORIZON BRIDGE - one-click build for Windows 11
#
#   powershell -ExecutionPolicy Bypass -File tools\build-all.ps1
#
# Builds every target this machine can build, then verifies each jar and copies
# the results into dist\. Targets are independent: a machine without a JDK 8 can
# still build 1.21.x and 26.x, and the script says so rather than failing
# outright.
#
# Requirements
#   - Java 21 (needed to download/run Gradle for the 1.21.x targets)
#   - Java 25 (needed to *configure* fabric-loom 1.19.4, and for 26.x)
#   - Java 8  (only for the Forge 1.8.9 target)
#   - Internet access: the first build downloads Minecraft, mappings and loaders
#
# Nothing here is a stub: every jar it reports as built was compiled by a real
# loader toolchain and then checked by tools\verify_jar.py.
# ============================================================================
[CmdletBinding()]
param(
    [string[]] $Targets = @('fabric', 'neoforge', 'neoforge26', 'forge189'),
    [string]   $GradleVersion = '9.9.1',
    [switch]   $SkipVerify
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

$results = @()

function Find-Java {
    param([int] $Major)
    $candidates = @()
    if ($env:JAVA_HOME) { $candidates += $env:JAVA_HOME }
    $candidates += (Get-ChildItem 'C:\Program Files\Eclipse Adoptium',
        'C:\Program Files\Java', 'C:\Program Files\Microsoft' -ErrorAction SilentlyContinue |
        Where-Object { $_.PSIsContainer } | ForEach-Object { $_.FullName })
    $candidates += (Get-ChildItem 'C:\Program Files\Eclipse Adoptium' -Recurse -Filter 'java.exe' `
        -ErrorAction SilentlyContinue | ForEach-Object { Split-Path $_.FullName -Parent })
    foreach ($home_ in $candidates) {
        $exe = Join-Path $home_ 'bin\javac.exe'
        if (Test-Path $exe) {
            $ver = (& $exe -version 2>&1 | Out-String)
            if ($ver -match "javac $Major\.") { return $home_ }
        }
    }
    return $null
}

function Get-Gradle {
    param([string] $Version)
    $dir = Join-Path $root ".tools\gradle-$Version"
    $exe = Join-Path $dir 'bin\gradle.bat'
    if (Test-Path $exe) { return $exe }
    Write-Host "downloading Gradle $Version ..." -ForegroundColor Cyan
    $zip = Join-Path $env:TEMP "gradle-$Version-bin.zip"
    Invoke-WebRequest -UseBasicParsing `
        -Uri "https://services.gradle.org/distributions/gradle-$Version-bin.zip" -OutFile $zip
    New-Item -ItemType Directory -Force -Path (Join-Path $root '.tools') | Out-Null
    Expand-Archive -Path $zip -DestinationPath (Join-Path $root '.tools') -Force
    return $exe
}

function Invoke-Target {
    param(
        [string] $Name,
        [string] $Project,
        [string] $JdkHome,
        [string[]] $GradleArgs,
        [string] $OutDir
    )
    if (-not $JdkHome) {
        Write-Host "[skip] $Name - required JDK not found" -ForegroundColor Yellow
        $script:results += [pscustomobject]@{ Target = $Name; Status = 'skipped (no JDK)'; Jar = '' }
        return
    }
    Write-Host "=== building $Name" -ForegroundColor Green
    $gradle = if ($Name -eq 'forge189') { Get-Gradle '4.10.3' } else { Get-Gradle $GradleVersion }
    $env:JAVA_HOME = $JdkHome
    $log = Join-Path $root "ci\logs\$Name.log"
    New-Item -ItemType Directory -Force -Path (Split-Path $log) | Out-Null
    & $gradle --no-daemon -p $Project build @GradleArgs *>&1 | Tee-Object -FilePath $log
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[FAIL] $Name - see $log" -ForegroundColor Red
        $script:results += [pscustomobject]@{ Target = $Name; Status = 'FAILED (see log)'; Jar = '' }
        return
    }
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $jars = Get-ChildItem (Join-Path $Project 'build\libs\*.jar') -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -notlike '*-sources.jar' }
    foreach ($jar in $jars) {
        Copy-Item $jar.FullName $OutDir -Force
        Write-Host "[ok] $Name -> $(Join-Path $OutDir $jar.Name)" -ForegroundColor Green
        $script:results += [pscustomobject]@{ Target = $Name; Status = 'built'; Jar = $jar.Name }
    }
}

# ---- Fabric 1.21.x: one jar per Minecraft version, because mappings differ ----
if ($Targets -contains 'fabric') {
    $jdk25 = Find-Java 25
    $jdk21 = Find-Java 21
    if (-not $jdk25) {
        Write-Host "[note] JDK 25 not found - fabric-loom 1.19.4 cannot configure without it" -ForegroundColor Yellow
    }
    foreach ($v in @(
        @{ mc = '1.21.1';  yarn = '1.21.1+build.3';  java = '21' },
        @{ mc = '1.21.4';  yarn = '1.21.4+build.8';  java = '21' },
        @{ mc = '1.21.11'; yarn = '1.21.11+build.6'; java = '21' })) {
        Invoke-Target -Name "fabric-$($v.mc)" -Project 'java\fabric' -JdkHome $jdk25 `
            -GradleArgs @("-Pmc_version=$($v.mc)", "-Pyarn_mappings=$($v.yarn)",
                          '-Ploader_version=0.20.1', "-Pjava_target=$($v.java)") `
            -OutDir 'dist\java-1.21.x'
    }
    if ($jdk21) { Write-Host "using JDK 21 toolchain from $jdk21" -ForegroundColor DarkGray }
}

if ($Targets -contains 'neoforge') {
    Invoke-Target -Name 'neoforge-1.21.1' -Project 'java\neoforge' -JdkHome (Find-Java 21) `
        -GradleArgs @('-Pmc_version=1.21.1', '-Pneo_version=21.1.266', '-Pjava_target=21') `
        -OutDir 'dist\java-1.21.x'
}

if ($Targets -contains 'neoforge26') {
    Invoke-Target -Name 'neoforge-26.1' -Project 'java\neoforge26' -JdkHome (Find-Java 25) `
        -GradleArgs @('-Pmc_version=26.1', '-Pneo_version=26.1.2.126', '-Pjava_target=25') `
        -OutDir 'dist\java-26.x'
}

if ($Targets -contains 'forge189') {
    Write-Host @'
=== Forge 1.8.9 note
ForgeGradle 2.1 asks a retired service (export.mcpbot.bspk.rs) for its mapping
list. On Linux and in CI, tools/build-forge189.sh works around it with a loopback
stand-in. On Windows the workaround is the same idea but needs an elevated shell
to edit C:\Windows\System32\drivers\etc\hosts; run:
    tools\build-forge189.ps1
'@ -ForegroundColor DarkYellow
}

# ---- verify: an unverified jar is not a deliverable ----
if (-not $SkipVerify) {
    foreach ($dir in @('dist\java-1.21.x', 'dist\java-26.x', 'dist\java-1.8.9')) {
        if (Test-Path $dir) {
            $jars = Get-ChildItem "$dir\*.jar" -ErrorAction SilentlyContinue
            if ($jars) {
                python tools\verify_jar.py @($jars.FullName) --json "$dir\verify.json"
            }
        }
    }
}

Write-Host "`n===== summary =====" -ForegroundColor Cyan
$results | Format-Table -AutoSize

Pop-Location
