# concordd

A concordance tool.

## Exit codes

| Exit | Meaning |
|---|---|
| 0 | ok |
| 2 | usage |
| 65 | logical |
| 69 | infrastructure |
| 70 | unknown |
| 75 | transient |

## Common codes

Every command may return these.

| Code | Class | Exit |
|---|---|---|
| fix.common.denied | logical | 65 |
| fix.common.busy | transient | 75 |

## Getting started

### Read a concept

Fetch one concept by its id.

Commands: `concept get`

## Commands

### concordd concept get

```text
Show one concept.

Usage: concordd concept get [OPTIONS] --id <id>

Options:
      --id <id>
          The concept id.

      --pretty
          Indent the JSON output.

      --compact
          Print the JSON output on one line.

      --hub <hub>
          The hub address.
          
          [env: OPREG_MANUAL_HUB_UNIQUE]

  -h, --help
          Print help
```

Access: reads, network

Exits:
- 4: records drifted
- 5: busy (on fix.common.busy)

| Code | Class | Exit |
|---|---|---|
| fix.concept.not_found | logical | 65 |
| fix.concept.drift | varies | varies |
| fix.common.busy | transient | 5 |

Also the common codes.

### concordd concept list

```text
List concepts.

Usage: concordd concept list [OPTIONS]

Options:
      --tag <tag>
          Only concepts with this tag.

      --limit <limit>
          The most items to return.

      --pretty
          Indent the JSON output.

      --compact
          Print the JSON output on one line.

      --cursor <cursor>
          Where to resume, from an earlier page's nextCursor.

      --hub <hub>
          The hub address.
          
          [env: OPREG_MANUAL_HUB_UNIQUE]

  -h, --help
          Print help
```

Access: reads

| Code | Class | Exit |
|---|---|---|
| fix.concept.too_many | logical | 65 |

Also the common codes.

## Library codes

| Code | Class | Exit |
|---|---|---|
| op_registry.usage.missing | logical | 2 |
| op_registry.usage.invalid | logical | 2 |
| op_registry.usage.unknown_key | logical | 2 |
| op_registry.usage.repeated_key | logical | 2 |
| op_registry.usage.one_of | logical | 2 |
| op_registry.usage.requires_if | logical | 2 |
| op_registry.usage.conflicts | logical | 2 |
| op_registry.usage.at_least_one | logical | 2 |
| op_registry.usage.stdin_twice | logical | 2 |
| op_registry.cli.unknown_command | logical | 2 |
| op_registry.call.unknown_operation | logical | 65 |
| op_registry.call.wrong_shape | logical | 65 |
| op_registry.call.outside_audience | logical | 65 |
| op_registry.call.wrong_path | logical | 65 |
