//! The programs running now, for Settings' picker to tie a game to one, as
//! Discord's "Add it!" lists them: this user's, as detection sees each, so
//! the path kept is the one detection matches; the newest first, as the game
//! just started is the one wanted. Listed once each time the picker asks,
//! and never kept here.

use std::collections::HashMap;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// A program running now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// Its file name.
    pub name: String,
    /// What detection matches it by: its executable's path, or for a
    /// Windows program under Wine, the one in its command line.
    pub path: String,
}

/// The newest of this user's programs, at most `limit`, each once; none of
/// the system's own, and not Yapfast.
pub fn running(limit: usize) -> Vec<Program> {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return Vec::new();
    }
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_exe(UpdateKind::Always)
            .with_user(UpdateKind::OnlyIfNotSet),
    );
    let own = sysinfo::get_current_pid().ok();
    let own_process = own.and_then(|pid| system.process(pid));
    let me = own_process.and_then(|process| process.user_id()).cloned();
    let own_exe = own_process.and_then(|process| process.exe().map(ToOwned::to_owned));
    let mine = |process: &sysinfo::Process| {
        me.as_ref()
            .zip(process.user_id())
            .is_none_or(|(me, other)| me == other)
    };
    // Only Wine's command lines are read, for the Windows program in each.
    let wine: Vec<_> = (system.processes().iter())
        .filter(|&(_, process)| mine(process) && super::is_wine(process))
        .map(|(pid, _)| *pid)
        .collect();
    if !wine.is_empty() {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&wine),
            false,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_cmd(UpdateKind::Always),
        );
    }
    let mut newest: HashMap<String, u64> = HashMap::new();
    for (pid, process) in system.processes() {
        if Some(*pid) == own || !mine(process) {
            continue;
        }
        let path = if super::is_wine(process) {
            super::wine_executable(process.cmd())
        } else {
            (process.exe())
                .filter(|exe| own_exe.as_deref() != Some(*exe))
                .map(|exe| exe.to_string_lossy().into_owned())
        };
        let Some(path) = path.filter(|path| !super::ignored(path) && !system_program(path)) else {
            continue;
        };
        let started = process.start_time();
        let kept = newest.entry(path).or_insert(started);
        *kept = (*kept).max(started);
    }
    let mut programs: Vec<(u64, String)> = (newest.into_iter())
        .map(|(path, started)| (started, path))
        .collect();
    programs.sort_unstable_by(|a, b| b.cmp(a));
    programs.truncate(limit);
    (programs.into_iter())
        .map(|(_, path)| Program {
            name: file_name(&path).to_owned(),
            path,
        })
        .collect()
}

/// The last part of a path, Windows' or the system's.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A program of the system's own, which no game is: the system's libraries
/// and services on Linux and macOS, and Windows' own folder.
fn system_program(path: &str) -> bool {
    const SYSTEM: [&str; 9] = [
        "/usr/lib/",
        "/usr/lib64/",
        "/usr/libexec/",
        "/usr/sbin/",
        "/lib/",
        "/lib64/",
        "/sbin/",
        "/System/",
        "/Library/Apple/",
    ];
    let windows = path.replace('\\', "/").to_ascii_lowercase();
    SYSTEM.iter().any(|folder| path.starts_with(folder))
        || windows.starts_with("c:/windows/")
        || windows.starts_with("//?/c:/windows/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_programs_and_paths_are_told_apart() {
        assert!(system_program("/usr/lib/systemd/systemd"));
        assert!(system_program("/usr/libexec/gvfsd"));
        assert!(system_program("C:\\Windows\\System32\\svchost.exe"));
        assert!(!system_program("/usr/bin/supertuxkart"));
        assert!(!system_program("/home/me/Games/game.x86_64"));
        assert!(!system_program("C:\\Games\\Game.exe"));
        assert_eq!(file_name("C:\\Games\\Game.exe"), "Game.exe");
        assert_eq!(file_name("/opt/game/run"), "run");
    }

    #[test]
    fn lists_this_users_programs_but_not_itself() {
        let programs = running(500);
        let own = std::env::current_exe().unwrap();
        assert!(
            programs
                .iter()
                .all(|program| std::path::Path::new(&program.path) != own),
            "not the program asking"
        );
        let mut paths: Vec<_> = programs.iter().map(|program| &program.path).collect();
        let listed = paths.len();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), listed, "each once");
        assert!(running(1).len() <= 1);
    }
}
