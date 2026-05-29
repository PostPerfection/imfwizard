#!/usr/bin/env bash
# Test that the GUI's CLI invocations use flags that actually exist.
# Run from the project root: bash tests/cli_flags_test.sh
#
# This parses main.js for Command.sidecar("imfwizard", [...]) calls,
# extracts the subcommand and flags, then verifies them against --help.

set -euo pipefail

BINARY="${1:-./build/imfwizard}"
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

echo "=== Summary ==="
if [[ $FAILURES -eq 0 ]]; then
  echo "All CLI flags verified successfully."
  exit 0
else
  echo "$FAILURES flag(s) not found in CLI --help output."
  exit 1
fi
