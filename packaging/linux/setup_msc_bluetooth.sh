#!/usr/bin/env bash
# SoundCraft — M-Vave SMC-Mixer Bluetooth BLE-MIDI Setup for Linux
# Configures BlueZ, WirePlumber/PipeWire, pairs and connects the M-Vave SMC-Mixer over BLE.

set -euo pipefail

CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${CYAN}======================================================${NC}"
echo -e "${CYAN}   SoundCraft — M-Vave SMC-Mixer Bluetooth BLE Setup  ${NC}"
echo -e "${CYAN}======================================================${NC}"

# 1. Check prerequisites
if ! command -v bluetoothctl &>/dev/null; then
    echo -e "${RED}[ERROR] bluetoothctl not found. Please install bluez: sudo apt install bluez${NC}"
    exit 1
fi

echo -e "\n${YELLOW}[1/5] Checking Bluetooth Controller State...${NC}"
rfkill unblock bluetooth 2>/dev/null || true
bluetoothctl power on >/dev/null

SHOW_INFO=$(bluetoothctl show || true)
if echo "$SHOW_INFO" | grep -q "Powered: yes"; then
    echo -e "${GREEN}[OK] Bluetooth Adapter is Powered ON and Ready.${NC}"
else
    echo -e "${RED}[WARN] Could not power on Bluetooth adapter. Please check hardware/rfkill.${NC}"
fi

# 2. Check PipeWire / WirePlumber BLE-MIDI support
echo -e "\n${YELLOW}[2/5] Checking PipeWire & ALSA Sequencer Subsystems...${NC}"
if pgrep -x "pipewire" >/dev/null; then
    echo -e "${GREEN}[OK] PipeWire is active.${NC}"
fi
if [ -e "/dev/snd/seq" ]; then
    echo -e "${GREEN}[OK] ALSA Sequencer (/dev/snd/seq) is available.${NC}"
fi

# 3. Scanning for M-Vave Hardware
echo -e "\n${YELLOW}[3/5] Scanning for M-Vave SMC-Mixer nearby (5 seconds)...${NC}"
echo -e "${CYAN}Tip: Make sure your SMC-Mixer is turned on with Bluetooth mode enabled (LED blinking).${NC}"

bluetoothctl --timeout 5 scan on >/dev/null 2>&1 || true

DEVICES=$(bluetoothctl devices || true)
echo -e "\nDiscovered Bluetooth Devices:"
echo "$DEVICES"

# Find M-Vave / SMC MAC address
TARGET_MAC=$(echo "$DEVICES" | grep -iE 'smc|m-vave|cuvave|footctrl|mixer' | head -n 1 | awk '{print $2}' || true)
TARGET_NAME=$(echo "$DEVICES" | grep -iE 'smc|m-vave|cuvave|footctrl|mixer' | head -n 1 | cut -d' ' -f3- || true)

if [ -n "$TARGET_MAC" ]; then
    echo -e "\n${GREEN}[FOUND] Detected M-Vave Device: $TARGET_NAME ($TARGET_MAC)${NC}"
    echo -e "\n${YELLOW}[4/5] Pairing, Trusting, and Connecting to $TARGET_MAC...${NC}"

    bluetoothctl pair "$TARGET_MAC" || true
    bluetoothctl trust "$TARGET_MAC" || true
    CONNECT_OUT=$(bluetoothctl connect "$TARGET_MAC" || true)

    if echo "$CONNECT_OUT" | grep -q "Successful"; then
        echo -e "${GREEN}[SUCCESS] Successfully connected to M-Vave SMC-Mixer via Bluetooth!${NC}"
    else
        echo -e "${YELLOW}[NOTE] Connection command output: $CONNECT_OUT${NC}"
    fi
else
    echo -e "\n${YELLOW}[NOTE] No device matching 'SMC-Mixer' or 'M-Vave' was automatically spotted in this 5-second window.${NC}"
    echo -e "If your device appears in the list above with a generic MAC or name, you can connect directly:"
    echo -e "    ${CYAN}bluetoothctl pair <MAC>${NC}"
    echo -e "    ${CYAN}bluetoothctl trust <MAC>${NC}"
    echo -e "    ${CYAN}bluetoothctl connect <MAC>${NC}"
fi

# 4. Check active ALSA MIDI Ports
echo -e "\n${YELLOW}[5/5] Active ALSA MIDI Clients & Ports:${NC}"
aconnect -l || true

echo -e "\n${GREEN}======================================================${NC}"
echo -e "${GREEN} Setup Complete! SoundCraft is configured for MCU.    ${NC}"
echo -e "${GREEN} Launch SoundCraft: cargo run --release -p soundcraft ${NC}"
echo -e "${GREEN}======================================================${NC}"
