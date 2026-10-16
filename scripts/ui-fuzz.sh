#!/bin/bash
# Fuzz da interface: abre um app da suíte num display virtual e faz cliques, arrastos, rolagens
# e atalhos aleatórios. Avisa se o app cair (panic) e salva uma captura de tela no fim.
#
# Uso:   scripts/ui-fuzz.sh [app] [passos] [semente] [-- argumentos do app]
# Ex.:   scripts/ui-fuzz.sh pawcut 500 7 -- video.mp4 musica.mp3
# Precisa de: xvfb, xdotool e imagemagick (`sudo apt install xvfb xdotool imagemagick`),
# e do app compilado (`cargo build -p pawcut-app`).
set -u
APP=${1:-pawcut}
STEPS=${2:-300}
SEED=${3:-1}
shift $(( $# > 3 ? 3 : $# )) 2>/dev/null
[ "${1:-}" = "--" ] && shift
BIN="$(dirname "$0")/../target/${PROFILE:-debug}/$APP"
[ -x "$BIN" ] || { echo "compile antes: cargo build -p ${APP}-app"; exit 1; }
SW=${WIDTH:-1600}; SH=${HEIGHT:-950}
DISP=:${DISPLAY_NUM:-97}
LOG=$(mktemp /tmp/ui-fuzz-XXXX.log)
HOME_DIR=$(mktemp -d /tmp/ui-fuzz-home-XXXX)

Xvfb $DISP -screen 0 ${SW}x${SH}x24 >/dev/null 2>&1 &
XPID=$!
trap 'kill $XPID 2>/dev/null; kill $APID 2>/dev/null' EXIT
sleep 2
export DISPLAY=$DISP HOME=$HOME_DIR
"$BIN" "$@" >"$LOG" 2>&1 &
APID=$!
sleep 8
WIN=$(xdotool search --name "." 2>/dev/null | head -1)
xdotool windowfocus "$WIN" 2>/dev/null

RANDOM=$SEED
KEYS=(space Home End Left Right Up Down i o b a n m Delete BackSpace ctrl+z ctrl+shift+z
  ctrl+b ctrl+c ctrl+v ctrl+x ctrl+a ctrl+d ctrl+s ctrl+n shift+BackSpace l j k d x c v e f g h p q
  r s t u w z y ctrl+minus ctrl+equal ctrl+plus shift+z F9 F10 shift+F12 bracketleft bracketright
  Escape Tab Return)
for i in $(seq 1 "$STEPS"); do
  x=$((RANDOM % SW)); y=$((RANDOM % SH))
  case $((RANDOM % 10)) in
    0|1|2) xdotool key "${KEYS[$((RANDOM % ${#KEYS[@]}))]}" ;;
    3|4) xdotool mousemove $x $y click 1 ;;
    5) xdotool mousemove $x $y click 3; sleep 0.2; xdotool key Escape ;;
    6|7) x2=$((RANDOM % SW)); y2=$((RANDOM % SH))
       xdotool mousemove $x $y mousedown 1; sleep 0.1
       xdotool mousemove $(( (x + x2) / 2 )) $(( (y + y2) / 2 )); sleep 0.05
       xdotool mousemove $x2 $y2 mouseup 1 ;;
    8) xdotool mousemove $x $y click 5 ;;
    9) xdotool mousemove $x $y click 4 ;;
  esac
  sleep 0.15
  if ! kill -0 $APID 2>/dev/null; then
    echo "CAIU no passo $i (semente $SEED). Log: $LOG"
    grep -n "panicked" -A4 "$LOG" | head -12
    exit 1
  fi
done
import -window root "/tmp/${APP}-fuzz.png" 2>/dev/null && echo "captura: /tmp/${APP}-fuzz.png"
echo "ok: $STEPS passos sem cair (semente $SEED)"
