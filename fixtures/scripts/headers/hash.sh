#!/usr/bin/env bash
# diro: name = "Total size of selection"
# diro: description = "Says how many items are selected and how big they are."
# diro: selection = "any"
# diro: selection_file = "nul"
# diro: places = "local"
# diro: timeout = "5m"

# Ordinary comments are passed over.
printf 'announce done\n' >> "${DIRO_DIRECTIVES:?}"
# diro: key = "ctrl+alt+z"
