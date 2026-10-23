#!/usr/bin/env bash
# diro: name = "Total size of selection"
# diro: description = "Says how many items are selected and how big they are."
# diro: selection = "any"
# diro: selection_file = "nul"
# diro: places = "local"
# diro: timeout = "5m"
#
# A script command for diroxide, in bash. The diro: lines above are its
# header, one key a line:
#   name            the command's name in menus and the palette.
#   description     one sentence for F1 help.
#   selection       any: the marked items, or the cursor's item when
#                   nothing is marked.
#   selection_file  nul: it reads the NUL-separated list, which holds
#                   any name, line breaks included.
#   places          local: it works on local folders only.
#   timeout         it is stopped after five minutes.
#
# Try it:
#   diro plugin check total-size             checks the header and the file
#   diro plugin run total-size -- FILE...    runs it once on FILE, after one question
#   diro plugin enable total-size            puts it in the Plugins menu
#
# Rules it keeps, and yours should too:
#   - Read lists from the NUL file with a read loop, never by looping
#     over ls or through xargs.
#   - Put -- before names, so a name that starts with a dash is not read
#     as an option.
#   - Remove nothing with the shell. A protocol plugin's fs/delete and
#     diro's own commands list first and use the trash.
#   - Put scratch files in DIRO_TEMP, which diroxide removes afterwards.
#   - Write every variable in a path as ${VAR:?}, so an empty one stops
#     the script instead of naming the wrong folder.
#   - No set -e: a file du cannot read is counted and said, not fatal.
#   - Write each directive with one printf, so lines never interleave.
#
# Exit codes: 0 done; 1 failed, nothing changed; 2 wrong use; 3 partly
# done, the output says which.

directives="${DIRO_DIRECTIVES:?}"
all="${DIRO_SELECTION_COUNT:-0}"
count=0
skipped=0
total_kb=0

while IFS= read -r -d '' name; do
  count=$((count + 1))
  if size=$(du -sk -- "$name" 2>/dev/null); then
    total_kb=$((total_kb + ${size%%[!0-9]*}))
  else
    skipped=$((skipped + 1))
  fi
  if [ "$all" -gt 50 ] && [ $((count % 50)) -eq 0 ]; then
    printf 'progress %d of %d\n' "$count" "$all" >> "$directives"
  fi
done < "${DIRO_SELECTION_FILE0:?}"

# The size in words, with whole-number arithmetic only: du counts KiB.
bytes=$((total_kb * 1024))
if [ "$bytes" -lt 1000 ]; then
  size="$bytes bytes"
elif [ "$bytes" -lt 1000000 ]; then
  size="$((bytes / 1000)).$((bytes % 1000 / 100)) KB"
elif [ "$bytes" -lt 1000000000 ]; then
  size="$((bytes / 1000000)).$((bytes % 1000000 / 100000)) MB"
else
  size="$((bytes / 1000000000)).$((bytes % 1000000000 / 100000000)) GB"
fi

if [ "$skipped" -gt 0 ]; then
  printf 'announce Selected: %d, size: %s\nwarn Skipped %d items it could not read\n' \
    "$count" "$size" "$skipped" >> "$directives"
  exit 3
fi
printf 'announce Selected: %d, size: %s\n' "$count" "$size" >> "$directives"
exit 0
