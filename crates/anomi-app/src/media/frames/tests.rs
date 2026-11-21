//! A worker's frames through the alarm: what each asks of the window, and
//! the slot a frame being presented lies in, never written under it.

use super::*;

use yapfast_core::frames::Writer;

#[test]
fn a_videos_frames_ask_for_the_view_at_the_first_and_each_step_of_its_bar() {
    let frames = Arc::new(Frames::create(4, 2).expect("frames"));
    let path = frames.path().expect("its file");
    let mut writer = Writer::open(&path).expect("the worker's side");
    let (live, clock) = (Live::default(), Arc::new(Clock::default()));
    let alarm = Alarm::default();
    alarm.set(vec![Source::Mapped(
        frames.clone(),
        live.clone(),
        clock.clone(),
    )]);
    assert_eq!(alarm.take(), Found::Nothing, "no frame yet");

    writer.write_timed(2, 1, 40, |rgba| {
        rgba.copy_from_slice(&[1, 2, 3, 255, 4, 5, 6, 255])
    });
    assert_eq!(alarm.take(), Found::View, "the first frame");
    let shown = live.newest().expect("a frame");
    let image::Handle::Rgba { pixels, .. } = &shown else {
        panic!("pixels");
    };
    assert_eq!(
        &pixels[..],
        &[3, 2, 1, 255, 6, 5, 4, 255],
        "BGRA, in the slot"
    );
    assert!(iced_tiny_skia::premultiplied::is_premultiplied(&shown));

    writer.write_timed(2, 1, 80, |rgba| rgba.fill(9));
    assert_eq!(alarm.take(), Found::Redraw, "the same step of the bar");
    writer.write_timed(2, 1, 300, |rgba| rgba.fill(9));
    assert_eq!(
        alarm.take(),
        Found::Nothing,
        "the one before isn't drawn yet"
    );
    drop(live.newest());
    assert_eq!(alarm.take(), Found::View, "drawn: the next, a step on");
    assert_eq!(clock.time(), 250);

    // Not shown, it waits and its worker is told.
    alarm.set(Vec::new());
    assert!(alarm.idle());
    frames.unlink();
}

/// A redraw reads a frame before its present copies the pixels, and the
/// read lets the next frame be taken at once, while that present goes on:
/// the worker then writes however many frames it makes into neither slot,
/// so the frame presented keeps its pixels, BGRA as they were taken. With
/// three slots the take gave the presented frame's slot back, and the
/// second frame after it was written there, as RGBA.
#[test]
fn a_frame_being_presented_is_never_written_under_it() {
    let frames = Arc::new(Frames::create(2, 1).expect("frames"));
    let mut writer = Writer::open(&frames.path().expect("its file")).expect("the worker's side");
    frames.unlink();
    let live = Live::default();
    let alarm = Alarm::default();
    alarm.set(vec![Source::Mapped(
        frames.clone(),
        live.clone(),
        Arc::new(Clock::default()),
    )]);

    writer.write(2, 1, |rgba| {
        rgba.copy_from_slice(&[1, 2, 3, 255, 1, 2, 3, 255])
    });
    assert_eq!(alarm.take(), Found::View, "the first frame");
    // Redraw R reads it; R's present holds the handle.
    let presenting = live.newest().expect("a frame");
    // Read, the next is taken at once, R's present still under way.
    writer.write(2, 1, |rgba| rgba.fill(7));
    assert_eq!(alarm.take(), Found::Redraw, "taken while R presents");
    // More come, while the frame taken isn't read.
    for value in 8..16 {
        writer.write(2, 1, |rgba| rgba.fill(value));
        assert_eq!(alarm.take(), Found::Nothing, "the one taken isn't read");
    }
    let image::Handle::Rgba { pixels, .. } = &presenting else {
        panic!("pixels");
    };
    assert_eq!(
        &pixels[..],
        &[3, 2, 1, 255, 3, 2, 1, 255],
        "the frame presented, as it was taken"
    );
    alarm.set(Vec::new());
}
