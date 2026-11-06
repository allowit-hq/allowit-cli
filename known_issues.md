# Known issues

## Native exports from older servers

Some servers omit portable administrator source approval from Native public exports.
Those exports cannot prove source consent on another device.
Use `policy export` from an owner workspace with the actual retained source signature, or export again from a server that includes `sourceApproval`.
The SDK verifies that consent and the independent server pin before export. It never treats a completed flag as consent.
An address alone cannot recover reviewed source or its approved parameters.
Policy-wallet imports verify the installed source, parameters, code and roles through independent RPC.
