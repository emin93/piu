#!/usr/bin/env node

import { provisionAgentRuntime } from "./agent-runtime-bundle.mjs";

const result = await provisionAgentRuntime();
console.log(
  `Provisioned Node ${result.nodeVersion} and Pi ${result.piVersion} at ${result.packageRoot}`,
);
