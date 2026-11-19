//! The command line, as `src/main.rs` of the Slint app takes it:
//!
//!     anomi [--preview] [--data-dir PATH] [--renderer auto|gpu|software] [--adapter ID]
//!           [--background] [LINK]
//!     anomi --version | --help
//!
//! and `--stream-worker`, which the core starts this same binary with for Go
//! Live and video, and `--list-adapters`, with which the window lists the
//! graphics adapters in a process of its own (`crate::gpu`). A link comes as
//! one word, or after `--open-link` as the desktop hands it; the `session`
//! commands stay with the Slint binary until Q05
//! (docs/iced/tasks/integration.md).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use yapfast_core::runtime::BackgroundRuntime;

use crate::gpu::Override;
use crate::layout::Layout;

/// What a start asks for.
#[derive(Debug, Default)]
pub struct Start {
    pub preview: bool,
    pub data_dir: Option<PathBuf>,
    /// Started at sign-in: the app goes to the tray with the saved session,
    /// where there's a tray and a session.
    pub background: bool,
    /// A Discord link to open (`platform::links`).
    pub link: Option<String>,
    /// What draws the window this run, over Settings' choice
    /// (`--renderer`, `--adapter`).
    pub renderer: Option<Override>,
}

pub fn main() -> Result<()> {
    fix_mmap_threshold();
    // A stream worker starts nothing else: no window, runtime or TLS.
    #[cfg(feature = "discord")]
    if std::env::args().nth(1).as_deref() == Some(yapfast_core::stream_worker::ARGUMENT) {
        yapfast_core::logging::init_to_stderr();
        return yapfast_core::stream_worker::run();
    }
    // Nor does listing the graphics adapters, which loads every Vulkan
    // driver into this process, so the window's never does.
    if std::env::args().nth(1).as_deref() == Some(crate::gpu::LIST_ADAPTERS) {
        return crate::gpu::print_adapters();
    }
    let start = match parse(std::env::args().skip(1))? {
        Parsed::Run(start) => start,
        Parsed::Print(text) => {
            println!("{text}");
            return Ok(());
        }
    };
    crate::logging::init();
    yapfast_core::init_tls()?;
    let root = match &start.data_dir {
        Some(directory) => directory.clone(),
        None => yapfast_core::default_directory()?,
    };
    // The layout first, for the cores it chooses: set on this thread before
    // any other starts, every thread after takes them from its own.
    let layout = Layout::read(&root);
    // So is what may draw the window: only its Vulkan driver may load.
    let plan = crate::gpu::prepare(&layout, start.renderer.as_ref());
    crate::run::place_on_cores(&layout);
    // openmls's pool for DAVE, small and named, rather than one thread for
    // each of the processor's.
    yapfast_core::cpu::init_pools();
    let runtime = BackgroundRuntime::new().context("could not start background workers")?;
    crate::run::run(runtime, root, layout, start, plan)
}

pub enum Parsed {
    Run(Start),
    Print(String),
}

pub fn parse(arguments: impl Iterator<Item = String>) -> Result<Parsed> {
    let mut start = Start::default();
    let mut linked = false;
    let (mut renderer, mut adapter) = (None, None);
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--preview" => start.preview = true,
            "--background" => start.background = true,
            "--data-dir" => {
                let path = arguments.next().context("--data-dir needs a path")?;
                start.data_dir = Some(path.into());
            }
            // Over Settings' choice, for this run (`crate::gpu`).
            "--renderer" => {
                let asked = arguments
                    .next()
                    .context("--renderer needs auto, gpu or software")?;
                renderer = Some(match asked.as_str() {
                    "auto" => Override::Auto,
                    "gpu" => Override::Gpu,
                    "software" => Override::Software,
                    _ => bail!("--renderer takes auto, gpu or software"),
                });
            }
            "--adapter" => {
                let id = arguments
                    .next()
                    .context("--adapter needs an adapter's PCI address, such as 0000:73:00.0")?;
                adapter = Some(crate::gpu::adapter_id(&id));
            }
            // As the desktop hands a link, the way `anomi_desktop::protocol`
            // registers it: whatever follows is the link.
            anomi_desktop::protocol::OPEN_LINK => {
                let link = arguments.next().context("--open-link needs a link")?;
                start.link = written(&link);
            }
            other if link(other) => {
                if std::mem::replace(&mut linked, true) {
                    bail!("specify one Discord link");
                }
                start.link = written(other);
                if start.link.is_none() {
                    tracing::info!("Anomi has no place for the link it was handed");
                }
            }
            "--version" | "-V" => {
                return Ok(Parsed::Print(format!(
                    "{} {} (iced interface)",
                    yapfast_lang::PRODUCT,
                    env!("CARGO_PKG_VERSION")
                )));
            }
            "--help" | "-h" => return Ok(Parsed::Print(HELP.into())),
            other => bail!("unknown argument {other:?}; run anomi --help"),
        }
    }
    start.renderer = match (renderer, adapter) {
        (None | Some(Override::Gpu), Some(id)) => Some(Override::Adapter(id)),
        (Some(_), Some(_)) => bail!("--adapter goes with --renderer gpu, or alone"),
        (renderer, None) => renderer,
    };
    Ok(Parsed::Run(start))
}

const HELP: &str = "Usage: anomi [--preview] [--data-dir PATH] [--renderer auto|gpu|software] [--adapter ID] [--background] [LINK]\n       anomi [--version | --help | --list-adapters]\n\nAnomi's iced interface, beside the Slint one, on the same core and data.\n--preview opens local sample conversations without connecting.\n--data-dir keeps the session and layout.json in PATH.\n--renderer draws the window for this run with Automatic (the integrated graphics, else the processor), a graphics adapter (the one Settings chose, else Automatic's, even after it failed) or the processor, over the choice in Settings.\n--adapter draws on the adapter at this PCI address (0000:73:00.0) for this run.\n--list-adapters prints the graphics adapters as JSON, with whether Vulkan can draw with each.\n--background starts in the system tray with a saved session, as starting at sign-in does.\nLINK opens a Discord server, channel, message, profile or invite link; with Anomi already running, it opens there.";

/// A link given as an argument: anything under `discord:` or `yapfast:`,
/// and Discord's web links this build opens (`link_arguments` in the Slint
/// app's `src/main.rs`).
fn link(argument: &str) -> bool {
    argument.split_once(':').is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("discord") || scheme.eq_ignore_ascii_case("yapfast")
    }) || written(argument).is_some()
}

/// A link as one word, when it names a place this build opens.
fn written(link: &str) -> Option<String> {
    #[cfg(feature = "discord")]
    return yapfast_core::discord::discord_link(link);
    #[cfg(not(feature = "discord"))]
    {
        let _ = link;
        None
    }
}

/// See `fix_mmap_threshold` in the Slint app's `src/main.rs`: a fixed mmap
/// threshold returns decoded pictures' memory as they go.
fn fix_mmap_threshold() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if std::env::var_os("MALLOC_MMAP_THRESHOLD_").is_none()
        && !std::env::var("GLIBC_TUNABLES")
            .is_ok_and(|tunables| tunables.contains("glibc.malloc.mmap_threshold"))
    {
        const M_MMAP_THRESHOLD: std::ffi::c_int = -3;
        unsafe extern "C" {
            fn mallopt(parameter: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
        }
        // SAFETY: this runs first in main, before any other thread starts.
        unsafe { mallopt(M_MMAP_THRESHOLD, 128 * 1024) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> impl Iterator<Item = String> {
        text.split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn takes_what_ui_session_passes() {
        let Ok(Parsed::Run(start)) = parse(args("--renderer software --data-dir /tmp/x")) else {
            panic!("a run");
        };
        assert_eq!(start.data_dir, Some(PathBuf::from("/tmp/x")));
        assert!(!start.preview);
        assert!(parse(args("--renderer vulkan")).is_err());
        assert!(parse(args("--nonsense")).is_err());
        let Ok(Parsed::Run(start)) = parse(args("--background https://discord.com/channels/1/2"))
        else {
            panic!("a run");
        };
        assert!(start.background);
        assert_eq!(
            start.link.as_deref(),
            Some("https://discord.com/channels/1/2")
        );
        assert!(parse(args("discord://a discord://b")).is_err());
    }

    #[test]
    fn the_renderer_and_the_adapter_override_settings_for_a_run() {
        let renderer = |text: &str| match parse(args(text)) {
            Ok(Parsed::Run(start)) => Some(start.renderer),
            _ => None,
        };
        assert_eq!(renderer(""), Some(None));
        assert_eq!(
            renderer("--renderer software"),
            Some(Some(Override::Software))
        );
        assert_eq!(renderer("--renderer auto"), Some(Some(Override::Auto)));
        let radeon = Some(Some(Override::Adapter("pci:0000:73:00.0".into())));
        assert_eq!(renderer("--adapter 0000:73:00.0"), radeon);
        assert_eq!(renderer("--adapter 0000:73:00.0 --renderer gpu"), radeon);
        assert_eq!(renderer("--renderer software --adapter 0000:73:00.0"), None);
        assert_eq!(renderer("--adapter"), None);
    }
}
