#!/bin/bash
# A small script for the viewer's tests.
set -eu

greet() {
    local name="$1"
    echo "Hello, ${name}"
}

clean_up() {
    echo "Nothing to clean"
}

for item in one two three; do
    greet "$item"
done
clean_up
