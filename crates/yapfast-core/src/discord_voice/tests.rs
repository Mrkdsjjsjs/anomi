use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn a_failed_join_shows_the_step_that_failed() {
    let failed =
        anyhow::anyhow!("the voice server didn't answer address discovery, asked 5 times over 5 s");
    assert_eq!(
        join_failed("1", Step::Connecting, failed).to_string(),
        "Couldn't connect to Discord voice: the voice server didn't answer address discovery, \
         asked 5 times over 5 s."
    );
    // The gateway's errors are sentences of their own.
    let failed = anyhow::anyhow!("Discord disconnected while joining voice.");
    assert_eq!(
        join_failed(PRIVATE, Step::Asking, failed).to_string(),
        "Discord disconnected while joining voice."
    );
}

/// Posts to one of the fake Discord's controls.
async fn control(path: &str) -> String {
    let server = yapfast_discord::client::test_server().unwrap();
    let address = server.trim_start_matches("http://").trim_end_matches('/');
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            format!(
                "POST {path} HTTP/1.1\r\nHost: {address}\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    answer
}

/// The tests that call share one fake and its account, and one turns its
/// address discovery off for everyone: they run one at a time.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs testing/fake_discord.ts and YAPFAST_DISCORD_TEST_SERVER"]
async fn a_call_that_cant_connect_says_why() {
    let _alone = ONE_AT_A_TIME.lock().await;
    assert!(
        yapfast_discord::client::test_server().is_some(),
        "set YAPFAST_DISCORD_TEST_SERVER to the fake Discord"
    );
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let discord = Discord::start(yapfast_discord::token::load().unwrap()).unwrap();
    let mut online = discord.snapshots();
    tokio::time::timeout(
        Duration::from_secs(10),
        online.wait_for(|snapshot| snapshot.status == Status::Online),
    )
    .await
    .expect("the fake Discord answers")
    .unwrap();
    assert!(
        control("/_control/voice-discovery?off")
            .await
            .contains("ignored")
    );
    let (_commands, mut received) = mpsc::unbounded_channel();
    let (snapshots, _states) = watch::channel(State::default());
    let mut state = State::default();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        call(
            &discord,
            "1400000000000000100",
            "1400000000000000103",
            &mut state,
            &mut received,
            &snapshots,
            None,
        ),
    )
    .await;
    control("/_control/voice-discovery").await;
    // What the voice worker puts in the state the UI shows.
    let error = result.expect("the call gave up").unwrap_err().to_string();
    assert!(
        error.starts_with(
            "Couldn't connect to Discord voice: the voice server didn't answer address discovery"
        ),
        "{error}"
    );
}

/// A call's audio, unheard.
struct Quiet;

impl yapfast_discord::voice::Listener for Quiet {
    fn tick(&self, _: &[(u64, &[i16])]) {}
    fn disconnected(&self, _: &str) {}
}

/// A screen shared in a call on the fake: the test pattern's cuts
/// (`YAPFAST_STREAM_TEST_PATTERN=cuts`, which `scripts/test-fake-discord`
/// sets), a whole new picture every 8 frames, noise among them, and a
/// keyframe asked for every 40 besides.
struct Cuts {
    /// The bitrate limit, which may pass Discord's 9 Mbit/s; none for Auto.
    kbps: Option<u32>,
    width: u32,
    height: u32,
    fps: u32,
    encode_on: stream_worker::EncodeOn,
    /// How long it stays live.
    live: Duration,
    /// Limits set while live, by how long after it went live.
    changes: Vec<(Duration, u32)>,
    /// The fake reports this many of the stream's last packets lost every
    /// this many milliseconds.
    losses: Option<(u32, u32)>,
}

/// Shares `cuts`: the call's details as the stream worker reported them
/// each second while live, and what the fake received of the stream.
async fn share_cuts(cuts: Cuts) -> (Vec<StreamStats>, serde_json::Value) {
    use std::io::{BufReader, Write};
    assert_eq!(
        std::env::var("YAPFAST_STREAM_TEST_PATTERN").as_deref(),
        Ok("cuts"),
        "scripts/test-fake-discord sets YAPFAST_STREAM_TEST_PATTERN=cuts"
    );
    let fake = yapfast_discord::client::test_server()
        .expect("set YAPFAST_DISCORD_TEST_SERVER to the fake Discord");
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let discord = Discord::start(yapfast_discord::token::load().unwrap()).unwrap();
    let mut online = discord.snapshots();
    tokio::time::timeout(
        Duration::from_secs(10),
        online.wait_for(|snapshot| snapshot.status == Status::Online),
    )
    .await
    .expect("the fake Discord answers")
    .unwrap();
    let (guild, lounge) = ("1400000000000000100", "1400000000000000103");
    let assigned = discord.join(guild, lounge).await.unwrap();
    let _call = tokio::time::timeout(
        Duration::from_secs(10),
        DiscordCall::connect(&assigned, guild, lounge, Arc::new(Quiet)),
    )
    .await
    .expect("timed out connecting to voice")
    .unwrap();
    let server = discord.go_live(guild, lounge).await.unwrap();
    // As Discord's server does, so the stream's target climbs to its cap.
    control("/_control/stream-reports").await;
    if let Some((count, every)) = cuts.losses {
        control(&format!(
            "/_control/stream-nacks?count={count}&every={every}"
        ))
        .await;
    }
    let frames =
        crate::frames::Frames::create(stream_worker::PREVIEW_WIDTH, stream_worker::PREVIEW_HEIGHT)
            .unwrap();
    let (width, height, fps) = (cuts.width, cuts.height, cuts.fps);
    let quality = move |max_kbps| stream_worker::Control::Quality {
        width,
        height,
        fps,
        effort: crate::encode::Effort::Light,
        max_kbps,
    };
    let job = stream_worker::Job::Share {
        source: stream_worker::Source::TestPattern,
        audio: crate::voice::StreamAudio::Off,
        frames: frames.path().unwrap(),
        width: cuts.width,
        height: cuts.height,
        fps: cuts.fps,
        effort: crate::encode::Effort::Light,
        encode_on: cuts.encode_on.clone(),
        max_kbps: cuts.kbps,
        bitrate_unlocked: true,
    };
    let (input, mut feed) = std::io::pipe().unwrap();
    writeln!(feed, "{}", serde_json::to_string(&job).unwrap()).unwrap();
    let send = stream_worker::Control::Send(server.clone());
    writeln!(feed, "{}", serde_json::to_string(&send).unwrap()).unwrap();
    let (reported, reports) = std::sync::mpsc::channel();
    let serving = std::thread::spawn(move || {
        stream_worker::serve(Box::new(BufReader::new(input)), &mut |report| {
            let _ = reported.send(report);
        });
    });
    let details = tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut live: Option<std::time::Instant> = None;
        let mut changes = cuts.changes.into_iter().peekable();
        let mut details = Vec::new();
        while live.is_none_or(|since| since.elapsed() < cuts.live) {
            assert!(
                live.is_some() || std::time::Instant::now() < deadline,
                "the stream didn't go live"
            );
            if let Some(since) = live
                && let Some(&(at, kbps)) = changes.peek()
                && since.elapsed() >= at
            {
                changes.next();
                writeln!(
                    feed,
                    "{}",
                    serde_json::to_string(&quality(Some(kbps))).unwrap()
                )
                .unwrap();
                // By the wall clock, for a capture to tell the limits apart.
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
                eprintln!("limit {kbps} at {} ns", now.unwrap().as_nanos());
            }
            match reports.recv_timeout(Duration::from_millis(50)) {
                Ok(Report::Streaming) => live = live.or(Some(std::time::Instant::now())),
                Ok(Report::Sending(stats)) if live.is_some() => details.push(stats),
                Ok(Report::Ended { error }) => panic!("the worker ended: {error:?}"),
                _ => {}
            }
        }
        writeln!(feed, "stop").unwrap();
        details
    })
    .await
    .unwrap();
    serving.join().unwrap();
    control("/_control/stream-reports?off").await;
    if cuts.losses.is_some() {
        control("/_control/stream-nacks").await;
    }
    let body = discord
        .download(
            &format!("{}/_control/video", fake.trim_end_matches('/')),
            1 << 20,
        )
        .await
        .unwrap();
    let all: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let video = all["received"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|found| found["stream"] == server.stream_key.as_str())
        .cloned()
        .expect("the fake received the stream");
    // The stream ends and the call is left, as when Yapfast quits, so the
    // next test's call doesn't find it still live.
    let _ = tokio::time::timeout(Duration::from_secs(5), discord.shut_down()).await;
    (details, video)
}

/// A shared screen keeps under its ceiling on the wire through the hardest
/// picture there is, the test pattern's cuts, encoded by x264 under a limit
/// of 2 Mbit/s. The fake counts every datagram as it arrives, with its IPv4
/// and UDP headers: no second past the ceiling, and no 100 ms past what the
/// limiter lets out (its depth and its rate for that long), with room for
/// the fake's own timing. The limiter's own count, on the stream's clock,
/// keeps to that exactly; and x264's rate control, whose buffer the limiter
/// sends within, never makes it drop a frame.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs testing/fake_discord.ts and YAPFAST_DISCORD_TEST_SERVER"]
async fn a_shared_screen_keeps_under_its_ceiling_through_cuts() {
    use yapfast_discord::pacer;
    let _alone = ONE_AT_A_TIME.lock().await;
    const CEILING_KBPS: u32 = 2_000;
    let (details, video) = share_cuts(Cuts {
        kbps: Some(CEILING_KBPS),
        width: 640,
        height: 360,
        fps: 30,
        encode_on: stream_worker::EncodeOn::Software,
        live: Duration::from_secs(6),
        changes: Vec::new(),
        losses: None,
    })
    .await;
    let ceiling = u64::from(CEILING_KBPS) * 1000;
    // A datagram's worth besides, for the RTCP that goes at once.
    let (depth, rate, datagram) = (pacer::depth(ceiling), pacer::rate(ceiling), 1500 * 8);
    let wire = &video["wire"];
    let kbps = |bits: u64| bits as f64 / 1000.0;
    let second = wire["peak1sKbps"].as_f64().unwrap();
    assert!(second <= kbps(ceiling), "a second past the ceiling: {wire}");
    assert!(
        second > kbps(ceiling) * 0.8,
        "the cuts kept it busy: {wire}"
    );
    // 20 ms at the ceiling for the fake's timing: a datagram it reads late
    // counts in the window after.
    let most_100ms = (depth + rate / 10 + datagram + ceiling / 50) * 10;
    assert!(
        wire["peak100msKbps"].as_f64().unwrap() <= kbps(most_100ms),
        "100 ms past what the limiter lets out: {wire}"
    );
    assert!(video["frames"].as_u64() >= Some(150), "{video}");
    assert!(video["keyframes"].as_u64() >= Some(4), "{video}");
    let last = details.last().expect("the call's details");
    assert_eq!(last.limit_kbps, CEILING_KBPS);
    assert!(last.target_kbps < CEILING_KBPS * 9 / 10, "{last:?}");
    assert_eq!(last.dropped, 0, "{last:?}");
    for each in &details {
        assert!(
            u64::from(each.peak_kbps) * 1000 <= depth + rate + datagram,
            "{each:?}"
        );
        assert!(
            u64::from(each.peak_100ms_kbps) * 1000 <= (depth + rate / 10 + datagram) * 10,
            "{each:?}"
        );
    }
}

/// Measures a shared screen of the test pattern's cuts on the fake, as
/// "Keeping the ceiling" in docs/discord/go-live.md describes:
/// `YAPFAST_MEASURE_STREAM=kbps,width,height,fps,seconds,encoder[,…]`,
/// with `encoder` `software`, `auto` or an adapter's key, then any of
/// `losses=COUNTxMS` and `AT=KBPS`, a new limit AT seconds after it went
/// live (`kbps` 0 is Auto). It prints the call's details each second and
/// what the fake received. Without the variable it does nothing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs testing/fake_discord.ts and YAPFAST_DISCORD_TEST_SERVER"]
async fn measures_a_shared_screen_on_the_wire() {
    let Ok(measure) = std::env::var("YAPFAST_MEASURE_STREAM") else {
        return;
    };
    let _alone = ONE_AT_A_TIME.lock().await;
    let fields: Vec<&str> = measure.split(',').collect();
    let number = |index: usize| fields[index].parse::<u32>().unwrap();
    let mut cuts = Cuts {
        kbps: Some(number(0)).filter(|&kbps| kbps > 0),
        width: number(1),
        height: number(2),
        fps: number(3),
        encode_on: stream_worker::EncodeOn::parse(fields[5]),
        live: Duration::from_secs(number(4).into()),
        changes: Vec::new(),
        losses: None,
    };
    for extra in &fields[6..] {
        let (key, value) = extra.split_once('=').unwrap();
        if key == "losses" {
            let (count, every) = value.split_once('x').unwrap();
            cuts.losses = Some((count.parse().unwrap(), every.parse().unwrap()));
        } else {
            let at = Duration::from_secs_f64(key.parse().unwrap());
            cuts.changes.push((at, value.parse().unwrap()));
        }
    }
    let (details, video) = share_cuts(cuts).await;
    for each in details {
        println!("details {}", serde_json::to_string(&each).unwrap());
    }
    println!("fake {video}");
}
