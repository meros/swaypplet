//! Fragmented MP4, read as it streams out of ffmpeg: just enough of the
//! box structure to cache the init segment, cut the stream into fragments,
//! and know which fragments a new viewer may start on.
//!
//! The stream is `ftyp` `moov` (the init segment, once), then `moof` `mdat`
//! pairs. A viewer that joins late gets the init segment and then the next
//! fragment whose first sample is a sync sample (a keyframe): a decoder
//! cannot start anywhere else. Whether the first sample is a sync sample is
//! in its sample flags, which come from the first place that states them:
//! the `trun`'s own per-sample flags, its `first_sample_flags`, the
//! `tfhd`'s default, or the `trex` default in the `moov` (ISO/IEC 14496-12,
//! 8.8.3, 8.8.7, 8.8.8). Bit 16 of the flags is `sample_is_non_sync_sample`.

/// One box: its type and its whole bytes (header included).
pub fn boxes(buf: &[u8]) -> impl Iterator<Item = (&[u8; 4], &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let (size, kind, header) = header(buf.get(at..)?)?;
        let b = buf.get(at..at + size)?;
        at += size;
        let kind: &[u8; 4] = kind.try_into().ok()?;
        Some((kind, &b[header..]))
    })
}

/// A box's size, type and header length, from its first bytes. `None`
/// when the bytes do not hold a whole header, or the size is impossible.
fn header(b: &[u8]) -> Option<(usize, &[u8], usize)> {
    let size = u32::from_be_bytes(b.get(0..4)?.try_into().ok()?) as usize;
    let kind = b.get(4..8)?;
    match size {
        1 => {
            let large = u64::from_be_bytes(b.get(8..16)?.try_into().ok()?);
            Some((usize::try_from(large).ok().filter(|s| *s >= 16)?, kind, 16))
        }
        s if s >= 8 => Some((s, kind, 8)),
        // 0 ("to the end of the file") never comes out of a fragmenting
        // muxer, and it cannot be cut out of a stream.
        _ => None,
    }
}

/// The body of the first child `kind` of a box body.
fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(body).find(|(k, _)| *k == kind).map(|(_, b)| b)
}

fn path<'a>(body: &'a [u8], kinds: &[&[u8; 4]]) -> Option<&'a [u8]> {
    kinds.iter().try_fold(body, |b, k| child(b, k))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

const NON_SYNC: u32 = 1 << 16;

/// What the init segment says that the rest needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitInfo {
    /// The MSE codec string, `avc1.PPCCLL` from the `avcC`.
    pub codec: String,
    /// Media time units per second.
    pub timescale: u32,
    /// The `trex` default sample flags.
    pub trex_flags: u32,
}

/// Read a `moov` body (the first video track).
pub fn init_info(moov: &[u8]) -> Option<InitInfo> {
    let trak = child(moov, b"trak")?;
    let mdhd = path(trak, &[b"mdia", b"mdhd"])?;
    let timescale = match mdhd.first()? {
        1 => u32_at(mdhd, 20)?,
        _ => u32_at(mdhd, 12)?,
    };
    let stsd = path(trak, &[b"mdia", b"minf", b"stbl", b"stsd"])?;
    // Version and flags, entry count, then the sample entries.
    let (kind, entry) = boxes(stsd.get(8..)?).next()?;
    // A visual sample entry's fixed fields take 78 bytes before its boxes.
    let avcc = child(entry.get(78..)?, b"avcC")?;
    let codec = format!(
        "{}.{:02x}{:02x}{:02x}",
        std::str::from_utf8(kind).ok()?,
        avcc.get(1)?,
        avcc.get(2)?,
        avcc.get(3)?
    );
    let trex_flags = path(moov, &[b"mvex", b"trex"])
        .and_then(|t| u32_at(t, 20))
        .unwrap_or(0);
    Some(InitInfo {
        codec,
        timescale,
        trex_flags,
    })
}

/// A `moof` body: whether its first sample is a keyframe, and its base
/// decode time (`tfdt`) in media units.
pub fn fragment_info(moof: &[u8], trex_flags: u32) -> Option<(bool, u64)> {
    let traf = child(moof, b"traf")?;
    let tfhd = child(traf, b"tfhd")?;
    let tf_flags = u32_at(tfhd, 0)? & 0x00ff_ffff;
    // track_ID, then the optional fields in flag order.
    let mut at = 8;
    for (bit, len) in [(0x01, 8), (0x02, 4), (0x08, 4), (0x10, 4)] {
        if tf_flags & bit != 0 {
            at += len;
        }
    }
    let tfhd_default = (tf_flags & 0x20 != 0).then(|| u32_at(tfhd, at)).flatten();

    let trun = child(traf, b"trun")?;
    let tr_flags = u32_at(trun, 0)? & 0x00ff_ffff;
    let mut at = 8; // version and flags, sample_count
    if tr_flags & 0x01 != 0 {
        at += 4; // data_offset
    }
    let first = if tr_flags & 0x04 != 0 {
        Some(u32_at(trun, at)?)
    } else if tr_flags & 0x400 != 0 {
        // The first sample's own fields: duration and size come first.
        let skip = [0x100, 0x200]
            .iter()
            .filter(|b| tr_flags & **b != 0)
            .count()
            * 4;
        Some(u32_at(trun, at + skip)?)
    } else {
        None
    };
    let flags = first.or(tfhd_default).unwrap_or(trex_flags);

    let tfdt = child(traf, b"tfdt")?;
    let decode = match tfdt.first()? {
        1 => u64_at(tfdt, 4)?,
        _ => u64::from(u32_at(tfdt, 4)?),
    };
    Some((flags & NON_SYNC == 0, decode))
}

/// One piece of the stream, as it is passed on.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    /// `ftyp` and `moov`, together.
    Init { bytes: Vec<u8>, info: InitInfo },
    /// A `moof` and its `mdat`, together.
    Fragment {
        bytes: Vec<u8>,
        key: bool,
        /// Base decode time in seconds.
        at: f64,
    },
}

/// Cuts a byte stream into [`Piece`]s, whatever the reads' sizes.
#[derive(Default)]
pub struct Splitter {
    buf: Vec<u8>,
    ftyp: Option<Vec<u8>>,
    info: Option<InitInfo>,
    /// A `moof` waiting for its `mdat`, with what it says.
    moof: Option<(Vec<u8>, bool, f64)>,
}

/// A box larger than this is not one ffmpeg writes for a 100 ms fragment;
/// the stream is out of step.
const MAX_BOX: usize = 32 << 20;

impl Splitter {
    /// Add bytes; every piece now whole, in order. `Err` when the stream
    /// cannot be a fragmented MP4.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Piece>, String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some((size, kind, header)) = header(&self.buf) else {
                if self.buf.len() >= 16 {
                    return Err("not an MP4 box".into());
                }
                break;
            };
            if size > MAX_BOX {
                return Err(format!("a box of {size} bytes"));
            }
            if self.buf.len() < size {
                break;
            }
            let kind: [u8; 4] = kind.try_into().map_err(|_| "box type")?;
            let whole: Vec<u8> = self.buf.drain(..size).collect();
            match &kind {
                b"ftyp" => self.ftyp = Some(whole),
                b"moov" => {
                    let info = init_info(&whole[header..])
                        .ok_or("an init segment without a video track")?;
                    let mut bytes = self.ftyp.take().unwrap_or_default();
                    bytes.extend_from_slice(&whole);
                    self.info = Some(info.clone());
                    out.push(Piece::Init { bytes, info });
                }
                b"moof" => {
                    let info = self
                        .info
                        .as_ref()
                        .ok_or("a fragment before the init segment")?;
                    let (key, decode) = fragment_info(&whole[header..], info.trex_flags)
                        .ok_or("a fragment that cannot be read")?;
                    let at = decode as f64 / f64::from(info.timescale.max(1));
                    self.moof = Some((whole, key, at));
                }
                b"mdat" => {
                    if let Some((mut bytes, key, at)) = self.moof.take() {
                        bytes.extend_from_slice(&whole);
                        out.push(Piece::Fragment { bytes, key, at });
                    }
                }
                // styp, sidx, free, …: nothing a viewer needs.
                _ => {}
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn mp4box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    fn full(version: u8, flags: u32, rest: &[u8]) -> Vec<u8> {
        let mut b = (flags | (u32::from(version) << 24)).to_be_bytes().to_vec();
        b.extend_from_slice(rest);
        b
    }

    /// An init segment like ffmpeg's: one avc1 track at 1/15360, trex
    /// default flags "non-sync".
    pub fn init() -> Vec<u8> {
        // Creation and modification times, then the timescale.
        let mut mdhd = vec![0u8; 8];
        mdhd.extend_from_slice(&15360u32.to_be_bytes());
        mdhd.extend_from_slice(&[0; 8]);
        let mut avc1 = vec![0u8; 78];
        avc1.extend(mp4box(b"avcC", &[1, 0x64, 0x00, 0x29, 0xff]));
        let mut stsd = full(0, 0, &1u32.to_be_bytes());
        stsd.extend(mp4box(b"avc1", &avc1));
        let stbl = mp4box(b"stbl", &mp4box(b"stsd", &stsd));
        let mdia = [mp4box(b"mdhd", &full(0, 0, &mdhd)), mp4box(b"minf", &stbl)].concat();
        let trak = mp4box(b"trak", &mp4box(b"mdia", &mdia));
        let mut trex = vec![0u8; 16];
        trex.extend_from_slice(&NON_SYNC.to_be_bytes());
        let mvex = mp4box(b"mvex", &mp4box(b"trex", &full(0, 0, &trex)));
        [
            mp4box(b"ftyp", b"isom\0\0\x02\0"),
            mp4box(b"moov", &[trak, mvex].concat()),
        ]
        .concat()
    }

    /// A fragment; `key` sets `first_sample_flags` to a sync sample, as
    /// ffmpeg writes it, else the trex default (non-sync) applies.
    pub fn fragment(key: bool, decode: u32) -> Vec<u8> {
        let tfhd = mp4box(b"tfhd", &full(0, 0x02_0000, &1u32.to_be_bytes()));
        let tfdt = mp4box(b"tfdt", &full(0, 0, &decode.to_be_bytes()));
        let trun = if key {
            mp4box(
                b"trun",
                &full(0, 0x05, &[0, 0, 0, 3, 0, 0, 0, 0, 0x02, 0, 0, 0]),
            )
        } else {
            mp4box(b"trun", &full(0, 0x01, &[0, 0, 0, 3, 0, 0, 0, 0]))
        };
        let traf = mp4box(b"traf", &[tfhd, tfdt, trun].concat());
        [mp4box(b"moof", &traf), mp4box(b"mdat", &[1, 2, 3])].concat()
    }

    #[test]
    fn the_init_segment_gives_the_codec_and_the_timescale() {
        let bytes = init();
        let moov = boxes(&bytes).find(|(k, _)| *k == b"moov").unwrap().1;
        let info = init_info(moov).unwrap();
        assert_eq!(info.codec, "avc1.640029");
        assert_eq!(info.timescale, 15360);
        assert_eq!(info.trex_flags, NON_SYNC);
    }

    #[test]
    fn a_fragment_says_whether_it_starts_on_a_keyframe() {
        for (key, decode) in [(true, 7680), (false, 9216)] {
            let bytes = fragment(key, decode);
            let moof = boxes(&bytes).next().unwrap().1;
            assert_eq!(
                fragment_info(moof, NON_SYNC),
                Some((key, u64::from(decode)))
            );
        }
    }

    #[test]
    fn the_splitter_cuts_whatever_the_reads_are() {
        let stream = [
            init(),
            fragment(true, 0),
            fragment(false, 1536),
            fragment(true, 7680),
        ]
        .concat();
        for chunk in [1, 7, 100, stream.len()] {
            let mut s = Splitter::default();
            let pieces: Vec<Piece> = stream
                .chunks(chunk)
                .flat_map(|c| s.push(c).unwrap())
                .collect();
            assert_eq!(pieces.len(), 4, "reads of {chunk}");
            let Piece::Init { bytes, info } = &pieces[0] else {
                panic!("init first")
            };
            assert!(bytes.starts_with(&mp4box(b"ftyp", b"isom\0\0\x02\0")));
            assert_eq!(info.codec, "avc1.640029");
            let keys: Vec<(bool, f64)> = pieces[1..]
                .iter()
                .map(|p| match p {
                    Piece::Fragment { key, at, .. } => (*key, *at),
                    _ => panic!(),
                })
                .collect();
            assert_eq!(keys, [(true, 0.0), (false, 0.1), (true, 0.5)]);
        }
    }

    #[test]
    fn garbage_is_refused_not_buffered_forever() {
        let mut s = Splitter::default();
        assert!(
            s.push(&[0, 0, 0, 2, b'x', b'x', b'x', b'x', 0, 0, 0, 0, 0, 0, 0, 0])
                .is_err()
        );
        let mut s = Splitter::default();
        assert!(
            s.push(&fragment(true, 0)).is_err(),
            "a fragment before init"
        );
    }
}
