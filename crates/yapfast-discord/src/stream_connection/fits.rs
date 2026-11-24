//! Which codec a watched stream's video is really in. A packet's payload
//! type names its codec in the numbering this viewer offered Discord
//! (`rtp::Codec::payload_types`), which Discord's server keeps to; a payload
//! type that names none, or names a codec the packets aren't in, would have
//! the assembler look for keyframes it can never recognise, and ask for
//! them for ever. So the packets' first bytes are held against each codec's
//! payload format as WebRTC senders pack it: a stream's packets all fit its
//! own codec, and almost none another's. DAVE leaves those bytes readable
//! (H264's and H265's unit headers, AV1's OBU headers, VP8's descriptor and
//! frame tag).
//!
//! Every [`WINDOW`] packets with a payload, a payload type whose packets
//! fit the codec it names is read as that; one whose packets almost all
//! misfit it, or that names none, is read in the one codec this viewer
//! decodes that they all fit ([`Verdict::Follow`]); with none or several,
//! it can't be read ([`Verdict::Unknown`]), and its packets keep only their
//! place in the sequence, asking for nothing.

use crate::rtp::{Codec, read_leb128};

/// Packets with a payload counted before a verdict: half a second of a
/// 720p stream, or a few frames of a 1080p keyframe.
const WINDOW: u32 = 50;
/// Of every hundred, at least this many misfit the codec named before
/// another is looked for. A stream in it has none, and bytes of no codec
/// fit any about one time in six at most (H264's headers).
const MISFIT: u32 = 20;

/// What a window's packets say of their payload type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// They're in the codec it names.
    Fits,
    /// They're in this codec, which this viewer decodes, and it names
    /// another or none.
    Follow(Codec),
    /// They're in no codec this viewer decodes, as far as they tell.
    Unknown,
}

/// One payload type's packets in the window, and how many of them misfit
/// each codec, in the order of [`Codec::ALL`].
#[derive(Debug, Default)]
struct Window {
    payload_type: Option<u8>,
    named: Option<Codec>,
    seen: u32,
    misfits: [u32; 4],
    /// Packets that start an AV1 OBU element, and a VP8 frame: a stream in
    /// either has some in a window, and the other's packets may read as
    /// all continuations of it.
    av1_starts: u32,
    vp8_starts: u32,
}

impl Window {
    /// Counts a packet of `payload_type`, which names `named`, carrying
    /// `payload`; at the window's end, says what they were in, among
    /// `decodes`.
    fn count(
        &mut self,
        payload_type: u8,
        named: Option<Codec>,
        payload: &[u8],
        decodes: &[Codec],
    ) -> Option<Verdict> {
        if self.payload_type != Some(payload_type) || self.named != named {
            *self = Self {
                payload_type: Some(payload_type),
                named,
                ..Self::default()
            };
        }
        self.seen += 1;
        for (misfits, codec) in self.misfits.iter_mut().zip(Codec::ALL) {
            *misfits += u32::from(!fits(codec, payload));
        }
        self.av1_starts += u32::from(payload.first().is_some_and(|header| header & 0x80 == 0));
        self.vp8_starts += u32::from(payload.first().is_some_and(|header| header & 0x17 == 0x10));
        if self.seen < WINDOW {
            return None;
        }
        let misfits = |codec: Codec| {
            let at = Codec::ALL.iter().position(|&each| each == codec);
            at.map_or(u32::MAX, |at| self.misfits[at])
        };
        let clean = |codec: Codec| {
            misfits(codec) == 0
                && match codec {
                    Codec::Av1 => self.av1_starts > 0,
                    Codec::Vp8 => self.vp8_starts > 0,
                    Codec::H264 | Codec::H265 => true,
                }
        };
        let verdict = match named {
            Some(codec) if misfits(codec) * 100 < MISFIT * self.seen => Verdict::Fits,
            _ => {
                let mut others = decodes
                    .iter()
                    .copied()
                    .filter(|&codec| Some(codec) != named && clean(codec));
                match (others.next(), others.next()) {
                    (Some(codec), None) => Verdict::Follow(codec),
                    _ => Verdict::Unknown,
                }
            }
        };
        *self = Self {
            payload_type: Some(payload_type),
            named,
            ..Self::default()
        };
        Some(verdict)
    }
}

/// A payload type whose packets are in no codec this viewer decodes, and
/// the codec it names, if any.
pub(super) type Unread = (u8, Option<Codec>);

/// How a watched stream's video packets are read: in the codec their
/// payload type names, or in the one they turn out to be in.
#[derive(Debug, Default)]
pub(super) struct Reading {
    window: Window,
    /// A payload type whose packets are in another codec than it names, or
    /// that names none, and that codec.
    followed: Option<(u8, Codec)>,
    unreadable: Option<Unread>,
    /// Each told once, at warn, a connection.
    told_followed: bool,
    told_unreadable: bool,
}

impl Reading {
    /// The codec a packet of `payload_type` with `payload` is put together
    /// in, if any, among `decodes`. A packet sent the first time counts
    /// towards the verdict; a resend, whose payload type is its media's,
    /// only follows it.
    pub(super) fn codec(
        &mut self,
        payload_type: u8,
        payload: &[u8],
        decodes: &[Codec],
        counts: bool,
    ) -> Option<Codec> {
        let named = match self.followed {
            Some((followed, codec)) if followed == payload_type => Some(codec),
            _ => Codec::from_payload_type(payload_type),
        };
        let verdict = (counts && !payload.is_empty())
            .then(|| self.window.count(payload_type, named, payload, decodes))
            .flatten();
        match verdict {
            Some(Verdict::Fits) => {
                if self
                    .unreadable
                    .is_some_and(|(unread, _)| unread == payload_type)
                {
                    self.unreadable = None;
                }
            }
            Some(Verdict::Follow(codec)) => {
                if !self.told_followed {
                    self.told_followed = true;
                    tracing::warn!(
                        payload_type,
                        named = ?named,
                        codec = ?codec,
                        "a watched stream's video isn't in the codec its payload type names: \
                         read in the one its packets are in"
                    );
                }
                self.followed = Some((payload_type, codec));
                self.unreadable = None;
                return Some(codec);
            }
            Some(Verdict::Unknown) => {
                if !self.told_unreadable {
                    self.told_unreadable = true;
                    tracing::warn!(
                        payload_type,
                        named = ?named,
                        decodes = ?decodes,
                        "a watched stream's video is in no codec this viewer decodes: \
                         it can't be shown"
                    );
                }
                self.unreadable = Some((payload_type, named));
            }
            None => {}
        }
        if self
            .unreadable
            .is_some_and(|(unread, _)| unread == payload_type)
        {
            return None;
        }
        named
    }

    /// The payload type whose packets can't be read, and the codec it
    /// names, while they can't.
    pub(super) fn unreadable(&self) -> Option<Unread> {
        self.unreadable
    }
}

/// Whether `payload` could be a packet of `codec` as a WebRTC sender packs
/// it.
fn fits(codec: Codec, payload: &[u8]) -> bool {
    match codec {
        Codec::H264 => h264(payload),
        Codec::H265 => h265(payload),
        Codec::Av1 => av1(payload),
        Codec::Vp8 => vp8(payload),
    }
}

/// RFC 6184: a unit header with its forbidden bit clear, of a type a
/// sender packs (a slice, SEI, a parameter set, a delimiter, an end or
/// filler: 1, 5 to 12), whole, aggregated (STAP-A, 24) or in fragments
/// (FU-A, 28, its reserved bit clear and not both start and end).
fn h264(payload: &[u8]) -> bool {
    let unit = |header: u8| header & 0x80 == 0 && matches!(header & 0x1f, 1 | 5..=12);
    let Some(&first) = payload.first() else {
        return false;
    };
    if first & 0x80 != 0 {
        return false;
    }
    match first & 0x1f {
        24 => payload.get(3).is_some_and(|&header| unit(header)),
        28 => payload.get(1).is_some_and(|&fragment| {
            fragment & 0x20 == 0 && fragment & 0xc0 != 0xc0 && unit(fragment & 0x1f)
        }),
        _ => unit(first),
    }
}

/// RFC 7798: a unit header with its forbidden bit clear, layer 0 and a
/// temporal id past 0, of a type a sender packs (a slice, 0 to 9 and 16 to
/// 21; a parameter set, delimiter, end, filler or SEI, 32 to 40), whole,
/// aggregated (48) or in fragments (49, not both start and end).
fn h265(payload: &[u8]) -> bool {
    let unit = |kind: u8| matches!(kind, 0..=9 | 16..=21 | 32..=40);
    let header = |first: u8, second: u8| {
        first & 0x80 == 0 && ((first & 1) << 5 | second >> 3) == 0 && second & 7 != 0
    };
    let [first, second, ..] = *payload else {
        return false;
    };
    if !header(first, second) {
        return false;
    }
    match (first >> 1) & 0x3f {
        48 => matches!(payload.get(4..6), Some(&[first, second])
            if header(first, second) && unit((first >> 1) & 0x3f)),
        49 => payload
            .get(2)
            .is_some_and(|&fragment| fragment & 0xc0 != 0xc0 && unit(fragment & 0x3f)),
        kind => unit(kind),
    }
}

/// AOM's payload format: an aggregation header with its last three bits
/// clear, marking a new sequence (N) only where an element starts (Z
/// clear), and an element that starts here beginning with an OBU header:
/// its forbidden and reserved bits clear, of a type there is (1 to 8, 15).
fn av1(payload: &[u8]) -> bool {
    let Some((&header, rest)) = payload.split_first() else {
        return false;
    };
    let continues = header & 0x80 != 0;
    if header & 0x07 != 0 || (continues && header & 0x08 != 0) {
        return false;
    }
    if continues {
        return true;
    }
    // One element alone has no size before it.
    let obu = if (header >> 4) & 0x03 == 1 {
        rest.first()
    } else {
        read_leb128(rest).and_then(|(_, used)| rest.get(used))
    };
    obu.is_some_and(|&obu| obu & 0x81 == 0 && matches!((obu >> 3) & 0x0f, 1..=8 | 15))
}

/// RFC 7741: a payload descriptor with its reserved bits clear and its
/// picture id, which WebRTC senders always send (X and I set, the
/// extension's reserved bits clear), and on a frame's first packet a frame
/// tag of a version there is (0 to 3).
fn vp8(payload: &[u8]) -> bool {
    let [descriptor, extension, ..] = *payload else {
        return false;
    };
    if descriptor & 0x48 != 0 || descriptor & 0x80 == 0 || extension & 0x8f != 0x80 {
        return false;
    }
    let mut at = 2 + if payload.get(2).is_some_and(|&id| id & 0x80 != 0) {
        2
    } else {
        1
    };
    at += usize::from(extension & 0x40 != 0) + usize::from(extension & 0x30 != 0);
    let starts = descriptor & 0x17 == 0x10;
    !starts || payload.get(at).is_some_and(|&tag| (tag >> 1) & 0x07 <= 3)
}

#[cfg(test)]
mod tests;
