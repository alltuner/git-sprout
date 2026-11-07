# Opt-in agent integration (Unix Bash/Zsh)

The installed `git-sprout` binary contains the installer and lifecycle handlers. No
checkout, Python runtime, shell command parser, or separately downloaded asset is
needed after installation. Python is used only by the integration test suite.

```sh
# Preview: writes nothing.
git sprout install-shim --agents --claude-worktrees
# Apply the reviewed plan.
git sprout install-shim --agents --claude-worktrees --apply
# Preview / apply removal; private pre-install backups are retained.
git sprout uninstall-shim
git sprout uninstall-shim --apply
```

Omit `--claude-worktrees` to keep Claude's native worktree lifecycle. Omit `--agents`
for a standalone shim, then source the printed activation file yourself. Installation
records the real Git executable and uses its stable PATH name (including Homebrew
symlinks). `--real-git /absolute/path/to/git` overrides discovery. A custom installation
location uses `--prefix DIR`; repeat that option for uninstall and scoped launch.
The default is `~/.local/share/git-sprout/shim`. Existing unmanaged directories are
rejected. Changes to existing files receive private backups beneath this directory.
Backups include original settings and may contain secrets; keep the prefix outside
version-controlled or shared directories.
Each backup directory contains a private `manifest.json` mapping numbered copies to
original paths and permission modes, including which files did not previously exist.
Uninstall leaves a harmless no-op activation file for manually sourced startup lines.

`--agents` adds marked Bash/Zsh startup blocks and guidance to global Codex and Claude
instruction files. Existing instruction content and file modes are preserved. Codex's first
nonempty `AGENTS.override.md` takes precedence over `AGENTS.md`. Claude instructions
live in each config directory's `CLAUDE.md`. Existing default profiles, `~/.codex*/config.toml`
and `~/.claude*/settings.json` profiles, and active `CODEX_HOME`/`CLAUDE_CONFIG_DIR`
are discovered. Profiles elsewhere require repeatable `--codex-home DIR` or
`--claude-home DIR`. Explicit home options and active profile environment variables
can intentionally create a new profile directory. Profiles created later require
another installation pass. With native Claude hooks enabled, `settings.json` is
reformatted and key order is normalized; existing values (including authentication
and MCP configuration) are preserved. Approval and sandbox policies are unchanged.
Reinstall and uninstall preserve later edits outside managed integration entries.

## Scope an agent process explicitly

Restart existing agents after installation. To ensure a launched executable and its
children inherit the shim PATH, without depending on interactive shell startup:

```sh
git sprout run -- codex
git sprout run -- claude
git sprout run -- env CODEX_HOME="$HOME/.codex3" codex
git sprout run -- /path/to/account-wrapper --existing-account-argument
```

The launcher takes executable argv, so shell-only aliases/functions are not expanded.
Use an executable account wrapper or `env` as shown. No account is reauthenticated.
Bash noninteractive startup and GUI application environments vary; startup changes
alone do not cover every such process. Absolute Git paths (such as `/usr/bin/git`),
processes that replace PATH, and integrations that bypass Git remain outside the
shim. Global instructions can also be overridden by more specific instructions.
This is explicit integration, not universal enforcement.

The shim accelerates only supported `git [global options] worktree add` forms, using
the same argv parser as Sprout. Other commands and unsupported flags are delegated
with the original argv to the recorded real Git. Normal Sprout fallback remains
available, with a recursion guard. If the Sprout binary or active installation
configuration disappears, the generated shim delegates directly to its embedded real
Git path. It never retries an add after Sprout has begun handling it. Existing core
hook/filter checks remain in force. An unset or empty PATH receives only the shim
directory; explicit existing PATH entries retain their user-selected behavior.

## Optional Claude native worktrees

`--claude-worktrees` adds documented `WorktreeCreate` and `WorktreeRemove` command
hooks to discovered Claude `settings.json` files. This covers Claude's native
worktree/subagent isolation even when it does not execute a PATH Git command.
A custom create hook also replaces Claude's native `.worktreeinclude` handling;
this integration does not copy those ignored/untracked files. Omit this option
when that native inclusion behavior is required.
The installer refuses foreign lifecycle hook entries in those settings files; it
does not discover plugin-supplied or project/local hooks. Resolve those separately
before enabling this option. Claude's normal trust/loading rules still apply.

The create handler accepts Claude's `cwd` and single-component `name`, creates
`<repository>/.claude/worktrees/<name>` on branch `claude/<name>`, and prints that path.
Existing branches/destinations are refused. Symlink destination components are
refused. Ownership is recorded privately in that worktree's Git metadata. If marker
writing fails after creation, the error reports the created path for manual recovery;
no force cleanup or branch deletion is attempted.

The remove handler only removes its own marked worktrees, using plain Git without
force. Modified, untracked **and ignored** files (including generated caches) prevent cleanup. Branches are always
retained. Refusal (including a missing ownership marker) exits nonzero so Claude reports
failed removal and keeps the existing worktree/session for manual review. An already
absent directory returns success. Claude does not fall back to Git after a registered
remove hook fails. An explicit user action such as `claude rm --force-remove-worktree`
can bypass that refusal; this handler does not prevent external deletion.
No PreToolUse input rewriting or permission/approval bypass is installed.

## Verification and contracts

```sh
cargo build --locked --workspace
SPROUT_BIN="$PWD/target/debug/git-sprout" python3 integrations/agents/test_installation.py
```

The suite uses temporary HOME directories and repositories only. It covers preview,
profile precedence/discovery, preservation/uninstall/reinstall, foreign hook refusal,
exact argv delegation, scoped account execution, real Git fallback without recursion,
native clean/dirty/untracked/ignored worktree lifecycle, missing-binary/config fallback,
true empty/unset PATH, absent default profiles, and recoverable backup mappings.

Official contracts checked for this implementation:

- [Codex global instruction precedence](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- [Codex hooks](https://learn.chatgpt.com/docs/hooks): no assumed `CODEX_ENV_FILE` behavior.
- [Claude hooks](https://code.claude.com/docs/en/hooks): native lifecycle inputs/output;
  SessionStart `CLAUDE_ENV_FILE` exists but is not needed by this installer.
- [Claude config directory](https://code.claude.com/docs/en/claude-directory):
  `CLAUDE_CONFIG_DIR` relocates profile instructions/settings.
