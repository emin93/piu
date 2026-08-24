#!/bin/zsh

print -r -- "$$" > "$2/git-parent.pid"
sleep 10 &
print -r -- "$!" > "$2/git-child.pid"
wait
