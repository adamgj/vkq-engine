$VersionMajorMatch=Select-String "^#define\s*VKQUAKE_VERSION_MAJOR\s*([0-9.]*)" "../../Quake/quakever.h"
$VersionMajor=$VersionMajorMatch.Matches.groups[1].value
$VersionMinorMatch=Select-String "^#define\s*VKQUAKE_VERSION_MINOR\s*([0-9.]*)" "../../Quake/quakever.h"
$VersionMinor=$VersionMinorMatch.Matches.groups[1].value
$PatchVersionMatch=Select-String "^#define\s*VKQUAKE_VER_PATCH\s*([0-9.]*)" "../../Quake/quakever.h"
$PatchVersion=$PatchVersionMatch.Matches.groups[1].value
$SuffixMatch=Select-String "^#define\s*VKQUAKE_VER_SUFFIX\s*`"([^`"]*)" "../../Quake/quakever.h"
$Suffix=$SuffixMatch.Matches.groups[1].value
$Version=$VersionMajor + '.' + $VersionMinor + '.' + $PatchVersion + $Suffix
$SrcDirX64="..\..\builddir-package"

# Cleanup
Del "vkqr-engine-*.exe"
Del "vkqr-engine-*.zip"

# Clean & build binaries (meson + clang-cl; run from a shell with the MSVC environment loaded)
$env:CC = 'clang-cl'
meson setup ..\..\builddir-package --buildtype=release -Ddebug=true -Dstrip=false -Duse_sdl3=enabled --wipe
meson compile -C ..\..\builddir-package

# Create NSIS exe installers
$Nsis="C:\Program Files (x86)\NSIS\Bin\makensis.exe"
$NsisArguments = '-V4 -DSRCDIR="' + $SrcDirX64 + '" -DPLATFORM=windows_x64 -DVERSION=' + $Version + ' vkqr-engine.nsi'
Start-Process -Wait -NoNewWindow -PassThru -FilePath $Nsis -ArgumentList $NsisArguments

# Create zip files
$compress = @{
  Path = "$SrcDirX64\*.exe", "$SrcDirX64\vkqr-engine.pdb", "$SrcDirX64\*.dll", "..\..\LICENSE.txt"
  CompressionLevel = "Optimal"
  DestinationPath = "vkqr-engine-" + $Version + "_windows_x64.zip"
}
Compress-Archive @compress
