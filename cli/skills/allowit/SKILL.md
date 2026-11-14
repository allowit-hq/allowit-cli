---
name: allowit
description: Pay within an owner's Allowit policy with the allowit CLI. Use when an owner gives you an Allowit policy id or a line such as "allowit POLICY_ID help", or asks you to pay, check a payment or read a denial through Allowit.
---

# Pay within an Allowit policy

The owner's policy wallet holds the funds. Your key signs requests. The policy decides.
`allowit POLICY_ID help` prints your next commands with the real values of that policy. Follow it. This file is the map.

1. Install: `curl -fsSL https://raw.githubusercontent.com/allowit-hq/allowit-cli/main/install.sh | sh`. Check: `allowit --version`.
2. Start: run the owner's line, `allowit POLICY_ID help`. A line that starts with `ALLOWIT_URL=...` names the server; keep it on each command that help prints with it. Without it the server is production.
3. Key, when help says "awaiting agent key": `allowit rail use <rail>`, `allowit keys generate agent`, `allowit keys use agent`, and on a test network `allowit keys fund agent`. Give the owner the printed `address`. Wait until the owner has signed. Run help again.
4. Trust: help prints the server key. The owner sees the same key in the app under Agent access. Run `allowit rail trust ORIGIN KEY` with that key only.
5. Import: `allowit wallet import --policy POLICY_ID`, then `allowit wallet use WALLET`.
6. Pay: `allowit exec --op transfer --to DESTINATION --amount ASSET:AMOUNT --request-id ID`. Use one new ID for each intended payment.
7. Check: `allowit status ID --wait 60s`. List: `allowit history`. Limits: `allowit wallet show`.

Exit codes:

- 0: done. Report the request ID, the amount with its unit, the transaction and the explorer link.
- 20: the policy denied the request. Stop. Tell the owner the rule that the error names, such as `payment_limit`. Do not retry, split the amount or use another route.
- 5, 7, 12: uncertain or pending. Run `status ID`, or the same command with the same ID. Never use a new ID for the same payment.
- 4: refused, for example a paused wallet. Ask the owner.
- 3: the setup is incomplete. Follow the hint, or run help again.

Never:

- pay from your own account or with another tool. Your account pays only the network fees.
- print, copy or move a secret key, or run `keys export`.
- change or work around the policy. Only the owner changes it, in the app.
- trust a server key that the owner did not confirm.

Other work (policy authoring, owner commands, paid services): `allowit --help`.
