import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

import {
  loadRuntimeLock,
  validateRuntimeLock,
  verifyAgentRuntime,
} from "./agent-runtime-bundle.mjs";

const expected = {
  nodeArchiveSha256: "8294b7aa9b03997481c06babf1e8b270c859358f27da57a11509afe537ac381d",
  nodeVersion: "24.19.0",
  piAiIntegrity:
    "sha512-M0YUV8vNO3y2WwWSyY8ijKJV5W4gkSUixuvk+Z00ZBjsyMfsdXfITsHEwP1UIf09YRWXT6oGn0GlCamt+P32XQ==",
  piCodingAgentIntegrity:
    "sha512-Yr2p9PubrbFZmYEPYI+C8KmZP9xlFuLDnAG64RtU0ZDgrdiXYWa+y7WGyJO5OlqPliOkVCMd9IzVszO3/t0D0w==",
  piCommit: "4e58f324fae8ebfa98a3d45181fb248072a2afac",
  piVersion: "0.84.3",
  properLockfileIntegrity:
    "sha512-TjNPblN4BwAWMXU8s9AEz4JmQxnD1NNL7bNOY/AKUzyamc379FWASUhc/K1pL2noVb+XmZKLL68cjzLsiOAMaA==",
};

test("runtime lock fixes the supported Node and Pi release", async () => {
  const lock = await loadRuntimeLock();
  const packageJson = JSON.parse(
    await readFile(resolve("src-tauri/vendor/agent-runtime/package.json"), "utf8"),
  );
  const packageLock = JSON.parse(
    await readFile(resolve("src-tauri/vendor/agent-runtime/package-lock.json"), "utf8"),
  );

  assert.equal(lock.target, "darwin-arm64");
  assert.equal(lock.node.version, expected.nodeVersion);
  assert.equal(lock.node.archiveSha256, expected.nodeArchiveSha256);
  assert.equal(lock.pi.commit, expected.piCommit);
  assert.equal(lock.pi.packages["@earendil-works/pi-coding-agent"].version, expected.piVersion);
  assert.equal(
    lock.pi.packages["@earendil-works/pi-coding-agent"].integrity,
    expected.piCodingAgentIntegrity,
  );
  assert.equal(lock.pi.packages["@earendil-works/pi-ai"].version, expected.piVersion);
  assert.equal(lock.pi.packages["@earendil-works/pi-ai"].integrity, expected.piAiIntegrity);
  assert.equal(lock.pi.packages["proper-lockfile"].version, "4.1.2");
  assert.equal(lock.pi.packages["proper-lockfile"].integrity, expected.properLockfileIntegrity);
  assert.deepEqual(packageJson.dependencies, {
    "@earendil-works/pi-ai": expected.piVersion,
    "@earendil-works/pi-coding-agent": expected.piVersion,
    "proper-lockfile": "4.1.2",
  });
  assert.equal(packageLock.lockfileVersion, 3);
  assert.equal(
    packageLock.packages["node_modules/@earendil-works/pi-coding-agent"].integrity,
    expected.piCodingAgentIntegrity,
  );
  assert.equal(
    packageLock.packages["node_modules/@earendil-works/pi-ai"].integrity,
    expected.piAiIntegrity,
  );
  assert.equal(
    packageLock.packages["node_modules/proper-lockfile"].integrity,
    expected.properLockfileIntegrity,
  );
  assert.doesNotThrow(() => validateRuntimeLock(lock));
});

test("runtime lock rejects a floating or altered Pi package", async () => {
  const lock = structuredClone(await loadRuntimeLock());
  lock.pi.packages["@earendil-works/pi-coding-agent"].version = "latest";

  assert.throws(() => validateRuntimeLock(lock), /pinned Pi coding-agent version/);
});

test("runtime verification fails closed when bundled Node is absent", async () => {
  const runtimeRoot = await mkdtemp(join(tmpdir(), "piu-agent-runtime-test-"));

  try {
    await assert.rejects(verifyAgentRuntime({ runtimeRoot }), /Bundled Node executable is missing/);
  } finally {
    await rm(runtimeRoot, { force: true, recursive: true });
  }
});

test("runtime verification rejects a non-arm64 Node executable", async () => {
  const runtimeRoot = await mkdtemp(join(tmpdir(), "piu-agent-runtime-test-"));
  const nodeExecutable = join(runtimeRoot, "node", "bin", "node");

  try {
    await mkdir(join(runtimeRoot, "node", "bin"), { recursive: true });
    await writeFile(nodeExecutable, "#!/bin/sh\nexit 0\n");
    await chmod(nodeExecutable, 0o755);

    await assert.rejects(verifyAgentRuntime({ runtimeRoot }), /not an arm64 Mach-O executable/);
  } finally {
    await rm(runtimeRoot, { force: true, recursive: true });
  }
});

test("Tauri maps only the fixed agent-runtime resource directory", async () => {
  const config = JSON.parse(await readFile(resolve("src-tauri/tauri.conf.json"), "utf8"));

  assert.equal(config.bundle.resources["vendor/agent-runtime/runtime/"], "agent-runtime/");
});
