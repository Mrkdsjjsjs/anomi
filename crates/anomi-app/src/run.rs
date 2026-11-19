//! Runs the program: the daemon iced keeps going without a window, while
//! the app sits in the tray; its fonts, theme and executor; what draws its
//! window; and the processor's cores it runs on.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use yapfast_core::runtime::BackgroundRuntime;

use anomi_theme::{fonts, typography};

use crate::app::App;
use crate::cli::Start;
use crate::executor::Small;
use crate::layout::{CpuChoice, Layout, Saved};
use crate::{gpu, language, pictures};

pub fn run(
    runtime: BackgroundRuntime,
    root: PathBuf,
    layout: Layout,
    start: Start,
    plan: gpu::Plan,
) -> Result<()> {
    // Before any picture is decoded.
    pictures::install();
    // Before the first frame: the fonts with Twemoji ahead of any system
    // emoji font and the name faces at every weight, none copied.
    fonts::install();
    // Another Anomi runs on the folder and took this start over.
    let Some(instance) = crate::platform::instance::claim(&root, &layout, &start) else {
        return Ok(());
    };
    language::start(layout.language());
    // Before the window opens, the GPU the plan chose may start: only here,
    // never in `App::new`, so the app's tests draw on the processor.
    let on_gpu = gpu::begin(plan);
    iced_wgpu::enabled::set_enabled(on_gpu);
    // Discord's cookies and the account as last seen, and beside them the
    // caches: pictures, messages, profiles and the favourite GIFs'
    // thumbnails, which come from there (`src/main.rs`'s `launch`).
    #[cfg(feature = "discord")]
    yapfast_core::keep_discord_files(&root);
    let mut app = App::new(Arc::new(runtime), root, layout);
    app.desktop.instance = instance;
    // iced boots through a closure it may call again; the state goes once.
    let app = Cell::new(Some(app));
    let boot = move || app.take().expect("the program boots once").boot(&start);
    iced::daemon(boot, App::update, App::view)
        // First: it sets every setting, the font below included.
        .settings(gpu::settings())
        .title(App::title)
        .theme(App::theme)
        // Moved as the person's font changes, so every pixel is drawn again.
        .style(|app: &App, theme| app.shown.settings.kept.repaint.style(theme))
        .scale_factor(App::scale_factor)
        .subscription(App::subscription)
        .font(typography::UI)
        .executor::<Small>()
        .run()
        .map_err(|error| anyhow!("the window failed: {error}"))
}

/// The core's mode for the cores chosen.
pub(crate) fn cpu_mode(choice: &CpuChoice) -> yapfast_core::cpu::CpuMode {
    use yapfast_core::cpu::CpuMode;
    match choice {
        CpuChoice::Auto => CpuMode::Auto,
        CpuChoice::All => CpuMode::All,
        CpuChoice::Cores(cores) => CpuMode::Cores(cores.clone()),
    }
}

/// Puts the program on the processor's cores the layout chooses
/// (`docs/cpu.md`): every thread it runs now, each thread started after,
/// which takes them from the one that starts it, and the stream workers it
/// starts, which are told. The processor is read again each time, so Auto
/// fits the one there is: as the program starts, before its other threads,
/// and as Settings changes the choice. About 2 ms; nothing after.
pub(crate) fn place_on_cores(saved: &Saved) {
    use yapfast_core::cpu;
    let topology = cpu::detect();
    let mask = cpu::choose(&topology, &cpu_mode(&saved.cores()));
    cpu::apply(mask.as_ref());
    // The workers take the logical processors by their numbers.
    #[cfg(feature = "discord")]
    yapfast_core::stream_worker::set_cpus(mask.map(|mask| mask.iter().collect()));
}
