// ABOUTME: Explicit, user-owned Unix PATH integration and scoped agent launching.
// ABOUTME: Optional Claude lifecycle hooks preserve approval policies and native Git.

use std::ffi::OsString;
use std::process::ExitCode;

pub fn dispatch(args: &[OsString]) -> Option<ExitCode> {
    let name = args.first()?.to_str()?;
    if !matches!(
        name,
        "install-shim"
            | "uninstall-shim"
            | "__shim"
            | "__claude-worktree-create"
            | "__claude-worktree-remove"
            | "run"
    ) {
        return None;
    }
    #[cfg(unix)]
    let result = unix::dispatch(name, &args[1..]);
    #[cfg(not(unix))]
    let result: Result<ExitCode, String> =
        Err("automatic integration currently supports Unix Bash/Zsh only".into());
    Some(result.unwrap_or_else(|error| {
        eprintln!("git-sprout: {error}");
        ExitCode::from(1)
    }))
}

#[cfg(unix)]
mod unix {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const EARLY: &str = "# BEGIN git-sprout PATH (early)";
    const EARLY_END: &str = "# END git-sprout PATH (early)";
    const LATE: &str = "\n# BEGIN git-sprout PATH";
    const LATE_END: &str = "# END git-sprout PATH";
    const GUIDE: &str = "<!-- BEGIN git-sprout agent guidance -->";
    const GUIDE_END: &str = "<!-- END git-sprout agent guidance -->";
    const GUARD: &str = "GIT_SPROUT_SHIM_ACTIVE";
    const WORKTREE_MARKER: &str = "git-sprout-claude-worktree.json";

    fn hook_commands(binary: &Path, prefix: &Path) -> Result<[String; 2], String> {
        let binary = quote(&string(binary)?);
        let settings = quote(&string(&prefix.join("installation.json"))?);
        Ok([
            format!("{binary} __claude-worktree-create {settings}"),
            format!("{binary} __claude-worktree-remove {settings}"),
        ])
    }
    fn claude_hooks(
        value: &str,
        commands: &[String; 2],
        old_commands: &[String; 2],
        add: bool,
    ) -> Result<String, String> {
        let mut settings: Value = if value.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(value).map_err(|e| format!("invalid Claude settings: {e}"))?
        };
        let object = settings
            .as_object_mut()
            .ok_or("Claude settings must be an object")?;
        let hooks = object
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("Claude hooks must be an object")?;
        for (index, event) in ["WorktreeCreate", "WorktreeRemove"].into_iter().enumerate() {
            let groups = hooks.remove(event).unwrap_or_else(|| json!([]));
            let mut remaining = vec![];
            for group in groups
                .as_array()
                .ok_or("Claude worktree hooks must be an array")?
            {
                let mut group = group.clone();
                let handlers = group["hooks"]
                    .as_array()
                    .ok_or("Claude worktree hook group has no handlers")?
                    .iter()
                    .filter(|handler| {
                        let command = handler["command"].as_str();
                        command != Some(commands[index].as_str())
                            && command != Some(old_commands[index].as_str())
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if !handlers.is_empty() {
                    group["hooks"] = json!(handlers);
                    remaining.push(group);
                }
            }
            if add && !remaining.is_empty() {
                return Err(format!("existing non-sprout {event} hook: refusing to install a second worktree lifecycle"));
            }
            if add {
                remaining.push(json!({"hooks":[{"type":"command","command":commands[index]}]}));
            }
            if !remaining.is_empty() {
                hooks.insert(event.into(), json!(remaining));
            }
        }
        if hooks.is_empty() {
            object.remove("hooks");
        }
        Ok(serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())? + "\n")
    }

    fn text(path: &Path) -> Result<String, String> {
        match fs::read_to_string(path) {
            Ok(value) => Ok(value),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
    fn string(path: &Path) -> Result<String, String> {
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| "integration paths must be UTF-8".into())
    }
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
    fn absolute(value: &std::ffi::OsStr) -> Result<PathBuf, String> {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path))
        }
    }
    fn executable(path: &Path) -> bool {
        fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    fn resolve_file(path: PathBuf) -> Result<PathBuf, String> {
        if path.is_symlink() {
            fs::canonicalize(&path).map_err(|e| format!("{}: {e}", path.display()))
        } else {
            Ok(path)
        }
    }
    fn write(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
        let parent = path.parent().ok_or("file has no parent")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temporary = parent.join(format!(
            ".sprout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
        let result = (|| {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(data).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
                .map_err(|e| e.to_string())?;
            fs::rename(&temporary, path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    fn strip(mut value: String, begin: &str, end: &str) -> Result<String, String> {
        if let Some(start) = value.find(begin) {
            let finish = value[start..]
                .find(end)
                .ok_or("incomplete git-sprout managed block; repair it before reinstalling")?
                + start
                + end.len();
            let finish = finish + usize::from(value.as_bytes().get(finish) == Some(&b'\n'));
            value.replace_range(start..finish, "");
            if value.contains(begin) {
                return Err(
                    "duplicate git-sprout managed blocks; inspect before reinstalling".into(),
                );
            }
        }
        Ok(value)
    }
    fn strip_owned(value: String) -> Result<String, String> {
        strip(
            strip(strip(value, EARLY, EARLY_END)?, LATE, LATE_END)?,
            GUIDE,
            GUIDE_END,
        )
    }
    fn config(prefix: &Path) -> Result<Value, String> {
        let value: Value = serde_json::from_str(&text(&prefix.join("installation.json"))?)
            .map_err(|_| "shim is not installed here; run install-shim --apply first")?;
        if value["version"] != 1 {
            return Err("unrecognized git-sprout installation metadata".into());
        }
        Ok(value)
    }
    struct Options {
        apply: bool,
        agents: bool,
        claude_worktrees: bool,
        prefix: PathBuf,
        real_git: Option<PathBuf>,
        codex: Vec<PathBuf>,
        claude: Vec<PathBuf>,
        home: PathBuf,
    }
    fn options(args: &[OsString]) -> Result<Options, String> {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
        let mut opts = Options {
            apply: false,
            agents: false,
            claude_worktrees: false,
            prefix: home.join(".local/share/git-sprout/shim"),
            real_git: None,
            codex: vec![],
            claude: vec![],
            home,
        };
        let mut index = 0;
        while index < args.len() {
            let flag = args[index].to_str().ok_or("invalid installer option")?;
            index += 1;
            match flag {
                "--apply" => opts.apply = true,
                "--agents" => opts.agents = true,
                "--claude-worktrees" => opts.claude_worktrees = true,
                "--prefix" | "--real-git" | "--codex-home" | "--claude-home" => {
                    let value =
                        absolute(args.get(index).ok_or("installer option requires a path")?)?;
                    index += 1;
                    match flag {
                        "--prefix" => opts.prefix = value,
                        "--real-git" => opts.real_git = Some(value),
                        "--codex-home" => opts.codex.push(value),
                        _ => opts.claude.push(value),
                    }
                }
                _ => return Err(format!("unknown installer option: {flag}")),
            }
        }
        if string(&opts.prefix)?.contains([':', '\n']) {
            return Err("shim directory cannot contain ':' or newline in PATH".into());
        }
        if opts.claude_worktrees && !opts.agents {
            return Err("--claude-worktrees requires --agents".into());
        }
        Ok(opts)
    }
    fn find_binary() -> Result<PathBuf, String> {
        // Preserve a stable Homebrew/cargo PATH name instead of pinning its Cellar target.
        let current = std::env::current_exe().map_err(|e| e.to_string())?;
        let invoked = std::env::args_os().next().unwrap_or_default();
        if Path::new(&invoked).components().count() > 1 {
            return absolute(&invoked);
        }
        for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            let candidate = directory.join("git-sprout");
            if fs::canonicalize(&candidate).ok().as_ref() == Some(&current) {
                return Ok(candidate);
            }
        }
        Ok(current)
    }
    fn find_git(opts: &Options, old: Option<&Value>) -> Result<PathBuf, String> {
        if let Some(path) = &opts.real_git {
            return Ok(path.clone());
        }
        if let Some(value) = old.and_then(|v| v["real_git"].as_str()) {
            return Ok(PathBuf::from(value));
        }
        for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            let candidate = directory.join("git");
            if candidate != opts.prefix.join("git") && executable(&candidate) {
                return absolute(candidate.as_os_str());
            }
        }
        Err("real git is not on PATH; specify --real-git /absolute/path/to/git".into())
    }
    fn profiles(opts: &Options) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>), String> {
        let mut codex = BTreeSet::new();
        let mut claude = BTreeSet::new();
        if opts.home.join(".codex").is_dir() {
            codex.insert(opts.home.join(".codex"));
        }
        if opts.home.join(".claude").is_dir() {
            claude.insert(opts.home.join(".claude"));
        }
        for entry in fs::read_dir(&opts.home).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let p = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if p.is_dir() && name.starts_with(".codex") && p.join("config.toml").is_file() {
                codex.insert(p.clone());
            }
            if p.is_dir() && name.starts_with(".claude") && p.join("settings.json").is_file() {
                claude.insert(p);
            }
        }
        if let Some(path) = std::env::var_os("CODEX_HOME") {
            codex.insert(absolute(&path)?);
        }
        if let Some(path) = std::env::var_os("CLAUDE_CONFIG_DIR") {
            claude.insert(absolute(&path)?);
        }
        codex.extend(opts.codex.iter().cloned());
        claude.extend(opts.claude.iter().cloned());
        Ok((codex, claude))
    }
    fn install(opts: Options) -> Result<ExitCode, String> {
        let old = if opts.prefix.join("installation.json").exists() {
            Some(config(&opts.prefix)?)
        } else {
            None
        };
        if old.is_none()
            && opts.prefix.is_dir()
            && fs::read_dir(&opts.prefix)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err("installation directory is non-empty and not managed by git-sprout".into());
        }
        let git = find_git(&opts, old.as_ref())?;
        let binary = find_binary()?;
        if !executable(&git)
            || fs::canonicalize(&git).ok() == fs::canonicalize(opts.prefix.join("git")).ok()
            || text(&git)
                .unwrap_or_default()
                .contains("# git-sprout managed shim")
        {
            return Err("--real-git must name an executable outside the shim".into());
        }
        let activation = opts.prefix.join("activate.sh");
        let source = format!(". {}", quote(&string(&activation)?));
        let mut changes: BTreeMap<PathBuf, String> = BTreeMap::new();
        let mut settings_files = BTreeSet::new();
        let native_hooks = opts.claude_worktrees
            || old
                .as_ref()
                .is_some_and(|v| v["installed"] == true && v["claude_worktrees"] == true);
        if native_hooks && !opts.agents {
            return Err("existing native Claude hooks require --agents when reinstalling".into());
        }
        let commands = hook_commands(&binary, &opts.prefix)?;
        let old_binary = old
            .as_ref()
            .and_then(|v| v["binary"].as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| binary.clone());
        let old_commands = hook_commands(&old_binary, &opts.prefix)?;
        if opts.agents {
            let bashrc = resolve_file(opts.home.join(".bashrc"))?;
            let original = strip_owned(text(&bashrc)?)?;
            let early = format!("{EARLY}\n{source}\n{EARLY_END}\n");
            let late = format!("{LATE}\n{source}\n{LATE_END}\n");
            let content = if original.starts_with("#!") {
                if let Some(index) = original.find('\n') {
                    format!(
                        "{}{early}{}{late}",
                        &original[..index + 1],
                        &original[index + 1..]
                    )
                } else {
                    format!("{original}\n{early}{late}")
                }
            } else {
                format!("{early}{original}{late}")
            };
            changes.insert(bashrc, content);
            let bashlogin = [".bash_profile", ".bash_login", ".profile"]
                .into_iter()
                .map(|n| opts.home.join(n))
                .find(|p| p.exists())
                .unwrap_or_else(|| opts.home.join(".profile"));
            for path in [
                bashlogin,
                opts.home.join(".zshenv"),
                opts.home.join(".zprofile"),
                opts.home.join(".zshrc"),
            ] {
                let path = resolve_file(path)?;
                let content = strip_owned(text(&path)?)?;
                changes.insert(path, format!("{content}{late}"));
            }
            let (codex, mut claude) = profiles(&opts)?;
            if native_hooks {
                if let Some(rows) = old.as_ref().and_then(|v| v["files"].as_array()) {
                    for row in rows {
                        if row["kind"] == "claude_settings" {
                            if let Some(parent) =
                                row["path"].as_str().and_then(|p| Path::new(p).parent())
                            {
                                claude.insert(parent.to_path_buf());
                            }
                        }
                    }
                }
            }
            let guidance=format!("{GUIDE}\nFor ordinary new Git worktrees, use `git worktree add` through the installed git-sprout PATH shim. Preserve its normal Git fallback; do not disable hooks or filters to force acceleration. Avoid absolute Git paths unless the user requests native Git or you are diagnosing the integration. When PATH is not inherited, launch the agent with `git sprout run -- <agent executable> [args...]`. This integration never changes approval or sandbox policy.\n{GUIDE_END}\n");
            for profile in codex {
                let chosen = if !text(&profile.join("AGENTS.override.md"))?.trim().is_empty() {
                    "AGENTS.override.md"
                } else {
                    "AGENTS.md"
                };
                let path = resolve_file(profile.join(chosen))?;
                let content = strip_owned(text(&path)?)?;
                changes.insert(path, format!("{guidance}{content}"));
            }
            for profile in claude {
                let path = resolve_file(profile.join("CLAUDE.md"))?;
                let content = strip_owned(text(&path)?)?;
                changes.insert(path, format!("{guidance}{content}"));
                if native_hooks {
                    let path = resolve_file(profile.join("settings.json"))?;
                    let value = claude_hooks(&text(&path)?, &commands, &old_commands, true)?;
                    settings_files.insert(path.clone());
                    changes.insert(path, value);
                }
            }
        }
        println!("Shim directory: {}", opts.prefix.display());
        for path in changes.keys() {
            println!("Update: {}", string(path)?);
        }
        println!("PATH activation: {source}");
        if !opts.apply {
            println!("Preview only; add --apply. --agents also activates Bash/Zsh and all discovered Codex/Claude profiles.");
            return Ok(ExitCode::SUCCESS);
        }
        fs::create_dir_all(&opts.prefix).map_err(|e| e.to_string())?;
        fs::set_permissions(&opts.prefix, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let backup = opts.prefix.join("backups").join(stamp.to_string());
        fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
        let mut originals = vec![];
        let mut files: BTreeMap<String, Value> = BTreeMap::new();
        if let Some(rows) = old.as_ref().and_then(|v| v["files"].as_array()) {
            for row in rows {
                if let Some(path) = row["path"].as_str() {
                    files.insert(path.to_owned(), row.clone());
                }
            }
        }
        let mut backup_manifest = vec![];
        for (index, (path, _)) in changes.iter().enumerate() {
            let before = fs::read(path).ok();
            let mode = fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o600);
            if let Some(data) = &before {
                write(&backup.join(index.to_string()), data, 0o600)?;
            }
            backup_manifest.push(json!({"path":path,"mode":mode,"existed":before.is_some(),
                "backup":if before.is_some() {Some(index.to_string())} else {None}}));
            files.entry(string(path)?).or_insert_with(||json!({"path":path,"created":before.is_none(),"kind":if settings_files.contains(path) {"claude_settings"}else{"text"}}));
            originals.push((path.clone(), before, mode));
        }
        write(
            &backup.join("manifest.json"),
            serde_json::to_string_pretty(&backup_manifest)
                .map_err(|e| e.to_string())?
                .as_bytes(),
            0o600,
        )?;
        let settings = json!({"version":1,"installed":true,"real_git":git,"binary":binary,"claude_worktrees":native_hooks,"files":files.into_values().collect::<Vec<_>>()});
        for name in ["git", "activate.sh", "installation.json"] {
            let path = opts.prefix.join(name);
            let before = fs::read(&path).ok();
            let mode = fs::metadata(&path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o600);
            originals.push((path, before, mode));
        }
        let result = (|| {
            write(
                &opts.prefix.join("git"),
                format!(
                    "#!/bin/sh\n# git-sprout managed shim\nif [ -x {binary} ]; then\n  exec {binary} __shim {config} {real} \"$@\"\nfi\nexec {real} \"$@\"\n",
                    binary=quote(&string(&binary)?),
                    config=quote(&string(&opts.prefix.join("installation.json"))?),
                    real=quote(&string(&git)?)
                )
                .as_bytes(),
                0o700,
            )?;
            write(&activation,format!("# git-sprout activation; safe to source more than once\ncase \"${{PATH-}}\" in {prefix}|{prefix}:*) ;; *) export PATH={prefix}\"${{PATH:+:$PATH}}\" ;; esac\n",prefix=quote(&string(&opts.prefix)?)).as_bytes(),0o600)?;
            write(
                &opts.prefix.join("installation.json"),
                serde_json::to_string_pretty(&settings)
                    .map_err(|e| e.to_string())?
                    .as_bytes(),
                0o600,
            )?;
            for (path, contents) in &changes {
                let mode = originals
                    .iter()
                    .find(|x| &x.0 == path)
                    .map(|x| x.2)
                    .unwrap_or(0o600);
                write(path, contents.as_bytes(), mode)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            for (path, before, mode) in originals {
                if let Some(bytes) = before {
                    let _ = write(&path, &bytes, mode);
                } else {
                    let _ = fs::remove_file(path);
                }
            }
            return Err(error);
        }
        println!("Installed. Restart existing agent sessions; use `git sprout run -- codex` or `git sprout run -- claude` for explicit inherited PATH.");
        Ok(ExitCode::SUCCESS)
    }
    fn uninstall(opts: Options) -> Result<ExitCode, String> {
        if !opts.prefix.join("installation.json").exists() {
            println!("No managed shim at {}", opts.prefix.display());
            return Ok(ExitCode::SUCCESS);
        }
        let data = config(&opts.prefix)?;
        if data["installed"] == false {
            println!("No active shim at {}", opts.prefix.display());
            return Ok(ExitCode::SUCCESS);
        }
        let mut changes = vec![];
        let commands = hook_commands(
            Path::new(
                data["binary"]
                    .as_str()
                    .ok_or("missing installed binary path")?,
            ),
            &opts.prefix,
        )?;
        for row in data["files"]
            .as_array()
            .ok_or("installation metadata has no file list")?
        {
            let path = PathBuf::from(row["path"].as_str().ok_or("invalid managed file path")?);
            if path.exists() {
                let value = if row["kind"] == "claude_settings" {
                    claude_hooks(&text(&path)?, &commands, &commands, false)?
                } else {
                    strip_owned(text(&path)?)?
                };
                changes.push((
                    path.clone(),
                    value,
                    row["created"].as_bool() == Some(true),
                    row["kind"] == "claude_settings",
                ));
            }
        }
        for (path, _, _, _) in &changes {
            println!("Remove managed integration: {}", path.display());
        }
        if !opts.apply {
            println!("Preview only; add --apply to uninstall.");
            return Ok(ExitCode::SUCCESS);
        }
        for (path, value, created, settings) in changes {
            let empty = value.is_empty()
                || (settings
                    && serde_json::from_str::<Value>(&value)
                        .ok()
                        .is_some_and(|v| v.as_object().is_some_and(|v| v.is_empty())));
            if created && empty {
                fs::remove_file(path).map_err(|e| e.to_string())?;
            } else {
                let mode = fs::metadata(&path)
                    .map_err(|e| e.to_string())?
                    .permissions()
                    .mode()
                    & 0o777;
                write(&path, value.as_bytes(), mode)?;
            }
        }
        fs::remove_file(opts.prefix.join("git")).map_err(|e| e.to_string())?;
        write(
            &opts.prefix.join("activate.sh"),
            b"# git-sprout uninstalled; retained for manually sourced startup files\n",
            0o600,
        )?;
        let retired = json!({"version":1,"installed":false,"real_git":data["real_git"],"binary":data["binary"],"files":[]});
        write(
            &opts.prefix.join("installation.json"),
            serde_json::to_string_pretty(&retired)
                .map_err(|e| e.to_string())?
                .as_bytes(),
            0o600,
        )?;
        println!(
            "Uninstalled; private backups retained at {}",
            opts.prefix.join("backups").display()
        );
        Ok(ExitCode::SUCCESS)
    }
    fn exec(mut command: Command) -> Result<ExitCode, String> {
        Err(command.exec().to_string())
    }
    fn shim(args: &[OsString]) -> Result<ExitCode, String> {
        let path = PathBuf::from(args.first().ok_or("missing shim configuration")?);
        let fallback = args.get(1).ok_or("missing embedded real Git path")?;
        let original = &args[2..];
        let data = text(&path)
            .ok()
            .and_then(|v| serde_json::from_str::<Value>(&v).ok());
        let active = data.as_ref().filter(|v| {
            v["version"] == 1
                && v["installed"] == true
                && v["real_git"]
                    .as_str()
                    .is_some_and(|p| Path::new(p).is_absolute() && executable(Path::new(p)))
        });
        let Some(data) = active else {
            let mut command = Command::new(fallback);
            command.args(original);
            return exec(command);
        };
        let real = data["real_git"].as_str().ok_or("invalid shim real_git")?;
        if std::env::var_os(GUARD).as_deref() != Some(std::ffi::OsStr::new("1")) {
            if let Some(tail) = crate::argv::shim_add_args(original) {
                std::env::set_var(GUARD, "1");
                return Ok(crate::run(tail));
            }
        }
        let mut command = Command::new(real);
        command.args(original);
        exec(command)
    }
    fn prepended_path(prefix: &Path) -> Result<OsString, String> {
        let mut paths = vec![prefix.to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH").filter(|p| !p.is_empty()) {
            paths.extend(std::env::split_paths(&existing));
        }
        std::env::join_paths(paths).map_err(|e| e.to_string())
    }
    fn launch(args: &[OsString]) -> Result<ExitCode, String> {
        let separator = args
            .iter()
            .position(|v| v == "--")
            .ok_or("usage: git sprout run [--prefix PATH] -- COMMAND [ARGS...]")?;
        let opts = options(&args[..separator])?;
        let command_args = &args[separator + 1..];
        let name = command_args
            .first()
            .ok_or("run requires an executable after --")?;
        if config(&opts.prefix)?["installed"] != true {
            return Err("shim is not active; run install-shim --apply first".into());
        }
        let path = prepended_path(&opts.prefix)?;
        let mut command = Command::new(name);
        command
            .args(&command_args[1..])
            .env("PATH", path)
            .env_remove(GUARD);
        exec(command)
    }
    fn git_output(git: &str, cwd: &Path, args: &[&str]) -> Result<String, String> {
        let result = Command::new(git)
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err("could not inspect Git worktree".into());
        }
        String::from_utf8(result.stdout)
            .map(|s| s.trim_end_matches(['\n', '\r']).to_owned())
            .map_err(|_| "Git returned a non-UTF-8 hook path".into())
    }
    fn safe_slug(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 100
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    }
    fn worktree_hook(name: &str, args: &[OsString]) -> Result<ExitCode, String> {
        let config_path = PathBuf::from(args.first().ok_or("missing hook configuration")?);
        let data: Value = serde_json::from_str(&text(&config_path)?).map_err(|e| e.to_string())?;
        if data["installed"] != true || data["claude_worktrees"] != true {
            return Err("native Claude worktree integration is not active".into());
        }
        let input: Value = serde_json::from_reader(std::io::stdin()).map_err(|e| e.to_string())?;
        let git = data["real_git"].as_str().ok_or("invalid real git")?;
        if name == "__claude-worktree-create" {
            let cwd = PathBuf::from(input["cwd"].as_str().ok_or("missing hook cwd")?);
            let slug = input["name"].as_str().ok_or("missing worktree name")?;
            if !safe_slug(slug) {
                return Err(
                    "worktree name must be a single alphanumeric/dash/underscore slug".into(),
                );
            }
            let root = fs::canonicalize(git_output(git, &cwd, &["rev-parse", "--show-toplevel"])?)
                .map_err(|e| e.to_string())?;
            let directory = root.join(".claude").join("worktrees");
            let target = directory.join(slug);
            if root.join(".claude").is_symlink()
                || directory.is_symlink()
                || target.exists()
                || target.is_symlink()
            {
                return Err("worktree destination already exists or traverses a symlink".into());
            }
            fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            let binary = data["binary"].as_str().ok_or("missing installed binary")?;
            let prefix = config_path
                .parent()
                .ok_or("hook configuration has no parent")?;
            let path = prepended_path(prefix)?;
            let status = Command::new(binary)
                .env("PATH", path)
                .env(GUARD, "1")
                .arg("-C")
                .arg(&root)
                .arg("add")
                .arg("-b")
                .arg(format!("claude/{slug}"))
                .arg(&target)
                .stdout(std::process::Stdio::null())
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("git-sprout could not create the Claude worktree".into());
            }
            let gitdir = PathBuf::from(git_output(
                git,
                &target,
                &["rev-parse", "--absolute-git-dir"],
            )?);
            write(
                &gitdir.join(WORKTREE_MARKER),
                serde_json::to_string(&json!({"root":root,"path":target,"name":slug}))
                    .map_err(|e| e.to_string())?
                    .as_bytes(),
                0o600,
            ).map_err(|error| format!("created worktree at {}; ownership marker could not be written: {error}; keep it for manual recovery", target.display()))?;
            println!("{}", target.display());
        } else {
            let target = PathBuf::from(
                input["worktree_path"]
                    .as_str()
                    .ok_or("missing worktree_path")?,
            );
            if !target.exists() {
                return Ok(ExitCode::SUCCESS);
            }
            let gitdir = PathBuf::from(git_output(
                git,
                &target,
                &["rev-parse", "--absolute-git-dir"],
            )?);
            let marker = gitdir.join(WORKTREE_MARKER);
            if !marker.is_file() {
                return Err("keeping worktree without sprout ownership marker".into());
            }
            let record: Value = serde_json::from_str(&text(&marker)?).map_err(|e| e.to_string())?;
            let root = PathBuf::from(record["root"].as_str().ok_or("invalid worktree marker")?);
            let slug = record["name"]
                .as_str()
                .ok_or("invalid worktree marker name")?;
            if !safe_slug(slug)
                || target != root.join(".claude/worktrees").join(slug)
                || record["path"].as_str() != target.to_str()
                || target.is_symlink()
                || root.join(".claude").is_symlink()
                || root.join(".claude/worktrees").is_symlink()
            {
                return Err("worktree removal ownership/path mismatch".into());
            }
            let dirty = git_output(
                git,
                &target,
                &[
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                    "--ignored=matching",
                ],
            )?;
            if !dirty.is_empty() {
                return Err(
                    "keeping worktree containing modified, untracked or ignored files".into(),
                );
            }
            let status = Command::new(git)
                .arg("-C")
                .arg(&root)
                .args(["worktree", "remove"])
                .arg(&target)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("Git refused to remove the worktree; no force was used".into());
            }
        }
        Ok(ExitCode::SUCCESS)
    }
    pub(super) fn dispatch(name: &str, args: &[OsString]) -> Result<ExitCode, String> {
        if args.iter().any(|a| a == "--help") && name != "__shim" && name != "run" {
            println!("git sprout {name} [--apply] [--agents] [--claude-worktrees] [--prefix DIR] [--real-git PATH] [--codex-home DIR] [--claude-home DIR]\nPreview is the default. --agents activates Bash/Zsh PATH and profile guidance; --claude-worktrees opts into native Claude isolation hooks. Unix only.");
            return Ok(ExitCode::SUCCESS);
        }
        match name {
            "install-shim" => install(options(args)?),
            "uninstall-shim" => uninstall(options(args)?),
            "run" => launch(args),
            "__shim" => shim(args),
            "__claude-worktree-create" | "__claude-worktree-remove" => worktree_hook(name, args),
            _ => unreachable!(),
        }
    }
}
