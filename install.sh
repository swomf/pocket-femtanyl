#!/usr/bin/env sh
if [ "$(id -u)" = 0 ]; then
  echo '[!] dont run as root' >&2
  exit 1
fi

ask() {
  printf ":: run '%s'? [y/N] " "$*"
  read -r yn
  case "$yn" in
  [Yy] | [Yy][Ee][Ss]) "$@" ;;
  esac
}

export PROG="pocket-femtanyl"
export PREFIX="${PREFIX:-/usr/local}"

ask cargo build --release
ask sudo install -Dm755 "./target/release/$PROG" "$PREFIX/bin/$PROG"
#ask install -Dm755 com.swomf.pocket_femtanyl.desktop "$PREFIX/share/applications/com.swomf.pocket_femtanyl.desktop"
ask install -Dm644 pocket-femtanyl.lua ~/.config/hypr/pocket-femtanyl.lua
echo '!!! THIS NEXT COMMAND IS NOT RECOMMENDED !!!'
echo '!!! idk what your setup looks like. so DO IT YOURSELF !!!'
ask sh -c "echo require\(\'pocket-femtanyl\'\) | tee -a ~/.config/hypr/hyprland.lua"
