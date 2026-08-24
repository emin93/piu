set -eu

printf 'ready\n'
while IFS= read -r line; do
  if [[ "$line" == "exit" ]]; then
    printf 'bye\n'
    exit 0
  fi
  printf 'echo:%s\n' "$line"
done
