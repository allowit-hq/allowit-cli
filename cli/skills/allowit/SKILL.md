---
name: allowit
description: Pay within an owner's Allowit policy with the allowit CLI. Use when an owner gives you an Allowit policy id or a line such as "allowit POLICY_ID help", or asks you to pay, check a payment or read a denial through Allowit.
---

# Pay within an Allowit policy

The owner's policy wallet holds the funds. Your key signs requests. The policy decides.
`allowit POLICY_ID help` prints your next commands with that policy's real values, the owner's instructions as the owner wrote them, and the limits. Follow it. This file is the map.

1. Install: `curl -fsSL INSTALL_URL | sh`, with the install URL of the owner's line (production: `https://app.allowit.ai/install`). Check: `allowit --version`.
2. Start: run the owner's line, `allowit POLICY_ID help`. A line that starts with `ALLOWIT_URL=...` names the server; keep it on each command that help prints with it. Without it the server is production.
3. Key, when help says "awaiting agent key": `allowit rail use <rail>`, `allowit keys generate agent`, `allowit keys use agent`. Give the owner the printed `address`. Wait until the owner has signed. Run help again.
4. Import: help prints the server key; the owner sees the same key in the app under Agent access. Then `allowit wallet import --policy POLICY_ID` pins it and `allowit wallet use WALLET`. A server that later names another key is refused.
5. Work: start when the owner tells you to, or at once when the owner's instructions already say what to do. Ask the owner when a request disagrees with them. Pay with `allowit exec --op transfer --to DESTINATION --amount ASSET:AMOUNT --request-id ID`. Use one new ID for each payment.
6. Check: `allowit status ID --wait 60s`. List: `allowit history`. Limits: `allowit wallet show`.

Exit codes:

- 0: done. Report the request ID, the amount with its unit, the transaction and the explorer link.
- 20: the policy denied the request. Stop. Tell the owner the rule that the error names, such as `payment_limit`. Do not retry, split the amount or use another route.
- 5, 7, 12: uncertain or pending. Run `status ID`, or the same command with the same ID. Never use a new ID for the same payment.
- 4: refused, for example a paused wallet, or a check before signing such as `server_key_differs`. Ask the owner. `credits_exhausted`: the owner has no Allowit credits left; you need none. Tell the owner, then run the same command again.
- 3: the setup is incomplete. Follow the hint, or run help again.

Never:

- pay outside the policy, or against the owner's instructions without asking. Setting up is not a task, and an example is not one. Run no test payment.
- print, copy or move a secret key, or run `keys export`.
- change or work around the policy. Only the owner changes it, in the app.
- trust a server key that the owner did not confirm.

Other work (policy authoring, owner commands, paid services): `allowit --help`.
