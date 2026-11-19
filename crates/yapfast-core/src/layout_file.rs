//! The data folder's `layout.json`, where both clients keep the window's
//! arrangement and the settings that are the window's own, and the one way
//! Yapfast writes its small settings files: `layout.json` and the core's
//! `devices.json` ([`crate::voice::DEVICES`]).

use std::{io, io::Write as _, path::Path};

/// The file, in the data folder.
pub const LAYOUT: &str = "layout.json";

/// Writes `bytes` as `root`'s `layout.json`, as [`write_private`] does.
pub fn write(root: &Path, bytes: &[u8]) -> io::Result<()> {
    write_private(&root.join(LAYOUT), bytes)
}

/// Writes `bytes` as the file at `path`: to a file beside it, its name with
/// `.tmp` added, then over it, so a crash or a power cut mid-write leaves
/// the old file whole rather than a short one that reads as empty.
///
/// On Unix the file is its owner's alone (0600), as the Discord caches are:
/// the settings name the people one mutes, turns down and filters. A file an
/// older build left readable by others is replaced by one that isn't, so it
/// tightens as it's next written. A folder missing on the way is made its
/// owner's alone too (0700); an existing one keeps its permissions. On
/// Windows the data folder is in the user's own LocalAppData, whose
/// permissions the file takes.
///
/// There's no fsync: on btrfs and ext4 a rename over an existing file waits
/// for the new file's data, and `layout.json` is written a second after
/// every run of changes.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let Some(name) = path.file_name() else {
        return Err(io::ErrorKind::InvalidInput.into());
    };
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(folder)?;
    }
    let mut beside = name.to_os_string();
    beside.push(".tmp");
    let beside = path.with_file_name(beside);
    let written = fill(&beside, bytes).and_then(|()| std::fs::rename(&beside, path));
    if written.is_err() {
        // A file cut short is no use to the next write, which starts over.
        let _ = std::fs::remove_file(&beside);
    }
    written
}

/// Puts `bytes` in the file at `beside`, made or emptied, its owner's alone.
fn fill(beside: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(beside)?;
    // A file left by a write cut short keeps the permissions it was made
    // with, which the mode above doesn't change.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_layout_is_replaced_whole() {
        let folder = tempfile::tempdir().expect("a folder");
        super::write(folder.path(), b"{\"a\": 1}").expect("written");
        super::write(folder.path(), b"{\"a\": 2}").expect("written again");
        let kept = std::fs::read(folder.path().join(super::LAYOUT)).expect("kept");
        assert_eq!(kept, b"{\"a\": 2}");
        assert!(!folder.path().join("layout.json.tmp").exists());
    }

    #[test]
    fn any_file_is_replaced_whole_in_a_folder_made_for_it() {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("data").join("devices.json");
        super::write_private(&path, b"{}").expect("written in a new folder");
        super::write_private(&path, b"{\"b\": 2}").expect("written again");
        assert_eq!(std::fs::read(&path).expect("kept"), b"{\"b\": 2}");
        assert!(!folder.path().join("data").join("devices.json.tmp").exists());
        assert!(super::write_private(folder.path().join("..").as_path(), b"{}").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn only_their_owner_reads_them_older_files_included() {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |path: &std::path::Path| {
            std::fs::metadata(path).expect("there").permissions().mode() & 0o777
        };
        let open = |path: &std::path::Path| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
                .expect("opened to everyone");
        };
        let folder = tempfile::tempdir().expect("a folder");
        let root = folder.path().join("data");
        super::write(&root, b"{}").expect("written");
        assert_eq!(mode(&root.join(super::LAYOUT)), 0o600);
        assert_eq!(mode(&root), 0o700, "a folder made for it");
        // As an older build left them: the file, and one a write cut short.
        let older = root.join("devices.json");
        std::fs::write(&older, b"{}").expect("an older file");
        open(&older);
        let cut_short = root.join("devices.json.tmp");
        std::fs::write(&cut_short, b"{\"ha").expect("a file cut short");
        open(&cut_short);
        super::write_private(&older, b"{\"a\": 1}").expect("written");
        assert_eq!(mode(&older), 0o600);
        assert_eq!(std::fs::read(&older).expect("kept"), b"{\"a\": 1}");
        assert!(!cut_short.exists());
        // An existing folder keeps its own permissions.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755))
            .expect("folder opened to everyone");
        super::write(&root, b"{}").expect("written again");
        assert_eq!(mode(&root), 0o755);
        assert_eq!(mode(&root.join(super::LAYOUT)), 0o600);
    }
}
