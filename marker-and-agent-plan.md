# Completion Marker And Agent Instruction Plan

## Goal

Move the completion marker out of the PRD and into Rust code as a built-in default, while keeping `--completion-marker` as the override. Preserve `runAfterEachFeature` as agent guidance only, not runtime-enforced verification.

## Desired Behavior

- Ralph has a built-in default completion marker in Rust.
- `--completion-marker` overrides that built-in default everywhere.
- The PRD template no longer contains a completion marker field.
- Existing PRDs that still include `completion.marker` continue to parse without breaking runs.
- `runAfterEachFeature` stays in the PRD and influences prompt instructions only.

## Implementation Plan

### 1. Add a Rust-owned default completion marker

- Introduce a constant and small resolver helper, likely in `src/config.rs` or a nearby runtime module.
- Resolution order should be:
  - CLI `--completion-marker`
  - built-in Rust default
- Reject an empty CLI override instead of treating it as a valid marker.

### 2. Stop storing the marker in the PRD model

- Remove `marker` from the `Completion` struct in `src/prd.rs`.
- Remove the marker entry from the generated PRD template.
- Update `prd.example.jsonc` to match.
- Rely on serde's default unknown-field tolerance so older PRDs with `marker` still load.

### 3. Use the resolved marker everywhere

- Compute the effective completion marker once in `src/runner.rs`.
- Pass that marker into prompt generation.
- Pass that same marker into iteration output analysis.
- Remove any code path that still reads the marker from the PRD.

### 4. Update prompt generation to reflect the new source of truth

- Change `src/prompt.rs` so the completion placeholder uses the resolved runtime marker, not PRD data.
- Keep custom prompt support intact.
- Ensure built-in and custom prompts both see the same effective marker value.

### 5. Keep `runAfterEachFeature` as agent guidance only

- Do not add verification execution to the normal Rust loop.
- Keep runtime verification execution limited to explicit modes like dry-run.
- Update the built-in prompt in `src/prompt.rs` so the instructions change based on `prd.verification.run_after_each_feature`:
  - if `true`, tell the agent to run verification before marking a feature complete
  - if `false`, tell the agent verification is not required after every feature and must at least be run before final completion

### 6. Align docs with the actual contract

- Update `README.md` to say:
  - the completion marker is built into Ralph and can be overridden with `--completion-marker`
  - `runAfterEachFeature` is an instruction to the agent, not a Rust runtime guarantee
- Update the PRD format example to remove the marker field.

### 7. Add regression coverage

- Tests for default marker resolution.
- Tests for CLI override precedence.
- Tests that empty CLI overrides are rejected.
- Tests that prompt substitution uses the resolved marker.
- Tests that prompt text changes based on `runAfterEachFeature`.
- Tests that legacy PRDs with an extra `completion.marker` field still parse successfully.

## Code Areas

- `src/config.rs`
- `src/runner.rs`
- `src/prompt.rs`
- `src/prd.rs`
- `prd.example.jsonc`
- `README.md`

## Migration Notes

- This should be backward-compatible for users with older PRDs because extra fields are ignored during deserialization.
- The only user-visible behavior change is that the marker now comes from Ralph itself unless overridden on the CLI.

## Recommended Order

1. Introduce the built-in marker resolver.
2. Remove marker usage from PRD parsing and template generation.
3. Thread the resolved marker through runner, prompt, and output analysis.
4. Update prompt behavior for `runAfterEachFeature`.
5. Add tests.
6. Update docs and examples.
