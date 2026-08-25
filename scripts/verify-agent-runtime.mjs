#!/usr/bin/env node

import { verifyAgentRuntime } from "./agent-runtime-bundle.mjs";

const result = await verifyAgentRuntime();
console.log(
  JSON.stringify(
    {
      nodeExecutable: result.nodeExecutable,
      nodeVersion: result.nodeVersion,
      packageRoot: result.packageRoot,
      piCommit: result.piCommit,
      piVersion: result.piVersion,
    },
    null,
    2,
  ),
);
