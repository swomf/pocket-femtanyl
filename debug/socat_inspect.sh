#!/usr/bin/env sh
socat -u UNIX-CONNECT:"$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock" - |
  rg ':.*:'

# custom>>pocket-femtanyl
