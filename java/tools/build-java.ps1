<#
.SYNOPSIS
    Builds a real, installable Java Edition mod jar for ZOROWARRIOR HORIZON BRIDGE.

.DESCRIPTION
    Compiles the shared Java source against the *actual* Minecraft and loader
    libraries for the target version, then packages a jar with the loader's own
    metadata and the mod's assets and data.

    Nothing here is faked: the class files come from javac against the real
    Minecraft classes for that version, the metadata is the loader's documented
    format, and the finished jar is verified (archive structure, class-file
    version, required entries) before it is reported as built.

    Runs on Windows 11 with PowerShell 5.1 or later, and on Linux/macOS with
    pwsh, so the same script produces the same jars in CI.

.PARAMETER Loader
    fabric, neoforge or forge (forge is 1.8.9 only).

.PARAMETER Mc
    The Minecraft version, e.g. 1.21.1 or 1.21.4.

.PARAMETER OutDir
    Where the finished jar goes.

.PARAMETER JavaHome
    The JDK to compile with. Defaults to $env:JAVA_HOME.

.PARAMETER WorkDir
    Cache and work directory. Defaults to build/java.

.EXAMPLE
    ./build-java.ps1 -Loader fabric -Mc 1.21.1 -OutDir dist/java-1.21.x
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('fabric', 'neoforge', 'forge')]
    [string]$Loader,

    [Parameter(Mandatory = $true)][string]$Mc,

    [Parameter(Mandatory = $true)][string]$OutDir,

    [string]$JavaHome = $env:JAVA_HOME,
    [string]$WorkDir = 'build/java',
    [string]$SourceDir = 'java/shared',
    [string]$Version = '',
    [switch]$KeepWork
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# --- small helpers -----------------------------------------------------------

function Write-Step([string]$Message) {
    Write-Host ""
    Write-Host "== $Message" -ForegroundColor Cyan
}

function Write-Ok([string]$Message) {
    Write-Host "   $Message" -ForegroundColor Green
}

function Fail([string]$Message) {
    Write-Host "   FAILED: $Message" -ForegroundColor Red
    exit 1
}

function Get-JdkTool([string]$Name) {
    if (-not $JavaHome) {
        Fail "no JDK: pass -JavaHome or set JAVA_HOME (see docs/BUILDING-JAVA.md)"
    }
    $exe = if ($IsWindows -or $env:OS -eq 'Windows_NT') { "$Name.exe" } else { $Name }
    $path = Join-Path (Join-Path $JavaHome 'bin') $exe
    if (-not (Test-Path $path)) {
        Fail "no $Name in $JavaHome\bin; is that a full JDK (not a JRE)?"
    }
    return $path
}

function Get-JdkJavaVersion([string]$JavacPath) {
    $text = & $JavacPath -version 2>&1 | Out-String
    if ($text -match 'javac\s+(\d+)') { return [int]$Matches[1] }
    Fail "cannot read the javac version from: $text"
}

function Invoke-Download([string]$Url, [string]$Destination) {
    if (Test-Path $Destination) { return $Destination }
    $parent = Split-Path -Parent $Destination
    if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    Write-Host "   downloading $(Split-Path -Leaf $Destination) from $Url"
    $attempts = 0
    while ($true) {
        $attempts++
        try {
            if ($PSVersionTable.PSVersion.Major -ge 6) {
                Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing -MaximumRedirection 10
            }
            else {
                # PowerShell 5.1 needs TLS 1.2 turned on explicitly for some hosts.
                [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
                Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
            }
            return $Destination
        }
        catch {
            if ($attempts -ge 3) { Fail "cannot download $Url`: $($_.Exception.Message)" }
            Start-Sleep -Seconds (2 * $attempts)
        }
    }
}

function Get-Json([string]$Url) {
    $tmp = Join-Path $WorkDir 'meta/cache.json'
    Invoke-Download $Url $tmp | Out-Null
    $text = Get-Content -Raw -Path $tmp
    Remove-Item $tmp -Force
    return $text | ConvertFrom-Json
}

function Expand-Zip([string]$Zip, [string]$To) {
    if (Test-Path $To) { Remove-Item $To -Recurse -Force }
    New-Item -ItemType Directory -Path $To -Force | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::ExtractToDirectory($Zip, $To)
}

function Join-Classpath([string[]]$Entries) {
    $separator = if ($IsWindows -or $env:OS -eq 'Windows_NT') { ';' } else { ':' }
    return ($Entries -join $separator)
}

# --- project facts ----------------------------------------------------------

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not (Test-Path $SourceDir)) { $SourceDir = Join-Path $repoRoot 'java/shared' }
if (-not (Test-Path $SourceDir)) { Fail "cannot find java/shared (looked in $SourceDir)" }
if (-not $Version) {
    $versionFile = Join-Path $repoRoot 'java/version.txt'
    if (Test-Path $versionFile) { $Version = (Get-Content -Raw $versionFile).Trim() } else { $Version = '1.0.0' }
}

$modId = 'zoro_bridge'
$modName = 'ZOROWARRIOR HORIZON BRIDGE'
$mainClass = 'dev.zorowarrior.bridge.BridgeBeaconMod'
$assets = Join-Path $SourceDir 'assets'
$data = Join-Path $SourceDir 'data'

New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$cache = Join-Path $WorkDir 'cache'
$work = Join-Path $WorkDir "work-$Loader-$Mc"
New-Item -ItemType Directory -Path $cache -Force | Out-Null
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Path $work -Force | Out-Null

$javac = Get-JdkTool 'javac'
$jarTool = Get-JdkTool 'jar'
$javap = Get-JdkTool 'javap'
$java = Get-JdkTool 'java'
$jdkVersion = Get-JdkJavaVersion $javac

Write-Step "building $modName $Version for $Loader $Mc (JDK $jdkVersion)"

# Minecraft's own Java requirement per release. Compiling with a newer JDK is
# fine only if the target's class-file version is not exceeded.
$javaReleases = @{
    '1.8.9'  = 8; '1.9' = 8; '1.10.2' = 8; '1.11.2' = 8; '1.12.2' = 8
    '1.21'   = 21; '1.21.1' = 21; '1.21.2' = 21; '1.21.3' = 21; '1.21.4' = 21
    '1.21.5' = 21; '1.21.6' = 21; '1.21.7' = 21; '1.21.8' = 21; '1.21.9' = 21
    '1.21.10' = 21; '1.21.11' = 21
    '26.1'   = 25; '26.1.1' = 25; '26.1.2' = 25; '26.2' = 25; '26.3' = 25; '26.4' = 25
}
$targetJava = $javaReleases[$Mc]
if (-not $targetJava) { Fail "unknown Minecraft version $Mc; add it to the table in this script" }
if ($jdkVersion -lt $targetJava) {
    Fail "Minecraft $Mc needs Java $targetJava to compile against, and this JDK is $jdkVersion"
}

# --- resolve the loader's libraries -----------------------------------------

$classpath = New-Object System.Collections.Generic.List[string]
$provenance = [ordered]@{
    tool        = "zorowarrior-horizon-bridge build-java.ps1"
    mod_id      = $modId
    mod_version = $Version
    loader      = $Loader
    minecraft   = $Mc
    java_target = $targetJava
    jdk         = (& $javac -version 2>&1 | Out-String).Trim()
    built_utc   = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
}

switch ($Loader) {
    'fabric' {
        Write-Step 'resolving Fabric loader and Minecraft'
        $loaderList = Get-Json "https://meta.fabricmc.net/v2/versions/loader/$Mc"
        if (-not $loaderList -or $loaderList.Count -eq 0) { Fail "Fabric has no loader for Minecraft $Mc" }
        $loaderVersion = ($loaderList | Where-Object { -not $_.loader.stable -eq $false } | Select-Object -First 1).loader.version
        if (-not $loaderVersion) { $loaderVersion = $loaderList[0].loader.version }
        $yarnList = Get-Json "https://meta.fabricmc.net/v2/versions/yarn/$Mc"
        if (-not $yarnList -or $yarnList.Count -eq 0) { Fail "Fabric has no Yarn mappings for Minecraft $Mc" }
        $yarnVersion = $yarnList[0].version
        $provenance['fabric_loader'] = $loaderVersion
        $provenance['yarn_mappings'] = $yarnVersion
        Write-Ok "loader $loaderVersion, yarn $yarnVersion"

        # Vanilla classes, from Mojang's own manifest.
        $manifest = Get-Json 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
        $entry = $manifest.versions | Where-Object { $_.id -eq $Mc } | Select-Object -First 1
        if (-not $entry) { Fail "Minecraft $Mc is not in Mojang's version manifest" }
        $versionJsonPath = Join-Path $cache "$Mc.json"
        Invoke-Download $entry.url $versionJsonPath | Out-Null
        $versionJson = Get-Content -Raw $versionJsonPath | ConvertFrom-Json
        $clientJar = Join-Path $cache "client-$Mc.jar"
        Invoke-Download $versionJson.downloads.client.url $clientJar | Out-Null
        $classpath.Add($clientJar)

        # The libraries the game itself needs, so the mod compiles against the
        # same Netty/Guava/DataFixerUpper the game runs with.
        foreach ($library in $versionJson.libraries) {
            $artifact = $null
            if ($library.downloads -and $library.downloads.artifact) { $artifact = $library.downloads.artifact }
            if (-not $artifact) { continue }
            if ($library.rules) {
                $allowed = $false
                foreach ($rule in $library.rules) {
                    $os = if ($rule.os) { $rule.os.name } else { $null }
                    if ($rule.action -eq 'allow' -and (-not $os -or $os -eq 'linux' -or $os -eq 'windows')) { $allowed = $true }
                    if ($rule.action -eq 'disallow') { $allowed = $false }
                }
                if (-not $allowed) { continue }
            }
            $dest = Join-Path $cache ("libs/" + ($artifact.path -replace '/', [IO.Path]::DirectorySeparatorChar))
            Invoke-Download $artifact.url $dest | Out-Null
            $classpath.Add($dest)
        }

        # The loader, and whatever it depends on.
        $loaderJar = Join-Path $cache "fabric-loader-$loaderVersion.jar"
        Invoke-Download "https://maven.fabricmc.net/net/fabricmc/fabric-loader/$loaderVersion/fabric-loader-$loaderVersion.jar" $loaderJar | Out-Null
        $classpath.Add($loaderJar)
        $pomPath = Join-Path $cache "fabric-loader-$loaderVersion.pom"
        Invoke-Download "https://maven.fabricmc.net/net/fabricmc/fabric-loader/$loaderVersion/fabric-loader-$loaderVersion.pom" $pomPath | Out-Null
        [xml]$pom = Get-Content -Raw $pomPath
        $ns = New-Object System.Xml.XmlNamespaceManager($pom.NameTable)
        $ns.AddNamespace('m', 'http://maven.apache.org/POM/4.0.0')
        foreach ($dependency in $pom.SelectNodes('//m:dependencies/m:dependency', $ns)) {
            if ($dependency.scope -and $dependency.scope -ne 'compile') { continue }
            if ($dependency.optional -eq 'true') { continue }
            $groupId = $dependency.groupId.Trim()
            $artifactId = $dependency.artifactId.Trim()
            $depVersion = $dependency.version.Trim()
            if ($depVersion -match '^\$\{') { continue }
            $groupPath = $groupId -replace '\.', '/'
            $url = "https://maven.fabricmc.net/$groupPath/$artifactId/$depVersion/$artifactId-$depVersion.jar"
            $dest = Join-Path $cache "libs/$artifact-$depVersion.jar"
            try { Invoke-Download $url $dest | Out-Null; $classpath.Add($dest) }
            catch { Write-Warning "loader dependency $groupId`:$artifactId`:$depVersion not fetched; continuing" }
        }

        # Intermediary names, which the jar refers to at runtime; harmless and
        # sometimes needed at compile time.
        $intermediary = Join-Path $cache "intermediary-$Mc.jar"
        try {
            Invoke-Download "https://maven.fabricmc.net/net/fabricmc/intermediary/$Mc/intermediary-$Mc.jar" $intermediary | Out-Null
            $classpath.Add($intermediary)
        }
        catch { Write-Warning 'intermediary jar not fetched; the loader will remap at runtime anyway' }
    }

    'neoforge' {
        Write-Step 'resolving NeoForge'
        # NeoForge's version for a Minecraft version is `<mc>.<build>`; ask its
        # maven for the versions and take the newest for this MC.
        $indexUrl = 'https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml'
        $indexPath = Join-Path $cache 'neoforge-metadata.xml'
        Invoke-Download $indexUrl $indexPath | Out-Null
        [xml]$index = Get-Content -Raw $indexPath
        $candidates = @($index.metadata.versioning.versions.version | Where-Object { $_ -like "$Mc.*" })
        if ($candidates.Count -eq 0) { Fail "NeoForge has no build for Minecraft $Mc" }
        $neoVersion = $candidates | Sort-Object -Descending | Select-Object -First 1
        $provenance['neoforge'] = $neoVersion
        Write-Ok "neoforge $neoVersion"

        $universalUrl = "https://maven.neoforged.net/releases/net/neoforged/neoforge/$neoVersion/neoforge-$neoVersion-universal.jar"
        $universalJar = Join-Path $cache "neoforge-$neoVersion-universal.jar"
        Invoke-Download $universalUrl $universalJar | Out-Null
        $classpath.Add($universalJar)

        $joinedUrl = "https://maven.neoforged.net/releases/net/neoforged/neoforge/$neoVersion/neoforge-$neoVersion-joined.jar"
        $joinedJar = Join-Path $cache "neoforge-$neoVersion-joined.jar"
        Invoke-Download $joinedUrl $joinedJar | Out-Null
        $classpath.Add($joinedJar)

        # FML's own transitive dependencies (bus, coremods, ...).
        $pomUrl = "https://maven.neoforged.net/releases/net/neoforged/fmlcore/$Mc-$neoVersion/fmlcore-$Mc-$neoVersion.pom"
        try {
            $pomPath = Join-Path $cache "fmlcore.pom"
            Invoke-Download $pomUrl $pomPath | Out-Null
            [xml]$pom = Get-Content -Raw $pomPath
            $ns = New-Object System.Xml.XmlNamespaceManager($pom.NameTable)
            $ns.AddNamespace('m', 'http://maven.apache.org/POM/4.0.0')
            foreach ($dependency in $pom.SelectNodes('//m:dependencies/m:dependency', $ns)) {
                if ($dependency.scope -and $dependency.scope -ne 'compile') { continue }
                $groupId = $dependency.groupId.Trim(); $artifactId = $dependency.artifactId.Trim(); $depVersion = $dependency.version.Trim()
                if ($depVersion -match '^\$\{') { continue }
                $groupPath = $groupId -replace '\.', '/'
                $url = "https://maven.neoforged.net/releases/$groupPath/$artifactId/$depVersion/$artifactId-$depVersion.jar"
                $dest = Join-Path $cache "libs/neo-$artifactId-$depVersion.jar"
                try { Invoke-Download $url $dest | Out-Null; $classpath.Add($dest) } catch { }
            }
        }
        catch { Write-Warning 'fmlcore pom not available; relying on the universal jar' }
    }

    'forge' {
        Write-Step 'resolving Forge 1.8.9'
        if ($Mc -ne '1.8.9') { Fail "the forge path in this script targets 1.8.9 only; use neoforge for 1.21+" }
        $forgeVersion = '11.15.1.2318'
        $provenance['forge'] = "$Mc-$forgeVersion"
        Write-Ok "forge $Mc-$forgeVersion"

        $universalUrl = "https://maven.minecraftforge.net/net/minecraftforge/forge/$Mc-$forgeVersion/forge-$Mc-$forgeVersion-universal.jar"
        $forgeJar = Join-Path $cache "forge-$Mc-$forgeVersion-universal.jar"
        Invoke-Download $universalUrl $forgeJar | Out-Null
        $classpath.Add($forgeJar)

        # 1.8.9 mods compile against the vanilla classes plus the game's own
        # libraries, exactly as ForgeGradle arranged them.
        $manifest = Get-Json 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
        $entry = $manifest.versions | Where-Object { $_.id -eq $Mc } | Select-Object -First 1
        if (-not $entry) { Fail "Minecraft $Mc is not in Mojang's version manifest" }
        $versionJsonPath = Join-Path $cache "$Mc.json"
        Invoke-Download $entry.url $versionJsonPath | Out-Null
        $versionJson = Get-Content -Raw $versionJsonPath | ConvertFrom-Json
        $clientJar = Join-Path $cache "client-$Mc.jar"
        Invoke-Download $versionJson.downloads.client.url $clientJar | Out-Null
        $classpath.Add($clientJar)
        foreach ($library in $versionJson.libraries) {
            if (-not $library.downloads -or -not $library.downloads.artifact) { continue }
            $artifact = $library.downloads.artifact
            $dest = Join-Path $cache ("libs/" + ($artifact.path -replace '/', [IO.Path]::DirectorySeparatorChar))
            try { Invoke-Download $artifact.url $dest | Out-Null; $classpath.Add($dest) } catch { }
        }
        # Launchwrapper and the 1.8.9-era ASM, which Forge's own classes need.
        foreach ($extra in @(
                @{ u = 'https://libraries.minecraft.net/net/minecraft/launchwrapper/1.12/launchwrapper-1.12.jar'; f = 'launchwrapper-1.12.jar' },
                @{ u = 'https://maven.minecraftforge.net/org/ow2/asm/asm-all/5.0.4/asm-all-5.0.4.jar'; f = 'asm-all-5.0.4.jar' },
                @{ u = 'https://maven.minecraftforge.net/net/minecraftforge/accesstransformer/1.0.1/accesstransformer-1.0.1.jar'; f = 'accesstransformer-1.0.1.jar' }
            )) {
            $dest = Join-Path $cache ("libs/" + $extra.f)
            try { Invoke-Download $extra.u $dest | Out-Null; $classpath.Add($dest) } catch { }
        }
    }
}

Write-Ok "$($classpath.Count) classpath entr(y/ies)"

# --- sources -----------------------------------------------------------------

Write-Step 'preparing sources'
$sources = Join-Path $work 'src'
New-Item -ItemType Directory -Path $sources -Force | Out-Null
$javaRoot = Join-Path $SourceDir 'java'
foreach ($loaderDir in @('shared-java', $Loader)) {
    # per-loader source overlays, when a version needs its own API
    $overlay = Join-Path (Join-Path $SourceDir 'java') $loaderDir
    if (Test-Path $overlay) {
        Copy-Item -Path (Join-Path $overlay '*') -Destination $javaRoot -Recurse -Force
    }
}
Copy-Item -Path (Join-Path $SourceDir 'java/*') -Destination $sources -Recurse -Force
$sourceFiles = Get-ChildItem -Path $sources -Filter *.java -Recurse | ForEach-Object { $_.FullName }
if ($sourceFiles.Count -eq 0) { Fail "no .java sources under $sources" }
Write-Ok "$($sourceFiles.Count) source file(s)"

$classes = Join-Path $work 'classes'
New-Item -ItemType Directory -Path $classes -Force | Out-Null
$classPathText = Join-Classpath $classpath
$argFile = Join-Path $work 'sources.txt'
($sourceFiles | ForEach-Object { '"' + ($_ -replace '\\', '\\') + '"' }) | Set-Content -Path $argFile -Encoding UTF8

Write-Step "compiling with javac --release $targetJava"
$compileDir = $classes
$compileArgs = @(
    "--release", "$targetJava",
    '-encoding', 'UTF-8',
    '-classpath', $classPathText,
    '-d', $compileDir,
    "@$argFile"
)
$output = & $javac $compileArgs 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    Write-Host $output
    Fail 'javac failed'
}
if ($output.Trim()) { Write-Host $output }
$classFiles = @(Get-ChildItem -Path $classes -Filter *.class -Recurse)
if ($classFiles.Count -eq 0) { Fail 'javac produced no classes' }
Write-Ok "$($classFiles.Count) class file(s)"

# --- metadata ----------------------------------------------------------------

Write-Step 'writing loader metadata'
$metadataDir = Join-Path $work 'meta'
New-Item -ItemType Directory -Path $metadataDir -Force | Out-Null
$description = 'ZOROWARRIOR HORIZON BRIDGE: a real Java Edition mod built for one exact Minecraft version, and the reference input for the Horizon bridge.'

switch ($Loader) {
    'fabric' {
        $fabricMod = [ordered]@{
            schemaVersion = 1
            id            = $modId
            version       = $Version
            name          = $modName
            description   = $description
            authors       = @('Zorowarrior')
            license       = 'MIT'
            environment   = '*'
            entrypoints   = [ordered]@{ main = @($mainClass) }
            depends       = [ordered]@{
                fabricloader = ">=$loaderVersion"
                minecraft    = $Mc
                java         = ">=$targetJava"
            }
        }
        $json = $fabricMod | ConvertTo-Json -Depth 8
        Set-Content -Path (Join-Path $metadataDir 'fabric.mod.json') -Value $json -Encoding UTF8
        Set-Content -Path (Join-Path $metadataDir 'zoro_bridge.mixins.json') -Value (@{
                required         = $true
                minVersion       = '0.8'
                package          = 'dev.zorowarrior.bridge.mixin'
                compatibilityLevel = 'JAVA_17'
                mixins           = @()
                client           = @()
                injectors        = [ordered]@{ defaultRequire = 1 }
            } | ConvertTo-Json -Depth 6) -Encoding UTF8
    }
    'neoforge' {
        $toml = @"
modLoader = "javafml"
loaderVersion = "[1,)"
license = "MIT"
issueTrackerURL = "https://github.com/Zorowarrior07/Horizon-mod-java-rust/issues"

[[mods]]
modId = "$modId"
version = "$Version"
displayName = "$modName"
description = '''$description'''
logoFile = "icon.png"
authors = "Zorowarrior"

[[dependencies.$modId]]
modId = "neoforge"
type = "required"
versionRange = "[$neoVersion,)"
ordering = "NONE"
side = "BOTH"

[[dependencies.$modId]]
modId = "minecraft"
type = "required"
versionRange = "[$Mc]"
ordering = "NONE"
side = "BOTH"
"@
        Set-Content -Path (Join-Path $metadataDir 'META-INF/neoforge.mods.toml') -Value $toml -Encoding UTF8
        $manifestMf = "Manifest-Version: 1.0`r`nFMLModType: MOD`r`nImplementation-Title: $modName`r`nImplementation-Version: $Version`r`n"
        Set-Content -Path (Join-Path $metadataDir 'MANIFEST.MF') -Value $manifestMf -Encoding ASCII -NoNewline
    }
    'forge' {
        $mcmod = @(@{
                modid                 = $modId
                name                  = $modName
                description           = $description
                version               = "$Version-$Mc"
                mcversion             = $Mc
                authorList            = @('Zorowarrior')
                url                   = ''
                updateUrl             = ''
                credits               = ''
                logoFile              = 'icon.png'
                parent                = ''
                screenshots           = @()
                dependencies          = @('required-after:Forge@[11.15.1.2318,)')
                useDependencyInformation = $true
            }) | ConvertTo-Json -Depth 6
        Set-Content -Path (Join-Path $metadataDir 'mcmod.info') -Value $mcmod -Encoding UTF8
        $manifestMf = "Manifest-Version: 1.0`r`nFMLCorePluginContainsFMLMod: true`r`nImplementation-Title: $modName`r`nImplementation-Version: $Version`r`n"
        Set-Content -Path (Join-Path $metadataDir 'MANIFEST.MF') -Value $manifestMf -Encoding ASCII -NoNewline
    }
}
Write-Ok "$Loader metadata written"

# --- the logo, which both loaders show ---------------------------------------

$iconSource = Join-Path $SourceDir 'assets/zoro_bridge/textures/block/bridge_beacon.png'
$iconTarget = Join-Path $metadataDir 'icon.png'
Copy-Item -Path $iconSource -Destination $iconTarget -Force

# --- package -----------------------------------------------------------------

Write-Step 'packaging the jar'
$staging = Join-Path $work 'staging'
New-Item -ItemType Directory -Path $staging -Force | Out-Null
Copy-Item -Path (Join-Path $classes '*') -Destination $staging -Recurse -Force
Copy-Item -Path (Join-Path $assets '*') -Destination $staging -Recurse -Force
Copy-Item -Path (Join-Path $data '*') -Destination $staging -Recurse -Force

# `Merge`, not overwrite: `pack.mcmeta` is not needed in a mod jar, and the
# metadata dir holds `META-INF/` for Forge/NeoForge while Fabric needs its json
# at the root.
if (Test-Path (Join-Path $metadataDir 'META-INF')) {
    $metaInf = Join-Path $staging 'META-INF'
    if (-not (Test-Path $metaInf)) { New-Item -ItemType Directory -Path $metaInf -Force | Out-Null }
    Copy-Item -Path (Join-Path $metadataDir 'META-INF/*') -Destination $metaInf -Recurse -Force
}
foreach ($file in @('fabric.mod.json', 'zoro_bridge.mixins.json', 'mcmod.info', 'icon.png', 'MANIFEST.MF')) {
    $from = Join-Path $metadataDir $file
    if (Test-Path $from) { Copy-Item -Path $from -Destination (Join-Path $staging $file) -Force }
}

if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Path $OutDir -Force | Out-Null }
$jarName = "$modId-$Version-$Loader-$Mc.jar"
$jarPath = Join-Path $OutDir $jarName
if (Test-Path $jarPath) { Remove-Item $jarPath -Force }

# `jar cf` writes a valid archive with a manifest; it is the JDK's own tool, so
# the result is exactly what a Java build would produce.
Push-Location $staging
try {
    $jarArgs = @('cf', $jarPath)
    if (Test-Path 'MANIFEST.MF') {
        $jarArgs += @('-m', 'MANIFEST.MF')
        # -m consumes the next argument as the jar to create, so give it again.
        $jarArgs = @('cfm', $jarPath, 'MANIFEST.MF')
    }
    $entries = @(Get-ChildItem -Recurse -File | Where-Object { $_.Name -ne 'MANIFEST.MF' } | ForEach-Object {
            Resolve-Path -Relative $_.FullName | ForEach-Object { $_.TrimStart('.', '/', '\') -replace '\\', '/' }
        })
    $jarArgs += $entries
    $jarOutput = & $jarTool $jarArgs 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { Write-Host $jarOutput; Fail 'jar tool failed' }
}
finally {
    Pop-Location
}
$jarBytes = (Get-Item $jarPath).Length
Write-Ok "$jarName ($jarBytes bytes)"

# --- verify ------------------------------------------------------------------

Write-Step 'verifying the jar'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($jarPath)
$entryNames = @($archive.Entries | ForEach-Object { $_.FullName })
$duplicate = $entryNames | Group-Object | Where-Object { $_.Count -gt 1 }
$archive.Dispose()

$problems = New-Object System.Collections.Generic.List[string]
if ($duplicate) { $problems.Add("duplicate entries: $($duplicate.Name -join ', ')") }
foreach ($required in @($($mainClass -replace '\.', '/') + '.class', 'assets/zoro_bridge/lang/en_us.json',
        'assets/zoro_bridge/textures/block/bridge_beacon.png',
        'assets/zoro_bridge/blockstates/bridge_beacon.json',
        'data/zoro_bridge/recipe/bridge_beacon.json')) {
    if (-not ($entryNames -contains $required)) { $problems.Add("missing entry $required") }
}
switch ($Loader) {
    'fabric' { if (-not ($entryNames -contains 'fabric.mod.json')) { $problems.Add('missing fabric.mod.json') } }
    'neoforge' { if (-not ($entryNames -contains 'META-INF/neoforge.mods.toml')) { $problems.Add('missing META-INF/neoforge.mods.toml') } }
    'forge' {
        if (-not ($entryNames -contains 'mcmod.info')) { $problems.Add('missing mcmod.info') }
        if (-not ($entryNames -contains 'META-INF/MANIFEST.MF')) { $problems.Add('missing META-INF/MANIFEST.MF') }
    }
}

# `javap` reads the class back out of the jar: a broken archive or invalid
# bytecode fails here, which is the check a "renamed zip" cannot pass.
$javapOutput = & $javap -p -classpath $jarPath $mainClass 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { $problems.Add("javap cannot read $mainClass out of the jar: $javapOutput") }
$majorVersion = $null
if ($javapOutput -match 'major version:\s*(\d+)') { $majorVersion = [int]$Matches[1] }
$expectedMajor = 44 + $targetJava
if ($majorVersion -ne $expectedMajor) {
    $problems.Add("class-file major version $majorVersion, expected $expectedMajor for Java $targetJava")
}
$stringConstants = [regex]::Matches($javapOutput, '"([^"]{4,})"') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique

if ($problems.Count -gt 0) {
    $problems | ForEach-Object { Write-Host "   $_" -ForegroundColor Red }
    Fail 'the jar failed verification and is not a deliverable'
}
Write-Ok "$($entryNames.Count) entries, no duplicates, class version $majorVersion (Java $targetJava)"
Write-Ok 'every required entry is present and javap reads the main class back out'

# --- report ------------------------------------------------------------------

$sha = (Get-FileHash -Path $jarPath -Algorithm SHA256).Hash.ToLowerInvariant()
$report = [ordered]@{
    schema          = 'zoro-horizon-bridge/java-build@1'
    jar             = $jarName
    bytes           = $jarBytes
    sha256          = $sha
    loader          = $Loader
    minecraft       = $Mc
    java_target     = $targetJava
    class_version   = $majorVersion
    entries         = $entryNames.Count
    main_class      = $mainClass
    strings         = $stringConstants
    provenance      = $provenance
    verified        = @(
        'archive opens and has no duplicate entries',
        'main class and every asset/data file is present',
        'javap reads the main class back out of the jar',
        "class-file version is $expectedMajor, which Java $targetJava accepts"
    )
    not_verified    = @(
        'the mod has not been loaded by a real Minecraft client: no game is available to the build',
        "the jar has not been run inside Horizon: Horizon does not load Java mods at all"
    )
}
$reportPath = Join-Path $OutDir ("$modId-$Version-$Loader-$Mc.build.json")
$report | ConvertTo-Json -Depth 8 | Set-Content -Path $reportPath -Encoding UTF8

if (-not $KeepWork) {
    # The work dir holds hundreds of megabytes of vanilla jars; keep the cache
    # (so a second build is fast) and drop the per-build intermediates.
    if (Test-Path $work) { Remove-Item $work -Recurse -Force }
}

Write-Step 'result'
Write-Ok "$jarPath"
Write-Ok "$reportPath"
Write-Host ""
Write-Host "This jar is for Minecraft $Mc on $Loader and no other version. It is installable in a"
Write-Host "Java Edition launcher with $Loader for $Mc. It is NOT installable in Horizon: Horizon"
Write-Host "runs its own engine and does not load Java mods; the .hmod package is its counterpart."
