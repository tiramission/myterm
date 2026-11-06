//! The shells a tab can start.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Shown in the tab and the profile menu.
    pub name: String,
    /// The executable to start.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
}

/// The profiles this machine has, in menu order.
pub fn discover() -> Vec<Profile> {
    let mut profiles = Vec::new();

    if let Some(path) = find_on_path("pwsh.exe") {
        profiles.push(Profile {
            name: "PowerShell 7".to_owned(),
            program: path,
            args: vec!["-NoLogo".to_owned()],
        });
    }
    if let Some(path) = find_on_path("powershell.exe") {
        profiles.push(Profile {
            name: "Windows PowerShell".to_owned(),
            program: path,
            args: vec!["-NoLogo".to_owned()],
        });
    }
    if let Some(path) = find_on_path("cmd.exe") {
        profiles.push(Profile {
            name: "命令提示符".to_owned(),
            program: path,
            args: Vec::new(),
        });
    }
    if let Some(path) = find_on_path("wsl.exe") {
        profiles.push(Profile {
            name: "Ubuntu (WSL)".to_owned(),
            program: path,
            args: Vec::new(),
        });
    }
    if let Some(path) = find_git_bash() {
        profiles.push(Profile {
            name: "Git Bash".to_owned(),
            program: path,
            args: vec!["--login".to_owned(), "-i".to_owned()],
        });
    }

    if profiles.is_empty() {
        profiles.push(Profile {
            name: "命令提示符".to_owned(),
            program: "cmd.exe".to_owned(),
            args: Vec::new(),
        });
    }
    profiles
}

fn find_on_path(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

fn find_git_bash() -> Option<String> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            roots.push(PathBuf::from(root));
        }
    }
    roots.push(PathBuf::from(r"C:\Program Files"));
    roots
        .into_iter()
        .map(|root| root.join("Git").join("bin").join("bash.exe"))
        .find(|candidate| candidate.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}
