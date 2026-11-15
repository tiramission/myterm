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
