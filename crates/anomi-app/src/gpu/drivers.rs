//! Which Vulkan drivers this process may load. Enumerating Vulkan's
//! adapters loads every installed driver into the process, and their
//! libraries stay: NVIDIA's alone triples the window's memory and keeps a
//! thread waking. So as the program starts, before any thread, the loader
//! is told to load only the drivers of the adapter that may draw
//! (`VK_LOADER_DRIVERS_SELECT`, which the Vulkan loader reads as it makes an
//! instance), unless the person chose drivers through the loader's own
//! variables. A driver left out can't load later in the same process: a
//! switch to its adapter waits for the next start. Whether an adapter's
//! driver is installed at all is read from the drivers' manifests, which
//! loads nothing.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

use yapfast_core::adapters::Adapter;

/// The loader's variable that picks drivers by their manifests' names.
pub(super) const SELECT: &str = "VK_LOADER_DRIVERS_SELECT";

/// The loader's variables with which a person picks drivers themselves:
/// theirs stand, and this process selects nothing.
const THEIRS: [&str; 5] = [
    SELECT,
    "VK_LOADER_DRIVERS_DISABLE",
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
];

/// What the start selected; `None` while any driver may load.
static SELECTED: OnceLock<Option<Vec<&'static str>>> = OnceLock::new();

/// The globs naming the manifests of an adapter's Vulkan drivers, by its
/// kernel driver: Mesa's RADV (`radeon_icd.*.json`) or AMDVLK
/// (`amd_icd*.json`), Mesa's Intel drivers (`intel_icd`, `intel_hasvk_icd`),
/// NVIDIA's own (`nvidia_icd.json`) or Mesa's NVK (`nouveau_icd`). None for
/// a kernel driver not known here (and on Windows): nothing is selected
/// for it, and its driver counts as installed.
pub(super) fn globs(card: &Adapter) -> &'static [&'static str] {
    match card.kernel_driver.as_str() {
        "amdgpu" | "radeon" => &["*radeon*", "*amd*"],
        "i915" | "xe" => &["*intel*"],
        "nvidia" => &["*nvidia*"],
        "nouveau" => &["*nouveau*"],
        _ => &[],
    }
}

/// Lets only the drivers `globs` name load into this process, on Linux and
/// unless the person picked drivers already. Called once, before any
/// thread starts.
pub(super) fn select(globs: &[&'static str]) {
    let theirs = THEIRS.iter().any(|name| std::env::var_os(name).is_some());
    let ours = cfg!(target_os = "linux") && !theirs && !globs.is_empty();
    let selected = ours.then(|| globs.to_vec());
    if let Some(globs) = &selected {
        let value = globs.join(",");
        // SAFETY: `cli::main` calls this before the program starts any
        // other thread, so nothing reads the environment meanwhile.
        unsafe { std::env::set_var(SELECT, &value) };
        tracing::info!(drivers = %value, "only these Vulkan drivers may load");
    }
    let _ = SELECTED.set(selected);
}

/// Whether this process set [`SELECT`]: a child that lists every adapter
/// goes without it.
pub(super) fn ours() -> bool {
    matches!(SELECTED.get(), Some(Some(_)))
}

/// Whether the drivers `globs` name may load in this process.
pub(super) fn loadable(globs: &[&'static str]) -> bool {
    match SELECTED.get() {
        Some(Some(selected)) => {
            !globs.is_empty() && globs.iter().all(|glob| selected.contains(glob))
        }
        Some(None) | None => true,
    }
}

/// Whether a Vulkan driver for `card` is installed: a manifest its globs
/// name, where the loader looks. Always on Windows, for a kernel driver not
/// known here, and where the person points the loader at drivers of their
/// own.
pub(super) fn installed(card: &Adapter) -> bool {
    let globs = globs(card);
    let own = THEIRS[2..]
        .iter()
        .any(|name| std::env::var_os(name).is_some());
    if !cfg!(target_os = "linux") || globs.is_empty() || own {
        return true;
    }
    folders().iter().any(|folder| {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            globs.iter().any(|glob| matches(glob, &name))
        })
    })
}

/// Whether a manifest's file name matches a loader's glob, `*` at either
/// end or both.
pub(super) fn matches(glob: &str, name: &str) -> bool {
    let (anywhere_before, rest) = match glob.strip_prefix('*') {
        Some(rest) => (true, rest),
        None => (false, glob),
    };
    let (anywhere_after, word) = match rest.strip_suffix('*') {
        Some(word) => (true, word),
        None => (false, rest),
    };
    match (anywhere_before, anywhere_after) {
        (true, true) => name.contains(word),
        (true, false) => name.ends_with(word),
        (false, true) => name.starts_with(word),
        (false, false) => name == word,
    }
}

/// Where the Vulkan loader looks for drivers' manifests on Linux, in its
/// order: the XDG configuration folders, `/etc`, then the XDG data folders.
fn folders() -> Vec<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = set("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    let data_home = set("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let config = set("XDG_CONFIG_DIRS").unwrap_or_else(|| OsString::from("/etc/xdg"));
    let data =
        set("XDG_DATA_DIRS").unwrap_or_else(|| OsString::from("/usr/local/share:/usr/share"));
    let mut folders: Vec<PathBuf> = Vec::new();
    folders.extend(config_home);
    folders.extend(std::env::split_paths(&config));
    folders.push(PathBuf::from("/etc"));
    folders.extend(data_home);
    folders.extend(std::env::split_paths(&data));
    folders
        .into_iter()
        .map(|folder| folder.join("vulkan/icd.d"))
        .collect()
}
