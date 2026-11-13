//! What was open when the app last closed.
//!
//! The record is deliberately thin. A tab is a shell and a shell can be started
//! again from its program and arguments; what the program was doing lives in
//! the program's own history. Offering to rebuild the tabs is honest, offering
//! to restore a half-finished build would not be.

use std::path::PathBuf;

/// One tab as it was when the window closed.
pub struct SnapshotTab {
    pub title: String,
    pub program: String,
    pub args: Vec<String>,
}

/// The tabs as they were when the window closed.
pub struct Snapshot {
    pub tabs: Vec<SnapshotTab>,
}

/// The separator between fields.
///
/// A control character rather than a tab: a shell title is arbitrary text, and
/// a title containing a tab would otherwise invent a column and shift every
/// argument after it.
const FIELD: char = '\u{1f}';

/// Where the record lives, beside the install location rather than in it: a
/// portable copy has as much right to remember its tabs as an installed one.
fn path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("myterm").join("session.txt"))
}

impl Snapshot {
    /// Reads the record, if there is one worth reading.
    ///
    /// Every failure is the same failure — a missing file, a truncated line, a
    /// field that does not parse — and the answer to all of them is "nothing to
    /// offer", so none of them is surfaced.
    pub fn load() -> Option<Self> {
        parse(&std::fs::read_to_string(path()?).ok()?)
    }

    /// Writes the record, replacing the previous one.
    ///
    /// A failure here is a failure to remember, not a failure to close: the
    /// window still goes away, and the next start simply has nothing to offer.
    pub fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        if let Some(dir) = path.parent()
            && std::fs::create_dir_all(dir).is_err()
        {
            return;
        }

        let text = self.encode();
        let _ = std::fs::write(path, text);
    }

    /// The record as text.
    fn encode(&self) -> String {
        let mut text = String::new();
        for tab in &self.tabs {
            text.push_str(&tab.title);
            text.push(FIELD);
            text.push_str(&tab.program);
            for arg in &tab.args {
                text.push(FIELD);
                text.push_str(arg);
            }
            text.push('\n');
        }
        text
    }

    /// Forgets the record, for the times the user says no.
    pub fn clear() {
        if let Some(path) = path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Reads a record from text.
///
/// Apart from the file so the format can be tested without touching the user's
/// own record: a test that wrote to `%LOCALAPPDATA%` would run against real
/// data and could delete the tabs someone had open.
fn parse(text: &str) -> Option<Snapshot> {
    let tabs = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(FIELD);
            let title = fields.next()?;
            let program = fields.next()?;
            if program.is_empty() {
                return None;
            }
            Some(SnapshotTab {
                title: title.to_owned(),
                program: program.to_owned(),
                args: fields.map(str::to_owned).collect(),
            })
        })
        .collect::<Vec<_>>();

    // One tab is what the app starts with anyway; offering to restore it would
    // be offering to do nothing.
    (tabs.len() > 1).then_some(Snapshot { tabs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_tab_is_not_worth_restoring() {
        assert!(
            parse("pwsh\u{1f}C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\pwsh.exe\n").is_none()
        );
    }

    #[test]
    fn tabs_keep_their_arguments() {
        let text = "build\u{1f}pwsh.exe\u{1f}-NoLogo\u{1f}-NoProfile\nlogs\u{1f}pwsh.exe\n";
        let snapshot = parse(text).expect("two tabs is worth restoring");

        assert_eq!(snapshot.tabs.len(), 2);
        assert_eq!(snapshot.tabs[0].title, "build");
        assert_eq!(
            snapshot.tabs[0].args,
            vec!["-NoLogo".to_owned(), "-NoProfile".to_owned()]
        );
        assert!(snapshot.tabs[1].args.is_empty());
    }

    #[test]
    fn a_line_without_a_program_is_ignored() {
        let text = "build\u{1f}pwsh.exe\ntorn\u{1f}\nlogs\u{1f}pwsh.exe\n";
        let snapshot = parse(text).expect("two good tabs remain");
        assert_eq!(snapshot.tabs.len(), 2);
    }
}
