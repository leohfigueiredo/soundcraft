#!/usr/bin/env bash
# SoundCraft Desktop Launcher
set -e

# Setup environment for ALSA / PipeWire / UI libraries
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
export PKG_CONFIG_PATH="$HOME/.local/usr/lib/x86_64-linux-gnu/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LIBRARY_PATH="$HOME/.local/usr/lib/x86_64-linux-gnu${LIBRARY_PATH:+:$LIBRARY_PATH}"
export LD_LIBRARY_PATH="$HOME/.local/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

BIN_PATH="/home/leonardo/Documents/Soundcraft/target/release/soundcraft"

if [ -x "$BIN_PATH" ]; then
    cd "/home/leonardo/Documents/Soundcraft"
    exec "$BIN_PATH" "$@"
elif [ -x "$HOME/.local/bin/soundcraft-bin" ]; then
    exec "$HOME/.local/bin/soundcraft-bin" "$@"
else
    cd "/home/leonardo/Documents/Soundcraft"
    exec cargo run --release -p soundcraft -- "$@"
fi
