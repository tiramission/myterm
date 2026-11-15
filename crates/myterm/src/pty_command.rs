//! Which shell a new pane starts.

use pty::Command;

use crate::config::Shell;

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
