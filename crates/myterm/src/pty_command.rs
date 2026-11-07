//! Which shell a new pane starts.

use pty::Command;

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
