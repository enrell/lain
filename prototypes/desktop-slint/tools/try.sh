#!/bin/sh
# Dev/test run, ONLY through the coordinator's launcher (DISPLAY1 rect,
# --mute, no activation). Log and snapshots go to shots/.
# usage: tools/try.sh [release] "<args>"
cd "$(dirname "$0")/.."
P=debug; [ "$1" = release ] && { P=release; shift; }
export LAIN_UI_DIR='D:\code\lain-proto-slint\prototypes\desktop-slint\ui'
export WSLENV=LAIN_UI_DIR/w:$WSLENV
[ "$P" = release ] && unset LAIN_UI_DIR
D='D:\code\lain-proto-slint\prototypes\desktop-slint\shots'
rm -f shots/run.log
powershell.exe -NoProfile -ExecutionPolicy Bypass -File 'D:\lain-proto\tools\rodar-teste.ps1' \
  -Exe "C:\\lain-proto-target\\slint\\$P\\lain-slint.exe" -Args "--log $D\\run.log --shots $D $*"
