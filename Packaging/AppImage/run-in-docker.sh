#!/bin/bash
set -euo pipefail

cd /usr/src/vkqr-engine

rm -rf build/appimage
rm -rf build/sdl2

# Rust migration Phase 9 M6: the shipped binary is the mixed build (Rust
# entry point + libquake_rs.a); the C-only configuration stays a CI-only
# comparison oracle in the harness jobs
python3 /opt/meson/meson.py setup build/appimage -Ddebug=true -Dstrip=false -Dmp3_lib=mad -Duse_rust=enabled
ninja -C build/appimage

# Compile check the SDL2 backend (the AppImage ships SDL3)
python3 /opt/meson/meson.py setup build/sdl2 -Ddebug=true -Dstrip=false -Dmp3_lib=mad -Duse_sdl3=disabled -Duse_rust=enabled
ninja -C build/sdl2

cd Packaging/AppImage
rm -rf AppDir
rm -rf vkquake* vkQuake-* vkqr-engine*

# NO_STRIP keeps the debug info in the binary for symbolizing crash
# reports; the sections are not loaded at runtime
NO_STRIP=1 ./linuxdeploy-x86_64.AppImage \
	-e ../../build/appimage/vkqr-engine --appdir=AppDir -d ../../Misc/vkquake.desktop \
	-i ../../Misc/vkQuake_256.png --icon-filename=vkquake --output appimage
