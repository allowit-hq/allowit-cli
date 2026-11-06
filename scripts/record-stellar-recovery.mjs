#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const rpc = "https://soroban-testnet.stellar.org";
const receiptArguments = ["request-id", "hash", "source", "recipient", "amount", "journal-file"];
const required = ["origin", "config-dir", "output-dir", ...receiptArguments];
const usage = `node scripts/record-stellar-recovery.mjs ${required.map(name => `--${name} VALUE`).join(" ")} [--allowit PATH]`;

async function record() {
  const { values: args } = parseArgs({
    options: {
      ...Object.fromEntries([...required, "allowit"].map(name => [name, { type: "string" }])),
      help: { type: "boolean" },
    },
  });
  if (args.help) {
    console.log(usage);
    console.log("Read the original request and verify its exact XLM receipt. Use a new output directory for each capture.");
    return;
  }
  if (!required.every(name => args[name])) throw new Error(usage);
  const { origin, "config-dir": configDir, "output-dir": outputDir, "request-id": requestId } = args;
  if (!/^https:\/\/(staging\.allowit\.ai|allowit-backend-7b19f2-[a-z0-9]+-agentools-projects\.vercel\.app)$/.test(origin)) {
    throw new Error("Use the original public staging origin or its Engine Preview");
  }
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(requestId)) throw new Error("Invalid request ID");
  await mkdir(outputDir, { mode: 0o700 });

  console.log(`AllowIt CLI | Stellar Testnet | ${origin}`);
  console.log(`$ allowit --rail stellar:testnet --json status ${requestId} --wait 0s`);
  const env = { ...process.env, ALLOWIT_URL: origin, NO_COLOR: "1" };
  delete env.ALLOWIT_WALLET;
  const lookup = spawnSync(args.allowit ?? "allowit", [
    "--rail", "stellar:testnet", "--rpc-url", rpc, "--config-dir", configDir,
    "--json", "status", requestId, "--wait", "0s",
  ], {
    encoding: "utf8", timeout: 60_000, maxBuffer: 1024 * 1024,
    env,
  });
  if (lookup.error) throw new Error("The CLI could not read the original request status");

  let status;
  try {
    const { requestId: foundId, status: outcome, transactionHash } = JSON.parse(lookup.stdout);
    if (foundId === requestId && /^[a-zA-Z]+$/.test(outcome) && /^[a-f0-9]{64}$/.test(transactionHash)) {
      status = { requestId: foundId, status: outcome, transactionHash };
    }
  } catch {
    // The CLI may return an error on stderr without a request status.
  }
  if (!status) throw new Error(`The CLI returned no usable original status (exit ${lookup.status})`);
  const statusPath = join(outputDir, "status.json");
  await writeFile(statusPath, JSON.stringify(status) + "\n", { mode: 0o600, flag: "wx" });
  console.log(JSON.stringify(status));
  if (lookup.status !== 0 || status.status !== "confirmed") {
    throw new Error("The original request has no confirmed application receipt");
  }

  console.log("$ Verify the original Stellar Testnet transfer against Soroban RPC and Horizon");
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL("./verify-stellar-receipt.mjs", import.meta.url)),
    ...receiptArguments.flatMap(name => [`--${name}`, args[name]]),
    "--status-file", statusPath,
  ], { encoding: "utf8", timeout: 100_000, maxBuffer: 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error(result.stderr?.trim() || "Receipt verification failed");
  const proof = JSON.parse(result.stdout);
  await writeFile(join(outputDir, "verified-receipt.json"), JSON.stringify(proof) + "\n",
    { mode: 0o600, flag: "wx" });
  console.log(JSON.stringify(proof));
  console.log("PASS | Original Stellar Testnet transfer confirmed.");
}

record().catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});
