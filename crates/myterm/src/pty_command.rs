//! Which shell a new pane starts.

use pty::Command;

use crate::config::Shell;

#[derive(Clone, Debug)]
pub struct WslInstance {
    name: String,
    is_default: bool,
}

impl WslInstance {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_default(&self) -> bool {
        self.is_default
    }
}

/// The shell the user asked for, or the best one on this machine.
///
/// A chosen shell that is not installed falls back to the default rather than
/// failing: a preference is not a promise that the program exists, and a
/// terminal that will not start because of one is worse than a terminal that
/// starts in another shell.
pub fn profile(shell: Shell) -> Command {
    let (program, args) = match shell {
        Shell::Auto => return default_profile(),
        Shell::Pwsh => ("pwsh.exe", vec!["-NoLogo".to_owned()]),
        Shell::Windows => ("powershell.exe", vec!["-NoLogo".to_owned()]),
        Shell::Cmd => ("cmd.exe", Vec::new()),
        Shell::Wsl => ("wsl.exe", Vec::new()),
    };

    match find_on_path(program) {
        Some(path) => Command {
            program: path,
            args,
        },
        None => default_profile(),
    }
}

/// Starts one installed WSL distribution. A stale saved name falls back to the
/// default WSL instance, just as an unavailable shell falls back to Auto.
pub fn profile_for_distribution(shell: Shell, distribution: Option<&str>) -> Command {
    if shell != Shell::Wsl {
        return profile(shell);
    }
    let Some(distribution) = distribution else {
        return profile(shell);
    };
    if !wsl_instances()
        .iter()
        .any(|instance| instance.name() == distribution)
    {
        return profile(shell);
    }
    let Some(program) = find_on_path("wsl.exe") else {
        return default_profile();
    };
    Command {
        program,
        args: vec!["--distribution".to_owned(), distribution.to_owned()],
    }
}

/// Installed WSL distributions, including the one WSL starts by default.
/// The process result is cached for this app run because rendering can ask for
/// the shell label every frame.
pub fn wsl_instances() -> &'static [WslInstance] {
    static INSTANCES: std::sync::OnceLock<Vec<WslInstance>> = std::sync::OnceLock::new();
    INSTANCES.get_or_init(read_wsl_instances)
}

/// The distribution a WSL command will enter, if it can be identified.
pub fn wsl_distribution_name(command: &Command) -> Option<String> {
    let program = std::path::Path::new(&command.program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&command.program);
    if !program.eq_ignore_ascii_case("wsl.exe") && !program.eq_ignore_ascii_case("wsl") {
        return None;
    }
    command
        .args
        .windows(2)
        .find(|pair| pair[0] == "--distribution" || pair[0] == "-d")
        .map(|pair| pair[1].clone())
        .or_else(|| {
            wsl_instances()
                .iter()
                .find(|instance| instance.is_default())
                .map(|instance| instance.name().to_owned())
        })
}

fn read_wsl_instances() -> Vec<WslInstance> {
    let mut command = std::process::Command::new("wsl.exe");
    command.args(["--list", "--verbose"]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::install::CREATE_NO_WINDOW);
    }
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let text = if output.stdout.get(1) == Some(&0) {
        let units = output
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_le_bytes)
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };

    text.lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches('\u{feff}');
            let (is_default, line) = if let Some(line) = line.strip_prefix('*') {
                (true, line.trim_start())
            } else {
                (false, line)
            };
            let name = line.split_whitespace().next()?;
            if name.eq_ignore_ascii_case("NAME") || name.is_empty() {
                return None;
            }
            Some(WslInstance {
                name: name.to_owned(),
                is_default,
            })
        })
        .collect()
}

/// Picks the best shell installed on this machine.
///
/// PowerShell 7 first, then Windows PowerShell, then `cmd`. Windows always has
/// at least one of them, so there is no empty case.
pub fn default_profile() -> Command {
    for (program, args) in [
        ("pwsh.exe", vec!["-NoLogo".to_owned()]),
        ("powershell.exe", vec!["-NoLogo".to_owned()]),
        ("cmd.exe", Vec::new()),
    ] {
        if let Some(path) = find_on_path(program) {
            return Command {
                program: path,
                args,
            };
        }
    }

    Command {
        program: "cmd.exe".to_owned(),
        args: Vec::new(),
    }
}

/// Whether an explicitly selected shell can be resolved from `PATH`.
pub fn is_available(shell: Shell) -> bool {
    let program = match shell {
        Shell::Auto => return true,
        Shell::Pwsh => "pwsh.exe",
        Shell::Windows => "powershell.exe",
        Shell::Cmd => "cmd.exe",
        Shell::Wsl => "wsl.exe",
    };
    find_on_path(program).is_some()
}

/// A short, stable label for the executable a pane actually starts.
pub fn display_name(command: &Command) -> String {
    let file_name = std::path::Path::new(&command.program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&command.program);
    let normalized = file_name.to_ascii_lowercase();
    let program = normalized.strip_suffix(".exe").unwrap_or(&normalized);
    match program {
        "pwsh" => "PowerShell 7".to_owned(),
        "powershell" => "Windows PowerShell".to_owned(),
        "cmd" => "命令提示符".to_owned(),
        "wsl" => wsl_distribution_name(command)
            .map_or_else(|| "WSL".to_owned(), |name| format!("WSL · {name}")),
        _ => program.to_owned(),
    }
}

/// Wraps a PowerShell command so its prompt reports the working directory.
///
/// The shell only tells a terminal where it is if something asks it to, and the
/// usual way is OSC 7 in the prompt. PowerShell has no hook for that, so the
/// prompt function is wrapped: the original is kept and called, and the sequence
/// is written just before it. The wrapper is built as a single `-Command` string
/// rather than a script file, so nothing is written to disk.
///
/// Only PowerShell is covered. `cmd` has no prompt function to wrap, and `wsl`
/// runs another shell whose integration is that shell's business.
pub fn with_shell_integration(command: Command) -> Command {
    let name = std::path::Path::new(&command.program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !name.starts_with("pwsh") && !name.starts_with("powershell") {
        return command;
    }

    // The whole snippet is one argument. It saves the current prompt, installs
    // one that emits OSC 7 then delegates, and leaves `-NoExit` in place so the
    // shell stays interactive.
    let script = r#"
if (-not $global:__myterm_prompt) {
  $global:__myterm_prompt = $function:prompt
  function global:prompt {
    try {
      $p = $ExecutionContext.SessionState.Path.CurrentLocation.ProviderPath
      if ($p) { [Console]::Write([string][char]27 + ']7;file:///' + $p.Replace('\','/') + [char]7) }
    } catch {}
    if ($global:__myterm_prompt) { & $global:__myterm_prompt } else { "PS $($ExecutionContext.SessionState.Path.CurrentLocation)> " }
  }
}
"#
    .trim()
    .to_owned();

    Command {
        program: command.program,
        args: vec![
            "-NoLogo".to_owned(),
            "-NoExit".to_owned(),
            "-Command".to_owned(),
            script,
        ],
    }
}

/// Resolves `program` against `PATH`.
fn find_on_path(program: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}
