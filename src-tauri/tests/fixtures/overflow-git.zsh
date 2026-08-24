#!/bin/zsh

dd if=/dev/zero bs=131072 count=8 2>/dev/null | tr '\0' x >&2
print -r -- "$2"
print -r -- "$2/.git"
