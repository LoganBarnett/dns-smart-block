# cargo-husky Pre-commit Hooks

This repository uses `cargo-husky` to automatically run `cargo fmt` before each commit.

## How it works

- `cargo-husky` is configured in the root `Cargo.toml` as a dev-dependency
- When you run any cargo command (like `cargo build`, `cargo test`, etc.), cargo-husky will automatically install the git hooks
- The hooks are defined in `.cargo-husky/hooks/pre-commit`

## Installation

The hooks will be automatically installed to `.git/hooks/` the first time you run any cargo command after cloning the repository:

```bash
cargo build
# or
cargo test
```

## What the pre-commit hook does

Before each commit, the hook will:
1. Run `cargo fmt --all --check` to verify all Rust code is properly formatted
2. If the code is not formatted, the commit will be blocked with an error message
3. You can then run `cargo fmt --all` to format the code and try committing again

## Bypassing the hook (not recommended)

If you absolutely need to commit unformatted code, you can bypass the hook with:

```bash
git commit --no-verify
```

However, this is not recommended as it defeats the purpose of having the hook.
