# Glossary: shared terms for CLI and MCP tools

A reference, not a rule. Nothing here is banned or required. When you name a flag,
JSON key, error area or help line, read this first and use the shared term if it
means what you mean. A product with its own idea defines it in its own repo's
glossary and qualifies it in help; it does not add it here.

Flags are kebab-case and JSON keys camelCase (spec, gp B). The error area is the
`<pillar>.<area>.*` segment. The subject of a command is `--id` (gp A); the flags
below name the *other* objects in a call.

Letters in "not to be confused with": G grorg, M multimatrix, S structmux,
R archegraph, gait by name.

Decided by gp (2026-11-20): grorg's and multimatrix's "machine" record is a
**place**; a structmuxd is a **daemon**; "computer" is only a real computer;
an address is `--addr` (dial) or `--listen` (bind), never `--host`; grorg keeps
**agent** for any acting identity.

| term | means | cardinality, relations | flag / JSON / error area | not to be confused with |
|---|---|---|---|---|
| **computer** | A physical or virtual box with an OS. Never a record; named by hostname. | Holds 0..n places and 0..n daemons; 1 computer identity dir. | `--computer` / `computer` / none (rarely a param) | "machine" in `MACHINE_IDENTITY_DIR` (this concept, old word); S "machine" label (a daemon); G/M `Machine` (a place) |
| **place** | One registered deployment unit, a container or a native install, with its own address; where environments and agents run. | On 1 computer; 1 address; 0..1 daemon; 0..n environments. An agent has 0..1 home place. | `--place` / `placeId` / `place` | G/M `Machine`/`machine_id` (this concept, old word); a computer |
| **daemon** | A resident service process with its own principal that clients dial (structmuxd is the main one). Qualify by product when ambiguous: "structmux daemon". | 0..1 per place, so 0..n per computer. | `--daemon` / `daemonId` / `daemon` | gait `daemon` (a tracked background process, gait-local); "machine" in S list output; M "worker" |
| **address** | The host:port a daemon or place listens on and is dialled at. | 1 per place; 1..n per daemon. | `--addr` to dial, `--listen` to bind / `address` / – | server endpoint; "host" (not used; say address) |
| **server endpoint** | The URL of the service API this CLI or MCP client calls. | 1 per client config. | `--endpoint` / `endpoint` / – | a place's address |
| **fleet** | Every daemon and place under one owner. | 1 per owner. | – / – / `fleet` | – |
| **principal** | A `p256:<hex>` genesis-key identity of anyone who signs: person, agent, daemon or service. | Many; 1 kind each. | `--principal` / `principalId` / `identity` | an agent id (an agent *has* a principal) |
| **agent** | An acting identity with its own principal that can be given work. | Many; id global, name unique per scope. | `--agent` / `agentId` / `agent` | G `Agent` also covers humans and services; gait agent profile; R `agent:` command |
| **agent config** | The named definition an agent is started from: command, args, env, role text. | Several per daemon config, gait dir or project. | `--agent-config` / `agentConfig` / `agent` | the agent identity |
| **instance** | One thing a daemon runs (terminal, agent, document); an agent instance is one running agent. | Many per daemon. | `--instance` / `instanceId` / `instance` | G federation instance (a grorg server); telemetry instance id |
| **standing agent** | An agent a daemon starts by itself from the owner-signed startup list; its opposite is a **one-off**. | Several per daemon. | `--standing` switch / `standing` / – | – |
| **owner** | The one human root principal of a fleet. | Exactly 1. | `--owner-principal` / `ownerPrincipal` / `owner` | S owner document and tool; G assignment role "owner"; M owner listener |
| **person** | A human principal. | Several. | `--person` / `personId` / `person` | – |
| **service** | A program principal (multimatrix, empeiria). | Several. | `--service` / `serviceId` / `service` | a systemd/compose service |
| **device** | One key pair on one install that signs for a principal. | Several per principal. | `--device` / `deviceId` / `device` | a computer; gait's "device code" login |
| **root key** | A principal's long-lived key; devices are delegated from it. | 1 live per principal. | `--root-key-file` / – / `identity` | R project `--root`; S limits `--root`; S `--root-file` |
| **identity dir** | A directory holding one principal's key files. | 1 per principal per computer. | `--identity-dir` / `identityDir` / `identity` | computer identity dir; G `identity_uri` |
| **computer identity dir** | The per-computer key store shared by every client on it. | 1 per computer. | env `MACHINE_IDENTITY_DIR` kept / – / `identity` | identity dir |
| **action** | A unit of assigned work in grorg, any size. Scripts say "job" for it. | Many; tree plus waits-for DAG. | `--action` / `actionId` / `action` | S `Action` (a semantic-node verb); R `*Action` enums |
| **dispatch** | Handing a ready action to its assignee's agent. | Per action. | – / – / `dispatch` | – |
| **turn** | One prompt to an agent through its final reply. | Many per session. | `--turn` / `turn` / `turn` | gait "run" (same thing, old word) |
| **attempt** | One try of a retried unit. | Several per unit, 1 current. | `--attempt` / `attempt` / – | – |
| **workflow** | A named DAG of steps a runner executes; a **workflow run** is one execution. | Several. | `--workflow` / `workflow` / `workflow` | a turn |
| **model** | An LLM model id. | Many per provider. | `--model` / `modelId` / `model` | – |
| **provider** | An LLM vendor namespace. | ~8. | `--provider` / `providerId` / `provider` | G `provider_id` row id vs key (G-local fix) |
| **codebase** | The source tree an agent edits or a tool analyses. | 1 per call. | `--codebase` / `codebase` / – | gait dir; R project subdirectory |
| **worktree** | A git worktree an agent edits in. | Per session or per build node. | `--worktree` / `worktree` / `worktree` | – |


Words with **no shared meaning**: session, node, run, request, status, target, root, worker,
host, job, task, machine. A product that uses one defines it in its own glossary and qualifies
it in help. For "machine", prefer computer or place; for "host", say address.
