#!/usr/bin/env bash
# Per-user build toolchain for XDM development without root: the GUI's pkg-config/fontconfig
# headers and a MinGW cross-compiler for Windows builds, extracted from distro packages into
# ~/.local/share/xdm-toolchain. Writes env.sh there; source it before cargo commands.
set -euo pipefail
root=${XDM_TOOLCHAIN:-$HOME/.local/share/xdm-toolchain}
mkdir -p "$root/debs" "$root/gui" "$root/cross"
cd "$root/debs"

fetch() { # fetch DEST PACKAGE... — download and extract packages not yet extracted
  local dest=$1; shift
  for package in "$@"; do
    [ -e "$dest/.done-$package" ] && continue
    apt-get download "$package" >/dev/null
    dpkg-deb -x "$(ls -t "${package}"_*.deb | head -1)" "$dest"
    touch "$dest/.done-$package"
  done
}

fetch "$root/gui" pkgconf-bin libpkgconf3 libfontconfig-dev libfontconfig1-dev libfreetype-dev \
  libexpat1-dev zlib1g-dev libpng-dev libbrotli-dev
fetch "$root/cross" binutils-mingw-w64-x86-64 gcc-mingw-w64-base gcc-mingw-w64-x86-64-posix \
  gcc-mingw-w64-x86-64-posix-runtime mingw-w64-common mingw-w64-x86-64-dev

# Debian wires these names through update-alternatives, which extraction skips.
ln -sf pkgconf "$root/gui/usr/bin/pkg-config"
ln -sf x86_64-w64-mingw32-gcc-posix "$root/cross/usr/bin/x86_64-w64-mingw32-gcc"

lib=$root/gui/usr/lib/x86_64-linux-gnu
# Dev packages ship .so symlinks to runtime libraries that only exist in the system
# directory; repoint dangling ones there so the linker does not fall back to static archives.
for link in "$lib"/*.so; do
  [ -L "$link" ] && [ ! -e "$link" ] && ln -sf "/usr/lib/x86_64-linux-gnu/$(readlink "$link")" "$link"
done
cat > "$root/env.sh" <<EOF
export PATH=$root/gui/usr/bin:\$HOME/.cargo/bin:\$PATH
export LIBRARY_PATH=$lib
export PKG_CONFIG_PATH=$lib/pkgconfig
export LD_LIBRARY_PATH=$lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}
# Windows cross builds: cargo build --target x86_64-pc-windows-gnu
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=$root/cross/usr/bin/x86_64-w64-mingw32-gcc
export XDM_CROSS_BIN=$root/cross/usr/bin
EOF
echo "toolchain ready: source $root/env.sh"
