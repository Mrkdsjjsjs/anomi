use std::io::Write;
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::ptr::NonNull;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use super::list::{Listed, read_list};
use super::thread::STOP_WITHIN;
use super::*;

#[test]
fn a_closed_stop_wakes_the_wait() {
    let (watched, _source) = std::io::pipe().unwrap();
    let (stop, stopping) = std::io::pipe().unwrap();
    drop(stopping);
    assert_eq!(wait(watched.as_fd(), stop.as_fd()).unwrap(), Woke::Stopped);
}

#[test]
fn something_to_read_wakes_the_wait() {
    let (watched, mut source) = std::io::pipe().unwrap();
    let (stop, _stopping) = std::io::pipe().unwrap();
    source.write_all(b"x").unwrap();
    assert_eq!(wait(watched.as_fd(), stop.as_fd()).unwrap(), Woke::Ready);
}

/// A quit wins over events still to read.
#[test]
fn the_stop_comes_before_what_waits_to_be_read() {
    let (watched, mut source) = std::io::pipe().unwrap();
    let (stop, stopping) = std::io::pipe().unwrap();
    source.write_all(b"x").unwrap();
    drop(stopping);
    assert_eq!(wait(watched.as_fd(), stop.as_fd()).unwrap(), Woke::Stopped);
}

/// winit closed its display, and the socket with it: the thread must not
/// read it, nor give its read back, even as the watch stops.
#[test]
fn a_socket_no_longer_open_reads_as_closed_before_a_stop() {
    let (stop, stopping) = std::io::pipe().unwrap();
    drop(stopping);
    // SAFETY: no file has this number, past any the kernel gives
    // (`fs.nr_open` caps them far below), so it stays closed; `poll` only
    // looks the number up.
    let gone = unsafe { BorrowedFd::borrow_raw(i32::MAX) };
    assert_eq!(wait(gone, stop.as_fd()).unwrap(), Woke::Closed);
}

#[test]
fn a_dropped_list_is_read_as_the_source_writes_it() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let (stop, _stopping) = std::io::pipe().unwrap();
    let writing = std::thread::spawn(move || {
        writer.write_all(b"file:///tmp/a\r\n").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        writer.write_all(b"file:///tmp/b\r\n").unwrap();
    });
    assert_eq!(
        read_list(reader, stop.as_fd()),
        Listed::Whole("file:///tmp/a\r\nfile:///tmp/b\r\n".into())
    );
    writing.join().unwrap();
}

/// A source that never closes its end doesn't hold up a quit during a drop.
#[test]
fn a_source_that_never_finishes_lets_the_watch_stop() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let (stop, stopping) = std::io::pipe().unwrap();
    writer.write_all(b"file:///tmp/a\r\n").unwrap();
    let quitting = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        drop(stopping);
    });
    assert_eq!(read_list(reader, stop.as_fd()), Listed::Stopped);
    quitting.join().unwrap();
    drop(writer);
}

#[test]
fn a_list_past_the_most_read_is_refused() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let (stop, _stopping) = std::io::pipe().unwrap();
    let writing = std::thread::spawn(move || {
        let line = [b'a'; 4096];
        // Until the reader gives up and its end closes.
        while writer.write_all(&line).is_ok() {}
    });
    assert!(matches!(read_list(reader, stop.as_fd()), Listed::Failed(_)));
    writing.join().unwrap();
}

#[test]
fn a_list_that_is_not_text_is_refused() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    let (stop, _stopping) = std::io::pipe().unwrap();
    writer.write_all(&[0xff, 0xfe]).unwrap();
    drop(writer);
    assert!(matches!(read_list(reader, stop.as_fd()), Listed::Failed(_)));
}

/// Dropping the watch waits for the thread to let go, so what it holds on
/// the display goes before the display does.
#[test]
fn dropping_the_watch_waits_for_its_thread_to_let_go() {
    let let_go = Arc::new(AtomicBool::new(false));
    let letting_go = let_go.clone();
    let watch = Watch::spawn(move |stop| {
        let (idle, _source) = std::io::pipe().unwrap();
        assert_eq!(wait(idle.as_fd(), stop.as_fd()).unwrap(), Woke::Stopped);
        // Letting go of the display's objects, slowly.
        std::thread::sleep(Duration::from_millis(20));
        letting_go.store(true, Ordering::Release);
    })
    .unwrap();
    drop(watch);
    assert!(let_go.load(Ordering::Acquire));
}

/// A thread that doesn't stop holds the quit up only so long.
#[test]
fn a_thread_that_does_not_stop_holds_the_quit_up_only_so_long() {
    let watch = Watch::spawn(|_stop| std::thread::sleep(STOP_WITHIN * 5)).unwrap();
    let dropping = Instant::now();
    drop(watch);
    let waited = dropping.elapsed();
    assert!(waited >= STOP_WITHIN, "waited {waited:?}");
    assert!(waited < STOP_WITHIN * 4, "waited {waited:?}");
}

/// A connection as winit's is, on a socket whose other end is the
/// compositor's, which reads nothing; none where libwayland isn't found.
fn connection() -> Option<(Connection, UnixStream)> {
    let (client, compositor) = UnixStream::pair().unwrap();
    let connection = Connection::from_socket(client).ok()?;
    Some((connection, compositor))
}

fn display(connection: &Connection) -> NonNull<std::ffi::c_void> {
    NonNull::new(connection.backend().display_ptr().cast()).unwrap()
}

/// As the app quits: its watch drops while winit's display is still open,
/// the thread lets go of all it had on it, and only then the display
/// closes, which then frees nothing the thread still holds.
#[test]
fn the_watch_lets_go_of_the_display_before_it_closes() {
    let Some((connection, _compositor)) = connection() else {
        return;
    };
    let (reported, heard) = mpsc::channel::<Drag>();
    // SAFETY: the display stays open until after the watch drops, below.
    let watch = unsafe {
        watch(display(&connection), move |drag| {
            let _ = reported.send(drag);
        })
    }
    .unwrap();
    // Let the thread bind and settle into its wait, as it would idle.
    std::thread::sleep(Duration::from_millis(20));
    let dropping = Instant::now();
    drop(watch);
    let waited = dropping.elapsed();
    if waited >= STOP_WITHIN {
        // The thread may still be on the display: closing it under the
        // thread would crash the whole test run rather than fail this.
        std::mem::forget(connection);
        panic!("the watch waited {waited:?} for its thread");
    }
    // `report` went with the thread's state, before the watch was done.
    assert!(matches!(
        heard.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    drop(connection);
}

/// As the compositor goes: the socket closes, and the thread ends on its
/// own, leaving the connection's objects alone; the watch then drops at
/// once, and the display closes after.
#[test]
fn a_broken_connection_ends_the_thread_by_itself() {
    let Some((connection, compositor)) = connection() else {
        return;
    };
    let (reported, heard) = mpsc::channel::<Drag>();
    // SAFETY: the display stays open until after the watch drops, below.
    let watch = unsafe {
        watch(display(&connection), move |drag| {
            let _ = reported.send(drag);
        })
    }
    .unwrap();
    drop(compositor);
    assert!(matches!(
        heard.recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
    let dropping = Instant::now();
    drop(watch);
    assert!(dropping.elapsed() < STOP_WITHIN);
    drop(connection);
}

/// A file that took the socket's number once winit closed it, as one may
/// while a panic unwinds past winit's event loop, isn't the display's: the
/// thread lets go of nothing on it as the watch stops.
#[test]
fn a_file_that_took_the_sockets_number_is_not_the_display() {
    let (client, _compositor) = UnixStream::pair().unwrap();
    let mut client = std::os::fd::OwnedFd::from(client);
    let fd = client.as_raw_fd();
    let socket = Socket { fd, file: file(fd) };
    assert!(socket.open());
    let (reader, _writer) = std::io::pipe().unwrap();
    rustix::io::dup2(&reader, &mut client).unwrap();
    assert!(!socket.open(), "another file under its number");
}
