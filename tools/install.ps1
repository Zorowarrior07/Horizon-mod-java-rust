# ============================================================================
# ZOROWARRIOR HORIZON BRIDGE - install the jars into a Minecraft launcher
#
#   powershell -ExecutionPolicy Bypass -File tools\install.ps1 -MinecraftDir "$env:APPDATA\.minecraft"
#
# Copies each jar into the mods folder of the launch profile that matches it.
# It does NOT install loaders (Fabric/Forge/NeoForge) - those are launcher
# profiles and must already exist. This script prints exactly what it did.
# ============================================================================
[CmdletBinding()]
param(
    [string] $MinecraftDir = (Join-Path $env:APPDATA '.minecraft'),
    [string] $DistDir = 'dist',
    [switch] $WhatIf
)
$root = Split-Path -Parent $PSScriptRoot
$plan = @(
    @{ Dir = 'java-1.8.9';  Loader = 'Forge 1.8.9';    Targets = @('1.8.9') },
    @{ Dir = 'java-1.21.x'; Loader = 'Fabric/NeoForge'; Targets = @('1.21.1', '1.21.4', '1.21.11') },
    @{ Dir = 'java-26.x';   Loader = 'NeoForge 26.x';  Targets = @('26.1') }
)
$modsRoot = Join-Path $MinecraftDir 'mods'
Write-Host "Minecraft directory: $MinecraftDir"
foreach ($entry in $plan) {
    $src = Join-Path $root (Join-Path $DistDir $entry.Dir)
    if (-not (Test-Path $src)) { continue }
    foreach ($jar in Get-ChildItem "$src\*.jar") {
        # The jar name carries the loader and the exact Minecraft version, so we
        # can file it without guessing.
        $target = if ($jar.Name -match 'fabric-([0-9.]+)') { "$($Matches[1])-fabric" }
                  elseif ($jar.Name -match 'neoforge-([0-9.]+)') { "$($Matches[1])-neoforge" }
                  elseif ($jar.Name -match 'forge-([0-9.]+)') { "$($Matches[1])-forge" }
                  else { 'unknown' }
        $dest = Join-Path $modsRoot $target
        if ($WhatIf) {
            Write-Host "[whatif] $($jar.Name) -> $dest"
        } else {
            New-Item -ItemType Directory -Force -Path $dest | Out-Null
            Copy-Item $jar.FullName $dest -Force
            Write-Host "[ok] $($jar.Name) -> $dest" -ForegroundColor Green
        }
    }
}
Write-Host @'

Reminder: only jars go in mods\. The Horizon launcher does not run jars at all -
install horizon\*.hmod there instead.
'@ -ForegroundColor Cyan
