import { isAbsolute } from "node:path";

const [emptyGlobalRoot, separator, command, flag] = process.argv.slice(2);

if (
  !emptyGlobalRoot ||
  !isAbsolute(emptyGlobalRoot) ||
  separator !== "--" ||
  command !== "root" ||
  flag !== "-g"
) {
  process.exitCode = 64;
} else {
  process.stdout.write(emptyGlobalRoot);
}
