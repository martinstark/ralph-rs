# ralph

Autonomous AI agent loop for Claude Code CLI. Iteratively works through features defined in a PRD until completion.

Implemented in Rust, based on a ralph shell script that I've used to educate teams and deploy code to production.

## Install

### AUR (Arch Linux)

```bash
paru -S ralph
```

### From source

```bash
cargo install --path .
```

Requires [Claude CLI](https://github.com/anthropics/claude-code) in PATH.

## Quick Start

```bash
cd <project>
ralph --init          # Create template prd.jsonc
# Edit prd.jsonc with your features
ralph                 # Run the loop
```

## Usage

1. **Generate template** — In your project directory, run:
   ```bash
   ralph --init
   ```
   This creates a `prd.jsonc` file with the basic structure.

2. **Populate the PRD** — The template needs to be filled with features for ralph to process. Start a Claude session and ask it to break down your task:
   ```bash
   claude
   ```
   Then prompt:
   ```
   Evaluate [YOUR TASK HERE] and break it down into implementation steps.
   Output the result in prd.jsonc format with features array containing
   id, category, description, steps, and status fields.
   ```

3. **Copy the output** — Replace the template content in `prd.jsonc` with Claude's structured breakdown.

4. **Run the loop** — Exit Claude and start ralph:
   ```bash
   ralph
   ```
   Ralph will iterate through each feature, spawning Claude sessions to implement them one by one until all are complete.

## How It Works

1. **Initialize** — Validates PRD, checks git status, shows feature summary
2. **Loop** — For each iteration:
   - Spawns Claude with PRD context
   - Claude implements one pending feature
   - Validates only status field was modified
   - Commits changes, updates progress
   - Repeats until all features complete

## PRD Format

```jsonc
{
  "project": {
    "name": "my-project",
    "description": "What this project does"
  },
  "verification": {
    "commands": [
      { "name": "check", "command": "cargo check" },
      { "name": "test", "command": "cargo test" }
    ],
    "runAfterEachFeature": true
  },
  "features": [
    {
      "id": "feature-id",
      "category": "functional",  // functional|bugfix|refactor|test|docs
      "description": "What needs to be done",
      "steps": ["Step 1", "Step 2"],
      "status": "pending"        // pending|in-progress|complete|blocked
    }
  ],
  "completion": {
    "allFeaturesComplete": true,
    "allVerificationsPassing": true,
    "marker": "<promise>COMPLETE</promise>"
  }
}
```

## Options

```
-p, --prd <PATH>                  PRD file path [default: prd.jsonc]
-P, --prompt <PATH>               Custom system prompt file
-m, --max-iterations <N>          Max iterations, 0=unlimited [default: 10]
-d, --delay <SECONDS>             Delay between iterations [default: 2]
-t, --timeout <SECONDS>           Claude timeout [default: 1800]
--permission-mode <MODE>          default|acceptEdits|plan [default: acceptEdits]
--continue-session                Preserve context between iterations
--skip-init                       Skip initialization phase
--dangerously-skip-permissions    Auto-approve all Claude actions
--init-prompt                     Generate prompt.md template and exit
```

## Custom Prompts

Ralph uses a built-in system prompt by default. To customize agent behavior, provide your own prompt file.

### Generate template

```bash
ralph --init-prompt              # Creates prompt.md
```

### Use custom prompt

```bash
ralph --prompt my-prompt.md      # Use custom prompt
ralph -P prompt.md               # Short form
```

### Placeholders

Custom prompts support these placeholders, replaced at runtime:

| Placeholder | Description |
|-------------|-------------|
| `{prd_path}` | Path to the PRD file |
| `{progress_path}` | Path to the progress file |
| `{verification_commands}` | Formatted list of verification commands |
| `{completion_marker}` | Completion marker from PRD |
| `{prd_content}` | Full contents of the PRD file |

### Example use case

Specialized prompts for different project types:

```bash
# Rust projects - strict linting, no unsafe
ralph --prompt prompts/rust-strict.md

# Python projects - pytest focus, type hints
ralph --prompt prompts/python.md

# Documentation - markdown style, grammar checks
ralph --prompt prompts/docs.md
```

## Safety

- **Validation** — Only PRD status field changes allowed per iteration
- **Failure limit** — Exits after 3 consecutive failures
- **Loop detection** — Detects stuck patterns and reports
- **Rate limiting** — Auto-retries after 60s cooldown
- **Ctrl+C** — Graceful shutdown with progress logged

## License

MIT
