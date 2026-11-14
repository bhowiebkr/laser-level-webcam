# Builds the Windows installer: dist\laser-level-webcam-<version>-windows-x64.msi
#
# Needs the WiX Toolset 5 .NET tool and its UI and Util extensions, once:
#   dotnet tool install --global wix --version 5.0.2
#   wix extension add --global WixToolset.UI.wixext/5.0.2
#   wix extension add --global WixToolset.Util.wixext/5.0.2

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
# An MSI version is numbers only; the pre-release label stays in the file name.
$msiVersion = ($version -split '-')[0]
$msi = "$root\dist\laser-level-webcam-$version-windows-x64.msi"

cargo build --release --manifest-path "$root\Cargo.toml" -p laser-level-webcam -p laser
if ($LASTEXITCODE) { exit $LASTEXITCODE }

New-Item -ItemType Directory -Force "$root\dist" | Out-Null
wix build "$PSScriptRoot\laser-level-webcam.wxs" -arch x64 -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -d "Version=$msiVersion" -d "Root=$root" -pdbtype none -o $msi
if ($LASTEXITCODE) { exit $LASTEXITCODE }
Write-Output "Built $msi"
