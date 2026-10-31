#!/usr/bin/env bash
# Build both plugins (GIS and Civiel) and install them into Open CAD Studio's
# user plugins folder. Separate builds: the Civiel plugin links the engine
# without the GIS plugin's entry points. That second build rewrites the GIS
# library in target/release without them, so each plugin is installed right
# after its own build.
set -euo pipefail
cd "$(dirname "$0")/.."

case "$(uname -s)" in
  Darwin) ext=dylib; pre=lib; cfg="$HOME/Library/Application Support" ;;
  MINGW*|MSYS*|CYGWIN*) ext=dll; pre=; cfg="$APPDATA" ;;
  *) ext=so; pre=lib; cfg="${XDG_CONFIG_HOME:-$HOME/.config}" ;;
esac

rustc_version="$(rustc --version)"
acadrust_source="$(awk '/^name = "acadrust"$/{f=1} f&&/^source = /{gsub(/^source = "|"$/,""); print; exit}' Cargo.lock)"

install_plugin() { # <plugin id> <library name> <plugin.toml>
  local dest="$cfg/OpenCADStudio/plugins/$1"
  mkdir -p "$dest"
  rm -f "$dest"/*.so "$dest"/*.dll "$dest"/*.dylib
  cp "target/release/${pre}$2.$ext" "$dest/"
  sed -e "s|__RUSTC_VERSION__|$rustc_version|" -e "s|__ACADRUST_SOURCE__|$acadrust_source|" "$3" > "$dest/plugin.toml"
  echo "Installed $1 in $dest"
}

cargo build --release -p open-infra
install_plugin openaec.open-infra open_infra plugin.toml
cargo build --release -p open-infra-civiel
install_plugin openaec.open-infra-civiel open_infra_civiel civiel/plugin.toml
echo "rustc_version = \"$rustc_version\""
echo "acadrust_source = \"$acadrust_source\""
