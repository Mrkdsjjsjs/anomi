//! Video frames from the stream worker to the UI, through a memory-mapped
//! file both processes open, in four slots: the worker fills its back slot
//! and swaps it into the middle; the UI takes the middle when it's newer
//! than what it shows, and keeps two, the frame it shows (its front) and
//! the one shown before it. A take gives the worker that one, not the
//! front: the window reads a frame before the present that copies its
//! pixels, and takes the next meanwhile, so the front may still be on its
//! way to the screen (`anomi-app`'s `media::frames`). Neither waits for the
//! other, and neither reads a slot the other is writing.
//!
//! The file lives in the runtime directory, which is memory on Linux, and is
//! removed once the worker has it open, so the memory returns when both
//! processes let go. Windows won't remove a mapped file, so there it's a
//! temporary file, kept in memory while it fits, that goes once the last
//! process closes it.

use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use memmap2::MmapMut;

const MAGIC: u32 = u32::from_le_bytes(*b"TKFR");
/// The header's bytes, before the slots: [`Header`] takes 80 with four.
const HEADER: usize = 128;
const SLOTS: u32 = 4;
/// Set beside the middle slot's index when the worker put a frame there that
/// the UI hasn't taken.
const FRESH: u32 = 4;
const _: () = assert!(SLOTS <= FRESH, "FRESH is clear of every slot's index");
/// [`Header::paused`]'s bits: the UI shows no frames now ([`Frames::pause`]),
/// and its window isn't on screen at all ([`Frames::hide`]).
const PAUSED: u32 = 1;
const HIDDEN: u32 = 2;

/// The header at the start of the file; slots follow it.
#[repr(C)]
struct Header {
    magic: u32,
    capacity: u32,
    /// The middle slot's index, with [`FRESH`].
    middle: AtomicU32,
    /// The UI doesn't show frames now, so the worker may make none
    /// ([`PAUSED`]); and its window is hidden ([`HIDDEN`]).
    paused: AtomicU32,
    /// Each slot's frame size, as width and height in pixels.
    sizes: [[AtomicU32; 2]; SLOTS as usize],
    /// When each slot's frame shows in a video file, in milliseconds.
    times: [AtomicU64; SLOTS as usize],
}

const _: () = assert!(size_of::<Header>() <= HEADER);

/// The line a worker writes among its reports after each frame, while the
/// UI asked to be woken for frames ([`crate::picture::set_frame_wake`]):
/// the UI's side reads the slots without a wake of its own, so the reports'
/// pipe brings it.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub(crate) const FRAME_LINE: &str = "\"frame\"";

static TELL: OnceLock<fn()> = OnceLock::new();

/// Calls `tell` after each frame a [`Writer`] shows, from the thread that
/// wrote it.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub(crate) fn tell_each_frame(tell: fn()) {
    let _ = TELL.set(tell);
}

/// The largest frame `width` by `height` RGBA pixels fit in.
fn capacity(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4
}

struct Mapping {
    map: MmapMut,
    capacity: usize,
}

impl Mapping {
    fn open(file: &File) -> Result<Self> {
        // SAFETY: only Yapfast's own processes open the file, and they touch the
        // slots through the swaps of the middle, never the same slot at once.
        let map = unsafe { MmapMut::map_mut(file) }.context("could not map the video frames")?;
        ensure!(map.len() >= HEADER, "the video frames file is too short");
        let mapping = Self { capacity: 0, map };
        ensure!(mapping.header().magic == MAGIC, "not a video frames file");
        let capacity = mapping.header().capacity as usize;
        ensure!(
            mapping.map.len() >= HEADER + SLOTS as usize * capacity,
            "the video frames file is too short"
        );
        Ok(Self {
            capacity,
            ..mapping
        })
    }

    fn header(&self) -> &Header {
        // SAFETY: the map is at least HEADER bytes, page aligned, and Header
        // is plain integers.
        unsafe { &*self.map.as_ptr().cast::<Header>() }
    }

    fn slot(&self, index: u32) -> *mut u8 {
        // SAFETY: Mapping::open checked the file holds every slot.
        unsafe {
            self.map
                .as_ptr()
                .cast_mut()
                .add(HEADER + index as usize * self.capacity)
        }
    }
}

/// The UI's side: it creates the file and shows the newest frame.
pub struct Frames {
    mapping: Mapping,
    /// The slots the UI keeps.
    shown: Mutex<Shown>,
    name: Mutex<Option<Name>>,
}

/// The two slots the UI keeps: the frame it took last, and the one before,
/// which its window may still present until it reads the frame taken last.
struct Shown {
    front: u32,
    before: u32,
}

/// What keeps the file's name until [`Frames::unlink`].
#[cfg(not(windows))]
type Name = tempfile::TempPath;

#[cfg(windows)]
struct Name {
    path: PathBuf,
    /// Opened to delete the file once every process closes it.
    _file: File,
}

#[cfg(windows)]
impl std::ops::Deref for Name {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

#[cfg(not(windows))]
fn create_file(directory: PathBuf) -> std::io::Result<(File, Name)> {
    let file = tempfile::Builder::new()
        .prefix("yapfast-frames-")
        .tempfile_in(directory)?;
    Ok(file.into_parts())
}

#[cfg(windows)]
fn create_file(directory: PathBuf) -> std::io::Result<(File, Name)> {
    use std::{os::windows::fs::OpenOptionsExt, sync::atomic::AtomicUsize};

    const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;
    const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
    static CREATED: AtomicUsize = AtomicUsize::new(0);
    loop {
        let path = directory.join(format!(
            "yapfast-frames-{}-{}",
            std::process::id(),
            CREATED.fetch_add(1, Ordering::Relaxed)
        ));
        // The worker opens it with the standard library's sharing, which lets
        // it be deleted.
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .attributes(FILE_ATTRIBUTE_TEMPORARY)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&path);
        match opened {
            Ok(file) => {
                let name = file.try_clone()?;
                return Ok((file, Name { path, _file: name }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

impl std::fmt::Debug for Frames {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Frames")
            .field("capacity", &self.mapping.capacity)
            .finish_non_exhaustive()
    }
}

impl Frames {
    /// Creates a file for frames up to `width` by `height` pixels.
    pub fn create(width: u32, height: u32) -> Result<Self> {
        let directory = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|directory| directory.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let (file, name) =
            create_file(directory).context("could not create the video frames file")?;
        let capacity = capacity(width, height);
        file.set_len((HEADER + SLOTS as usize * capacity) as u64)
            .context("could not size the video frames file")?;
        {
            // SAFETY: the file was just created and no one else has it yet.
            let mut map =
                unsafe { MmapMut::map_mut(&file) }.context("could not map the video frames")?;
            map[..4].copy_from_slice(&MAGIC.to_ne_bytes());
            map[4..8].copy_from_slice(&u32::try_from(capacity)?.to_ne_bytes());
            // The worker starts with slot 0, the middle is slot 1, and the
            // UI keeps 2 and 3.
            map[8..12].copy_from_slice(&1u32.to_ne_bytes());
        }
        Ok(Self {
            mapping: Mapping::open(&file)?,
            shown: Mutex::new(Shown {
                front: 2,
                before: 3,
            }),
            name: Mutex::new(Some(name)),
        })
    }

    /// Where the worker opens the file, until [`Self::unlink`].
    pub fn path(&self) -> Option<PathBuf> {
        self.name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_deref()
            .map(Path::to_owned)
    }

    /// Removes the file's name once the worker has it open. On Windows the
    /// name stays until both processes close the file.
    pub fn unlink(&self) {
        self.name
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    /// Tells the worker whether the UI shows frames: while it doesn't (a
    /// preview of one's own stream in a window behind another), the worker
    /// skips the work of making them.
    pub fn pause(&self, paused: bool) {
        self.mark(PAUSED, paused);
    }

    /// Tells the worker whether the UI's window is hidden, not on screen at
    /// all (in the tray, minimised, covered, on another workspace), apart
    /// from whether it shows these frames: a watched stream nobody can see
    /// lets its video stop sooner than one in a window that only rests
    /// (`stream_worker`'s hold).
    pub fn hide(&self, hidden: bool) {
        self.mark(HIDDEN, hidden);
    }

    /// Sets `bit` of the header's word for the worker, or clears it.
    fn mark(&self, bit: u32, on: bool) {
        let word = &self.mapping.header().paused;
        if on {
            word.fetch_or(bit, Ordering::Relaxed);
        } else {
            word.fetch_and(!bit, Ordering::Relaxed);
        }
    }

    /// Calls `show` with the newest frame's width, height and RGBA pixels,
    /// if one arrived since the last call. Its slot stays the UI's until the
    /// second take after this one, which gives it back to the worker.
    pub fn take<T>(&self, show: impl FnOnce(u32, u32, &[u8]) -> T) -> Option<T> {
        self.take_timed(show).map(|(shown, _)| shown)
    }

    /// As [`Self::take`], with when the frame shows in a video file.
    pub fn take_timed<T>(&self, show: impl FnOnce(u32, u32, &[u8]) -> T) -> Option<(T, u64)> {
        let header = self.mapping.header();
        if header.middle.load(Ordering::Acquire) & FRESH == 0 {
            return None;
        }
        let mut shown = self
            .shown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The worker gets the slot shown before the front, not the front:
        // the window reads a frame before the present that copies it, and
        // the next is taken meanwhile, so the front may still be presented.
        let fresh = header.middle.swap(shown.before, Ordering::AcqRel) & !FRESH;
        shown.before = shown.front;
        shown.front = fresh;
        let [width, height] = &header.sizes[fresh as usize];
        let (width, height) = (
            width.load(Ordering::Relaxed),
            height.load(Ordering::Relaxed),
        );
        let length = capacity(width, height).min(self.mapping.capacity);
        // SAFETY: the worker no longer writes this slot: it's the UI's until
        // the second take after this one.
        let pixels = unsafe { std::slice::from_raw_parts(self.mapping.slot(fresh), length) };
        let time = header.times[fresh as usize].load(Ordering::Relaxed);
        Some((show(width, height, pixels), time))
    }
}

/// The worker's side: it fills frames.
pub struct Writer {
    mapping: Mapping,
    back: u32,
}

impl Writer {
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .context("could not open the video frames file")?;
        Ok(Self {
            mapping: Mapping::open(&file)?,
            back: 0,
        })
    }

    /// Whether the UI asked for no frames, as [`Frames::pause`] says.
    pub fn paused(&self) -> bool {
        self.mapping.header().paused.load(Ordering::Relaxed) & PAUSED != 0
    }

    /// Whether the UI's window is hidden, as [`Frames::hide`] says.
    pub fn hidden(&self) -> bool {
        self.mapping.header().paused.load(Ordering::Relaxed) & HIDDEN != 0
    }

    /// Whether nobody sees the frames: the UI asked for none, or its window
    /// is hidden, which it says after each of its updates even where it
    /// didn't come to pause these (frames made while it's in the tray).
    pub fn unseen(&self) -> bool {
        self.mapping.header().paused.load(Ordering::Relaxed) != 0
    }

    /// The largest frame, in pixels, that fits.
    pub fn fits(&self, width: u32, height: u32) -> bool {
        capacity(width, height) <= self.mapping.capacity
    }

    /// Writes a frame with `fill`, which gets its RGBA pixels, then shows it.
    pub fn write(&mut self, width: u32, height: u32, fill: impl FnOnce(&mut [u8])) {
        self.write_timed(width, height, 0, fill);
    }

    /// As [`Self::write`], for a frame of a video file showing at `time_ms`.
    pub fn write_timed(
        &mut self,
        width: u32,
        height: u32,
        time_ms: u64,
        fill: impl FnOnce(&mut [u8]),
    ) {
        assert!(self.fits(width, height), "a frame larger than its slot");
        // SAFETY: the back slot is the worker's alone until it's swapped in:
        // the UI gave it back only at the second take after the one that
        // showed it.
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(self.mapping.slot(self.back), capacity(width, height))
        };
        fill(pixels);
        let header = self.mapping.header();
        let [w, h] = &header.sizes[self.back as usize];
        w.store(width, Ordering::Relaxed);
        h.store(height, Ordering::Relaxed);
        header.times[self.back as usize].store(time_ms, Ordering::Relaxed);
        self.back = header.middle.swap(self.back | FRESH, Ordering::AcqRel) & !FRESH;
        if let Some(tell) = TELL.get() {
            tell();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_pass_from_writer_to_reader() {
        let frames = Frames::create(4, 2).unwrap();
        let path = frames.path().unwrap();
        let mut writer = Writer::open(&path).unwrap();
        frames.unlink();
        assert!(frames.path().is_none());
        if cfg!(not(windows)) {
            assert!(!path.exists());
        }
        assert!(frames.take(|_, _, _| ()).is_none(), "nothing yet");
        assert!(writer.fits(2, 4) && !writer.fits(5, 2));

        writer.write(2, 1, |pixels| pixels.fill(1));
        writer.write(4, 2, |pixels| pixels.fill(2));
        let newest = frames.take(|width, height, pixels| (width, height, pixels.to_vec()));
        assert_eq!(newest, Some((4, 2, vec![2; 32])), "only the newest");
        assert!(frames.take(|_, _, _| ()).is_none(), "taken once");
        assert!(!writer.paused(), "frames are wanted at first");
        frames.pause(true);
        assert!(writer.paused());
        frames.pause(false);
        assert!(!writer.paused());

        // Many frames while the UI holds one: it never sees a partial frame.
        for value in 3..10 {
            writer.write(1, 1, |pixels| pixels.fill(value));
        }
        assert_eq!(
            frames.take(|_, _, pixels| pixels.to_vec()),
            Some(vec![9; 4])
        );

        // A video's frames carry their time.
        writer.write_timed(1, 1, 1234, |pixels| pixels.fill(0));
        assert_eq!(frames.take_timed(|_, _, _| ()), Some(((), 1234)));

        drop((writer, frames));
        assert!(!path.exists(), "gone once both let go");
    }

    /// A take gives the worker the slot shown before the front, never the
    /// front: the window may still present the frame it read last while the
    /// next is taken, so the worker writes over it only after a further
    /// take, however many frames it makes meanwhile.
    #[test]
    fn the_slot_shown_before_the_front_is_the_one_given_back() {
        let frames = Frames::create(1, 1).unwrap();
        let mut writer = Writer::open(&frames.path().unwrap()).unwrap();
        frames.unlink();
        // SAFETY: a slot of the mapping `frames` keeps open, one pixel.
        let read = |address: usize| unsafe { std::slice::from_raw_parts(address as *const u8, 4) };
        let address = |pixels: &[u8]| pixels.as_ptr() as usize;

        writer.write(1, 1, |pixels| pixels.fill(1));
        let first = frames.take(|_, _, pixels| address(pixels)).unwrap();
        writer.write(1, 1, |pixels| pixels.fill(2));
        let second = frames.take(|_, _, pixels| address(pixels)).unwrap();
        assert_ne!(first, second);
        for value in 3..=20 {
            writer.write(1, 1, |pixels| pixels.fill(value));
        }
        assert_eq!(
            read(first),
            [1; 4],
            "shown before the front: still the UI's"
        );
        assert_eq!(read(second), [2; 4], "the front");

        assert_eq!(
            frames.take(|_, _, pixels| pixels.to_vec()),
            Some(vec![20; 4])
        );
        writer.write(1, 1, |pixels| pixels.fill(21));
        writer.write(1, 1, |pixels| pixels.fill(22));
        assert_eq!(read(first), [22; 4], "given back by the take after");
        assert_eq!(read(second), [2; 4], "now shown before the front");
    }

    /// The window hidden is told apart from frames paused: each holds
    /// whatever the other does.
    #[test]
    fn a_hidden_window_is_told_apart_from_a_pause() {
        let frames = Frames::create(1, 1).unwrap();
        let writer = Writer::open(&frames.path().unwrap()).unwrap();
        frames.unlink();
        assert!(!writer.hidden(), "shown at first");
        frames.hide(true);
        assert!(writer.hidden() && !writer.paused(), "frames still wanted");
        assert!(writer.unseen(), "but nobody sees them");
        frames.pause(true);
        assert!(writer.hidden() && writer.paused());
        frames.hide(false);
        assert!(!writer.hidden() && writer.paused(), "a pause outlives it");
        assert!(writer.unseen());
        frames.pause(false);
        assert!(!writer.hidden() && !writer.paused() && !writer.unseen());
    }
}
