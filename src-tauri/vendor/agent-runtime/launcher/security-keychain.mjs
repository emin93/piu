import { spawn } from "node:child_process";

const SECURITY_EXECUTABLE = "/usr/bin/security";
const MISSING_ITEM_EXIT_CODE = 44;
const MAX_COMMAND_OUTPUT_BYTES = 1024 * 1024;
const PROVIDER_ID = /^[a-z0-9][a-z0-9._-]*$/;

export class KeychainCommandError extends Error {
  constructor(code = "keychainUnavailable") {
    super("macOS Keychain is unavailable");
    this.name = "KeychainCommandError";
    this.code = code;
  }
}

export function runSecurityCommand({ args, executable = SECURITY_EXECUTABLE, input, signal }) {
  signal?.throwIfAborted();
  return new Promise((resolve, reject) => {
    let settled = false;
    let stdoutBytes = 0;
    let stderrBytes = 0;
    const stdout = [];
    const child = spawn(executable, args, {
      env: {},
      signal,
      stdio: ["pipe", "pipe", "pipe"],
    });
    const fail = () => {
      if (settled) return;
      settled = true;
      child.kill("SIGKILL");
      reject(new KeychainCommandError());
    };

    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length;
      if (stdoutBytes > MAX_COMMAND_OUTPUT_BYTES) {
        fail();
        return;
      }
      stdout.push(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes > MAX_COMMAND_OUTPUT_BYTES) fail();
    });
    child.on("error", () => {
      if (!signal?.aborted) {
        fail();
        return;
      }
      if (settled) return;
      settled = true;
      reject(signal.reason);
    });
    child.on("close", (code) => {
      if (settled) return;
      settled = true;
      resolve({ code, stdout: Buffer.concat(stdout).toString("utf8") });
    });
    child.stdin.on("error", fail);
    child.stdin.end(input);
  });
}

function accountArguments(providerId, service) {
  if (!PROVIDER_ID.test(providerId)) throw new KeychainCommandError("invalidProvider");
  return ["-a", providerId, "-s", service];
}

function requireSuccess(result) {
  if (result.code !== 0) throw new KeychainCommandError();
}

export function createSecurityKeychain({
  runCommand = runSecurityCommand,
  service = "ch.emin.piu.pi-credentials",
} = {}) {
  return {
    async delete(providerId, options = {}) {
      const result = await runCommand({
        executable: SECURITY_EXECUTABLE,
        args: ["delete-generic-password", ...accountArguments(providerId, service)],
        signal: options.signal,
      });
      if (result.code !== 0 && result.code !== MISSING_ITEM_EXIT_CODE) {
        throw new KeychainCommandError();
      }
    },

    async exists(providerId, options = {}) {
      const result = await runCommand({
        executable: SECURITY_EXECUTABLE,
        args: ["find-generic-password", ...accountArguments(providerId, service)],
        signal: options.signal,
      });
      if (result.code === MISSING_ITEM_EXIT_CODE) return false;
      requireSuccess(result);
      return true;
    },

    async read(providerId, options = {}) {
      const result = await runCommand({
        executable: SECURITY_EXECUTABLE,
        args: ["find-generic-password", ...accountArguments(providerId, service), "-w"],
        signal: options.signal,
      });
      if (result.code === MISSING_ITEM_EXIT_CODE) return undefined;
      requireSuccess(result);
      try {
        return JSON.parse(result.stdout.replace(/\n$/, ""));
      } catch {
        throw new KeychainCommandError("invalidCredential");
      }
    },

    async write(providerId, credential, options = {}) {
      const serialized = JSON.stringify(credential);
      const result = await runCommand({
        executable: SECURITY_EXECUTABLE,
        args: ["add-generic-password", ...accountArguments(providerId, service), "-U", "-w"],
        input: `${serialized}\n${serialized}\n`,
        signal: options.signal,
      });
      requireSuccess(result);
    },
  };
}
