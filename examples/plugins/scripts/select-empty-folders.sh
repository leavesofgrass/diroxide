#!/usr/bin/env bash
# diro: name = "Select empty folders"
# diro: description = "Marks the empty folders of the active panel."
# diro: selection = "none"
# diro: places = "local"
# diro: timeout = "1m"
#
# A script command for diroxide, in bash. It marks the empty folders
# directly inside the active panel's folder, hidden ones included.
#
# It shows a directive that waits: the select lines take effect when
# the script ends, so the panel never changes while you listen. Clear
# the marks with the usual key if you change your mind; it changes
# nothing else, so it has no preview.
#
# Try it:
#   diro plugin check select-empty-folders
#   diro plugin run select-empty-folders
#
# Exit codes: 0 done; 1 failed, nothing changed; 2 wrong use.

directives="${DIRO_DIRECTIVES:?}"
found="${DIRO_TEMP:?}/empty-folders"
newline='
'

# find writes each empty folder ended by a NUL into a file; the loop
# reads that file through a redirect, never through a pipe.
if ! find "${DIRO_ACTIVE:?}" -mindepth 1 -maxdepth 1 -type d -empty -print0 > "$found"; then
  printf 'error Could not look through this folder\n' >> "$directives"
  exit 1
fi

count=0
skipped=0
while IFS= read -r -d '' folder; do
  case "$folder" in
    *"$newline"*)
      # A directive is one line, so a name with a line break cannot be
      # marked from a script.
      skipped=$((skipped + 1))
      continue
      ;;
  esac
  printf 'select %s\n' "$folder" >> "$directives"
  count=$((count + 1))
done < "$found"

if [ "$skipped" -gt 0 ]; then
  printf 'warn Left out %d folders whose names have a line break\n' "$skipped" >> "$directives"
fi
case "$count" in
  0) printf 'announce No empty folders here\n' >> "$directives" ;;
  1) printf 'announce 1 empty folder selected\n' >> "$directives" ;;
  *) printf 'announce %d empty folders selected\n' "$count" >> "$directives" ;;
esac
exit 0
