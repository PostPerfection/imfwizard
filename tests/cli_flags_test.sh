#!/usr/bin/env bash
# Test that the GUI's CLI invocations use flags that actually exist.
# Run from the project root: bash tests/cli_flags_test.sh
#
# This parses main.js for Command.sidecar("imfwizard", [...]) calls,
# extracts the subcommand and flags, then verifies them against --help.

set -euo pipefail

BINARY="${1:-./rust/target/release/imfwizard}"
JS_FILE="gui/src/main.js"
FAILURES=0

if [[ ! -x "$BINARY" ]]; then
  echo "ERROR: Binary not found at $BINARY"
  echo "Usage: $0 [path-to-imfwizard-binary]"
  exit 1
fi

if [[ ! -f "$JS_FILE" ]]; then
  echo "ERROR: $JS_FILE not found. Run from project root."
  exit 1
fi

check_flag() {
  local subcmd="$1"
  local flag="$2"
  local help_text

  # Skip positional args (no leading -)
  if [[ ! "$flag" =~ ^- ]]; then
    return 0
  fi

  help_text=$("$BINARY" "$subcmd" --help 2>&1 || true)

  if echo "$help_text" | grep -qF -- "$flag"; then
    return 0
  else
    echo "  FAIL: '$subcmd $flag' not found in '$subcmd --help'"
    FAILURES=$((FAILURES + 1))
    return 1
  fi
}

echo "=== IMFWizard CLI Flag Verification ==="
echo "Binary: $BINARY"
echo ""

# Extract subcommands from args arrays and inline Command calls
JS_SUBCMDS=$(grep -oP 'args\s*=\s*\["([a-z][-a-z]*)"|Command\.(?:sidecar|create)\("imfwizard",\s*\["([a-z][-a-z]*)' "$JS_FILE" \
  | grep -oP '\["[a-z][-a-z]*' | tr -d '["' | sort -u)

for subcmd in $JS_SUBCMDS; do
  # Only check things that are real subcommands
  if ! "$BINARY" "$subcmd" --help &>/dev/null; then
    echo "FAIL: subcommand '$subcmd' does not exist in binary"
    FAILURES=$((FAILURES + 1))
    continue
  fi

  echo "Checking subcommand: $subcmd"

  # Find the line(s) defining the args array for this subcommand and extract flags
  FLAGS=$(grep -P "\[\"$subcmd\"" "$JS_FILE" \
    | grep -oP '"--[a-z][-a-z0-9]*"|"-[a-z]"' \
    | tr -d '"' \
    | sort -u || true)

  for flag in $FLAGS; do
    check_flag "$subcmd" "$flag" || true
  done

  if [[ -z "$FLAGS" ]]; then
    echo "  (no flags found in args)"
  fi
  echo ""
done

# Properties panel controls that stand for a `create` flag. The GUI build path
# calls the library rather than the sidecar, so the loop above never sees these.
# Checking the pair by name catches a flag renamed on one side only, and a
# control left as markup with nothing reading it.
HTML_FILE="gui/index.html"
CREATE_CONTROLS=(
  "prop-audio-delay=--audio-delay"
  "prop-source-colourspace=--source-colourspace"
  "prop-trim-start=--trim-start"
  "prop-trim-end=--trim-end"
  "prop-still-length=--still-length"
  "prop-content-kind=--kind"
  "prop-burn-subtitle=--burn-subtitle"
  "prop-burn-subtitle-font=--burn-subtitle-font"
  "prop-burn-font-size=--burn-font-size"
  "prop-burn-colour=--burn-colour"
  "prop-burn-effect=--burn-effect"
  "prop-burn-effect-colour=--burn-effect-colour"
  "prop-burn-outline-width=--burn-outline-width"
  "prop-burn-line-height=--burn-line-height"
  "prop-burn-margin=--burn-margin"
  "prop-burn-fade-up=--burn-fade-up"
  "prop-burn-fade-down=--burn-fade-down"
  "prop-crop-left=--crop-left"
  "prop-crop-right=--crop-right"
  "prop-crop-top=--crop-top"
  "prop-crop-bottom=--crop-bottom"
  "prop-auto-crop-threshold=--auto-crop-threshold"
  "prop-fill-crop=--fill-crop"
  "prop-deinterlace=--deinterlace"
  "prop-denoise=--denoise"
  "prop-rotate=--rotate"
  "prop-flip=--flip"
  "prop-raster=--raster"
  "prop-audio-map=--audio-map"
)

echo "Checking Properties panel controls against 'create' flags"
for pair in "${CREATE_CONTROLS[@]}"; do
  control="${pair%%=*}"
  flag="${pair#*=}"

  if ! grep -qF "id=\"$control\"" "$HTML_FILE"; then
    echo "  FAIL: no '$control' control in $HTML_FILE for '$flag'"
    FAILURES=$((FAILURES + 1))
    continue
  fi
  if ! grep -qF "$control" "$JS_FILE"; then
    echo "  FAIL: '$control' is markup nothing in $JS_FILE reads"
    FAILURES=$((FAILURES + 1))
    continue
  fi
  check_flag "create" "$flag" || true
done
echo ""

echo "=== Summary ==="
if [[ $FAILURES -eq 0 ]]; then
  echo "All CLI flags verified successfully."
  exit 0
else
  echo "$FAILURES flag(s) not found in CLI --help output."
  exit 1
fi
