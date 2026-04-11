# ralph

`ralph` is a CLI loop that runs Claude Code against a PRD (`prd.jsonc`) until the agent reports completion.

Each iteration Ralph:

- builds a system prompt from the PRD
- invokes `claude`
- analyzes output for completion, rate limits, and stuck-loop patterns
- writes a per-iteration log to `.ralph/logs/`

Git is optional, but recommended. In Git repos Ralph validates that PRD edits only change feature `status` fields.

## Install

### AUR (Arch Linux)

```bash
paru -S ralph
```

### From source

```bash
cargo install --path .
```

Requirements:

- [`claude`](https://github.com/anthropics/claude-code) must be in `PATH`
- Git is recommended if you want PRD diff validation and init-phase git context

## Quick Start

```bash
cd <project>
ralph --init       # creates prd.jsonc
$EDITOR prd.jsonc  # edit the generated template
ralph --dry-run    # validate PRD and run verification commands
ralph              # start the loop
```

## Files

These files live next to the PRD. If you use `--prd path/to/custom.jsonc`, Ralph uses that directory instead of the current one.

| Path | Purpose |
|------|---------|
| `prd.jsonc` | Task definition and verification commands |
| `progress.txt` | Append-only progress log used by the built-in prompt |
| `.ralph/logs/` | One log file per Claude iteration |

## Workflow

1. Load the PRD and resolve the effective completion marker.
2. Create `progress.txt` and `.ralph/logs/` if needed.
3. Run the optional init phase: git status, PRD summary, progress summary, recent commits.
4. Invoke `claude` with the built-in prompt or a custom prompt.
5. Print the resolved active prompt once at launch so you can inspect the exact instructions being sent.
6. Classify the result as `continue`, `complete`, `rate-limit`, `loop-detected`, `failed`, or `cancelled`.
7. In Git repos, validate that PRD changes only touched `status`.
8. Stop on completion marker, interruption, max iterations, or repeated failures.

Completion is marker-based. By default Ralph finishes when Claude outputs `<promise>COMPLETE</promise>`. Override it with `--completion-marker`.

The built-in prompt tells the agent to append to `progress.txt` and commit its work. Ralph does not enforce either action itself.

## PRD Format

```jsonc
{
  "project": {
    "name": "my-project",
    "description": "What this project does",
    "repository": "https://github.com/example/my-project"
  },
  "verification": {
    "commands": [
      {
        "name": "check",
        "command": "cargo check",
        "description": "Compile / type-check"
      },
      {
        "name": "test",
        "command": "cargo test",
        "description": "Run the test suite"
      }
    ],
    "runAfterEachFeature": true
  },
  "features": [
    {
      "id": "feature-id",
      "category": "functional",
      "description": "What needs to be done",
      "steps": ["Step 1", "Step 2"],
      "status": "pending",
      "notes": "Optional context"
    }
  ],
  "completion": {
    "allFeaturesComplete": true,
    "allVerificationsPassing": true
  }
}
```

Notes:

- `status` must be one of `pending`, `in-progress`, `complete`, or `blocked`.
- `category` is free-form. Common values are `functional`, `bugfix`, `refactor`, `test`, and `docs`.
- `project.repository` and `features[].notes` are optional.
- `verification.commands[].description` is required.
- `runAfterEachFeature` only changes the instructions Ralph gives the agent. Ralph itself runs verification commands in `--dry-run`; during normal runs the agent is instructed to run them.
- `completion` is part of the required schema, but the Rust loop stops on the completion marker in Claude output, not on those boolean fields.
- Use `--completion-marker` to override the built-in marker. Ralph no longer reads a completion marker from the PRD.

## Options

```text
-p, --prd <PATH>                  PRD file path [default: prd.jsonc]
-P, --prompt <PATH>               Custom prompt file
-m, --max-iterations <N>          Maximum iterations, 0 = unlimited [default: 10]
-d, --delay <SECONDS>             Delay between iterations [default: 2]
-c, --completion-marker <TEXT>    Override the built-in completion marker
--permission-mode <MODE>          Passed to `claude --permission-mode` [default: acceptEdits]
--continue-session                Use `claude --continue` instead of `--print`
--dangerously-skip-permissions    Pass through to Claude to auto-approve actions
--skip-init                       Skip the initialization phase
--init                            Create a PRD template and exit
--init-prompt                     Create `prompt.md` template and exit
--dry-run                         Validate PRD, run verification commands, exit
--webhook <URL>                   Send best-effort session lifecycle POSTs
--max-iteration-errors <N>        Auto-block the current feature after N iteration errors (0 = disabled)
--rate-limit-fallback-seconds <N> Fallback cooldown when no reset time is parsed [default: 60]
--rate-limit-buffer-seconds <N>   Safety buffer after parsed reset times [default: 60]
--rate-limit-post-reset-max-backoff-seconds <N>
                                  Cap post-reset backoff [default: 1800]
--rate-limit-post-reset-max-retries <N>
                                  Abort after N post-reset retries [default: 4]
--exit-on-rate-limit              Exit immediately instead of waiting and retrying
-t, --timeout <SECONDS>           Timeout per Claude execution [default: 1800]
```

## Custom Prompts

Ralph has a built-in prompt. A custom prompt fully replaces it after placeholder substitution.

```bash
ralph --init-prompt   # creates prompt.md in the current directory
ralph --prompt prompt.md
```

If your custom prompt omits rules about PRD edits, verification cadence, progress logging, or completion, Ralph will not add them back.

Supported placeholders:

| Placeholder | Description |
|-------------|-------------|
| `{prd_path}` | Path to the PRD file |
| `{progress_path}` | Path to the progress file |
| `{verification_commands}` | Formatted verification command list |
| `{completion_marker}` | Effective completion marker |
| `{verification_rule}` | Verification policy sentence derived from `runAfterEachFeature` |
| `{verification_workflow}` | Workflow sentence for verification timing |
| `{completion_workflow}` | Workflow sentence for when to mark a feature complete |

## Webhooks

Send best-effort HTTP `POST` notifications when a session starts, completes, or fails:

```bash
ralph --webhook https://example.com/webhook
```

Events:

| Event | Trigger |
|-------|---------|
| `session_start` | Session begins |
| `session_complete` | Completion marker detected |
| `session_failed` | Session exits due to failure conditions |

Payload:

```json
{
  "event": "session_start",
  "timestamp": "2024-01-15T10:30:00Z",
  "message": "Starting session for my-project"
}
```

| Field | Description |
|-------|-------------|
| `event` | Event type string |
| `timestamp` | RFC3339 timestamp |
| `message` | Human-readable description |

Webhook failures are logged but do not fail the session.

## Operational Notes

- PRD validation is Git-dependent. Ralph validates `prd.jsonc` with `git diff HEAD -- <prd>`; outside a Git repo that validation is skipped.
- Ralph aborts after 3 consecutive failed iterations.
- `--max-iteration-errors` can auto-block the current in-progress feature after repeated iteration errors.
- Rate-limit handling parses reset times, waits with a safety buffer, and uses post-reset backoff unless `--exit-on-rate-limit` is set.
- First `Ctrl+C` requests graceful shutdown and Ralph exits with code `130` after cleanup. Second `Ctrl+C` forces immediate exit.
- On Unix, Ralph manages Claude in a separate process group for cleanup. On non-Unix platforms it terminates the direct child process.

## License

MIT
