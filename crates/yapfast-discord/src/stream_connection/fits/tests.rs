use super::*;
use crate::rtp::{Packetizer, parse};

/// An OBU with its size.
fn obu(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut obu = vec![kind << 3 | 0x02, body.len() as u8];
    obu.extend_from_slice(body);
    obu
}

/// A keyframe and a few frames after it in `codec`, large enough to go in
/// fragments, as each codec's own sender writes them.
fn frames(codec: Codec) -> Vec<(Vec<u8>, bool)> {
    let sized = |mut frame: Vec<u8>, size: usize, fill: u8| {
        frame.resize(size, fill);
        frame
    };
    let mut frames = vec![];
    for index in 0..6 {
        let key = index == 0;
        let size = if key { 4000 } else { 600 + index * 300 };
        let frame = match codec {
            Codec::H264 => {
                let mut frame = if key {
                    vec![
                        0, 0, 0, 1, 0x67, 0x42, 0, 0x1f, 0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80,
                    ]
                } else {
                    vec![]
                };
                frame.extend_from_slice(&[0, 0, 0, 1, if key { 0x65 } else { 0x41 }]);
                sized(frame, size, 0x9a)
            }
            Codec::H265 => {
                let mut frame = if key {
                    vec![
                        0, 0, 0, 1, 0x40, 0x01, 0x0c, 0, 0, 0, 1, 0x42, 0x01, 0x01, 0, 0, 0, 1,
                        0x44, 0x01, 0xc0,
                    ]
                } else {
                    vec![]
                };
                frame.extend_from_slice(&[0, 0, 0, 1, if key { 0x26 } else { 0x02 }, 0x01]);
                sized(frame, size, 0xaf)
            }
            Codec::Av1 => {
                let mut frame = if key {
                    obu(1, &[0, 0, 0, 0x2a])
                } else {
                    vec![]
                };
                // A frame OBU too large for its one-byte size: as several.
                for _ in 0..size / 120 {
                    frame.extend_from_slice(&obu(6, &[0x5d; 120]));
                }
                frame
            }
            Codec::Vp8 => {
                let tag = if key { 0x10 } else { 0x11 };
                sized(vec![tag, 0x02, 0x00, 0x9d, 0x01, 0x2a], size, 0x33)
            }
        };
        frames.push((frame, key));
    }
    frames
}

/// The payloads of `codec`'s packets for [`frames`], again and again.
fn payloads(codec: Codec, count: usize) -> Vec<Vec<u8>> {
    let mut packetizer = Packetizer::new(5, codec);
    let mut payloads = vec![];
    for (index, (frame, key)) in frames(codec).iter().cycle().enumerate() {
        for packet in packetizer.packets(frame, index as u32 * 3000, *key) {
            payloads.push(parse(&packet).unwrap().payload.to_vec());
        }
        if payloads.len() >= count {
            break;
        }
    }
    payloads.truncate(count);
    payloads
}

/// Every packet of a codec fits it, and almost none of them another's;
/// but each of VP8's reads as an AV1 element carrying on, which only a
/// window tells apart: no element starts in it.
#[test]
fn each_codecs_packets_fit_it_alone() {
    for codec in Codec::ALL {
        let payloads = payloads(codec, 200);
        for other in Codec::ALL {
            let fitting = payloads
                .iter()
                .filter(|payload| fits(other, payload))
                .count();
            if other == codec {
                assert_eq!(fitting, payloads.len(), "{codec:?}'s own");
            } else if (codec, other) != (Codec::Vp8, Codec::Av1) {
                assert!(
                    fitting * 10 < payloads.len(),
                    "{codec:?} read as {other:?}: {fitting} fit"
                );
            }
        }
    }
}

/// Reads `codec`'s packets sent as `payload_type` by a viewer that decodes
/// `decodes`: the codec each was put together in, and what's unreadable
/// at the end.
fn read_as(
    codec: Codec,
    payload_type: u8,
    decodes: &[Codec],
) -> (Vec<Option<Codec>>, Option<Unread>) {
    let mut reading = Reading::default();
    let read = payloads(codec, 3 * WINDOW as usize)
        .iter()
        .map(|payload| reading.codec(payload_type, payload, decodes, true))
        .collect();
    (read, reading.unreadable())
}

#[test]
fn a_payload_type_is_read_as_the_codec_its_packets_are_in() {
    let all = [Codec::Av1, Codec::H265, Codec::H264, Codec::Vp8];
    let window = WINDOW as usize;
    // Named rightly: as named, from the first.
    for codec in Codec::ALL {
        let (read, unreadable) = read_as(codec, codec.payload_types().0, &all);
        assert!(read.iter().all(|&read| read == Some(codec)), "{codec:?}");
        assert_eq!(unreadable, None);
    }
    // H265 under H264's number, and under one this viewer never offered:
    // as named, then nothing, until a window shows what they're in.
    let (read, unreadable) = read_as(Codec::H265, rtp::H264, &all);
    assert!(
        read[..window - 1]
            .iter()
            .all(|&read| read == Some(Codec::H264))
    );
    assert!(
        read[window - 1..]
            .iter()
            .all(|&read| read == Some(Codec::H265))
    );
    assert_eq!(unreadable, None);
    for codec in Codec::ALL {
        let (read, _) = read_as(codec, 110, &all);
        assert!(read[..window - 1].iter().all(Option::is_none));
        assert!(
            read[window - 1..].iter().all(|&read| read == Some(codec)),
            "{codec:?}"
        );
    }
    // A codec this viewer doesn't decode: unreadable, and nothing is put
    // together that would ask for keyframes.
    let (read, unreadable) = read_as(Codec::Vp8, 110, &[Codec::H264, Codec::H265]);
    assert!(read.iter().all(Option::is_none));
    assert_eq!(unreadable, Some((110, None)));
    let (read, unreadable) = read_as(Codec::H265, rtp::H265, &[Codec::H264, Codec::Vp8]);
    assert!(
        read[window - 1..]
            .iter()
            .all(|&read| read == Some(Codec::H265)),
        "named, it fits"
    );
    assert_eq!(unreadable, None);
    let (read, unreadable) = read_as(Codec::Av1, rtp::H264, &[Codec::H264, Codec::Vp8]);
    assert!(read[window - 1..].iter().all(Option::is_none));
    assert_eq!(unreadable, Some((rtp::H264, Some(Codec::H264))));
}

use crate::rtp;
