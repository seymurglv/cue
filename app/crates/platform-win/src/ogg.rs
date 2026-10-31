//! A writer of one Ogg Opus stream: Ogg pages (RFC 3533) carrying Opus as RFC 7845 maps it. The identification header
//! and the comment header each get a page of their own, then come the audio packets, whole, up to a second of sound on
//! a page, and the last page is marked as the end of the stream. Only what CUE's recordings need: one stream, packets
//! that fit in a page. It reads nothing; the tests have a reader of their own.
use std::io::{self, Write};

/// The flags of a page header (RFC 3533 §6) that CUE writes: the first page of the stream, the last one.
const FIRST_PAGE: u8 = 0x02;
const LAST_PAGE: u8 = 0x04;
/// A page has room for 255 lacing values, so a packet takes up to 255 × 255 − 1 bytes.
const MAX_LACING: usize = 255;
/// An audio page is written after a second of sound (48 kHz samples): a player seeks by pages, and the page headers
/// stay about 4% of a recording at 16 kbit/s.
const PAGE_SAMPLES: u64 = 48_000;
/// The vendor string of the comment header: who wrote the stream.
const VENDOR: &[u8] = b"CUE";

/// Ogg's CRC-32 (RFC 3533 §6): polynomial 0x04C11DB7, most significant bit first, initial value 0, no final XOR.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |crc, &b| (crc << 8) ^ CRC_TABLE[usize::from((crc >> 24) as u8 ^ b)])
}

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut r = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            bit += 1;
        }
        table[i] = r;
        i += 1;
    }
    table
};

/// One Opus stream being written to `out`, page by page.
pub(crate) struct OggOpus<W: Write> {
    out: W,
    serial: u32,
    /// The number of the next page.
    sequence: u32,
    /// The lacing values and the bytes of the packets of the page being filled.
    lacing: Vec<u8>,
    body: Vec<u8>,
    /// The samples (48 kHz) of the packets of the page being filled.
    page_samples: u64,
    /// The granule position after every packet so far: samples at 48 kHz from the start, pre-skip included.
    granule: u64,
    /// The granule position of the last page written.
    written: u64,
}

impl<W: Write> OggOpus<W> {
    /// Starts the stream `serial`: writes the identification header (`OpusHead`: one channel, `pre_skip` samples at
    /// 48 kHz for a player to drop at the start, the `rate` of the sound that was encoded, no gain) and the comment
    /// header (`OpusTags`: the vendor, no comments).
    pub(crate) fn start(out: W, serial: u32, pre_skip: u16, rate: u32) -> io::Result<Self> {
        let mut stream = Self {
            out,
            serial,
            sequence: 0,
            lacing: Vec::new(),
            body: Vec::new(),
            page_samples: 0,
            granule: 0,
            written: 0,
        };
        let mut head = b"OpusHead".to_vec();
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&rate.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family 0: mono or stereo, no mapping table
        stream.header_page(FIRST_PAGE, &head)?;
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
        tags.extend_from_slice(VENDOR);
        tags.extend_from_slice(&0u32.to_le_bytes()); // no comments
        stream.header_page(0, &tags)?;
        Ok(stream)
    }

    /// Adds one audio packet that decodes to `samples` samples at 48 kHz. The page being filled is written first when
    /// it holds a second of sound or has no room for the packet.
    pub(crate) fn packet(&mut self, packet: &[u8], samples: u64) -> io::Result<()> {
        let values = lacing_values(packet.len())?;
        if !self.lacing.is_empty() && (self.lacing.len() + values > MAX_LACING || self.page_samples >= PAGE_SAMPLES) {
            self.flush(0, self.granule)?;
        }
        lace(&mut self.lacing, packet.len());
        self.body.extend_from_slice(packet);
        self.page_samples += samples;
        self.granule += samples;
        Ok(())
    }

    /// Ends the stream with the page being filled, marked as the last one, at the granule position `end`: pre-skip and
    /// the samples of the sound, at 48 kHz. `end` may be less than what the packets decode to — a player drops the rest
    /// (RFC 7845 §4.4) — but not more, and not less than the page before it. Gives `out` back.
    pub(crate) fn finish(mut self, end: u64) -> io::Result<W> {
        if self.lacing.is_empty() {
            return Err(invalid("an Ogg Opus stream ends on a page with sound"));
        }
        if end > self.granule || end < self.written {
            return Err(invalid("the end of the stream is outside its last page"));
        }
        self.flush(LAST_PAGE, end)?;
        Ok(self.out)
    }

    /// A header packet on a page of its own, at granule position 0.
    fn header_page(&mut self, flags: u8, packet: &[u8]) -> io::Result<()> {
        lacing_values(packet.len())?;
        lace(&mut self.lacing, packet.len());
        self.body.extend_from_slice(packet);
        self.flush(flags, 0)
    }

    /// Writes the page being filled with `flags` at the granule position `granule`, and starts an empty one.
    fn flush(&mut self, flags: u8, granule: u64) -> io::Result<()> {
        let mut page = Vec::with_capacity(27 + self.lacing.len() + self.body.len());
        page.extend_from_slice(b"OggS");
        page.push(0); // version
        page.push(flags);
        page.extend_from_slice(&granule.to_le_bytes());
        page.extend_from_slice(&self.serial.to_le_bytes());
        page.extend_from_slice(&self.sequence.to_le_bytes());
        page.extend_from_slice(&[0; 4]); // the CRC, computed over the page with these four bytes at 0
        page.push(self.lacing.len() as u8);
        page.extend_from_slice(&self.lacing);
        page.extend_from_slice(&self.body);
        let crc = crc32(&page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        self.out.write_all(&page)?;
        self.sequence += 1;
        self.lacing.clear();
        self.body.clear();
        self.page_samples = 0;
        self.written = granule;
        Ok(())
    }
}

/// How many lacing values a packet of `len` bytes takes: one per 255 bytes, then one below 255 (0 when `len` is a
/// multiple of 255). A packet that does not fit in a page is refused: CUE's packets never span pages.
fn lacing_values(len: usize) -> io::Result<usize> {
    let values = len / 255 + 1;
    if values > MAX_LACING {
        return Err(invalid("an Opus packet is too large for an Ogg page"));
    }
    Ok(values)
}

/// Appends the lacing values of a packet of `len` bytes.
fn lace(lacing: &mut Vec<u8>, len: usize) {
    lacing.extend(std::iter::repeat_n(255, len / 255));
    lacing.push((len % 255) as u8);
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what)
}

/// A strict reader for the tests: splits a stream into its pages and their packets.
#[cfg(test)]
pub(crate) mod read {
    pub(crate) struct Page {
        pub(crate) flags: u8,
        pub(crate) granule: u64,
        pub(crate) serial: u32,
        pub(crate) sequence: u32,
        pub(crate) lacing: usize,
        pub(crate) packets: Vec<Vec<u8>>,
    }

    /// Every page of `bytes`, in order. Panics on a page without the capture pattern or version 0, a wrong CRC, a
    /// page cut off, or a packet that goes on to the next page (CUE never writes one).
    pub(crate) fn pages(bytes: &[u8]) -> Vec<Page> {
        let mut pages = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let head = &bytes[at..];
            assert!(head.len() >= 27, "a page header is cut off at byte {at}");
            assert_eq!(&head[..4], b"OggS", "no capture pattern at byte {at}");
            assert_eq!(head[4], 0, "page version");
            let count = usize::from(head[26]);
            let lacing = &head[27..27 + count];
            let body_len: usize = lacing.iter().map(|&v| usize::from(v)).sum();
            let len = 27 + count + body_len;
            assert!(head.len() >= len, "a page is cut off at byte {at}");
            let mut zeroed = head[..len].to_vec();
            zeroed[22..26].fill(0);
            let crc = u32::from_le_bytes(head[22..26].try_into().unwrap());
            assert_eq!(super::crc32(&zeroed), crc, "the CRC of page {}", pages.len());
            assert_ne!(lacing.last(), Some(&255), "a packet goes on to the next page");
            let mut packets = Vec::new();
            let (mut packet, mut from) = (Vec::new(), 27 + count);
            for &v in lacing {
                packet.extend_from_slice(&head[from..from + usize::from(v)]);
                from += usize::from(v);
                if v < 255 {
                    packets.push(std::mem::take(&mut packet));
                }
            }
            pages.push(Page {
                flags: head[5],
                granule: u64::from_le_bytes(head[6..14].try_into().unwrap()),
                serial: u32::from_le_bytes(head[14..18].try_into().unwrap()),
                sequence: u32::from_le_bytes(head[18..22].try_into().unwrap()),
                lacing: count,
                packets,
            });
            at += len;
        }
        pages
    }
}

#[cfg(test)]
mod tests {
    use super::read::pages;
    use super::*;

    /// The CRC bit by bit, straight from its definition, against the table.
    fn crc_by_bits(bytes: &[u8]) -> u32 {
        let mut crc = 0u32;
        for &b in bytes {
            crc ^= u32::from(b) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
            }
        }
        crc
    }

    #[test]
    fn crc_is_ogg_crc() {
        // The check value of this CRC (CRC-32/CKSUM without its final XOR).
        assert_eq!(crc32(b"123456789"), 0x89A1_897F);
        assert_eq!(crc32(b""), 0);
        let bytes: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        assert_eq!(crc32(&bytes), crc_by_bits(&bytes));
    }

    #[test]
    fn headers_have_pages_of_their_own() {
        let mut stream = OggOpus::start(Vec::new(), 7, 312, 16_000).unwrap();
        stream.packet(&[1, 2, 3], 960).unwrap();
        let bytes = stream.finish(960).unwrap();
        let pages = pages(&bytes);
        assert_eq!(pages.len(), 3);
        let (head, tags) = (&pages[0], &pages[1]);
        assert_eq!((head.flags, head.granule, head.sequence, head.serial), (FIRST_PAGE, 0, 0, 7));
        assert_eq!(head.packets.len(), 1);
        let h = &head.packets[0];
        assert_eq!(h.len(), 19);
        assert_eq!(&h[..8], b"OpusHead");
        assert_eq!((h[8], h[9]), (1, 1), "version 1, one channel");
        assert_eq!(u16::from_le_bytes([h[10], h[11]]), 312);
        assert_eq!(u32::from_le_bytes(h[12..16].try_into().unwrap()), 16_000);
        assert_eq!((i16::from_le_bytes([h[16], h[17]]), h[18]), (0, 0), "no gain, mapping family 0");
        assert_eq!((tags.flags, tags.granule, tags.sequence, tags.serial), (0, 0, 1, 7));
        assert_eq!(tags.packets, vec![b"OpusTags\x03\0\0\0CUE\0\0\0\0".to_vec()]);
        assert_eq!((pages[2].flags, pages[2].granule, pages[2].sequence), (LAST_PAGE, 960, 2));
        assert_eq!(pages[2].packets, vec![vec![1, 2, 3]]);
    }

    /// Packets of every awkward length come back whole: 0, one under, at and over a lacing value.
    #[test]
    fn packets_round_trip() {
        let lens = [0usize, 1, 254, 255, 256, 509, 510, 511, 1275];
        let mut stream = OggOpus::start(Vec::new(), 1, 0, 16_000).unwrap();
        let sent: Vec<Vec<u8>> = lens.iter().map(|&n| (0..n).map(|i| (i * 7 + n) as u8).collect()).collect();
        for p in &sent {
            stream.packet(p, 960).unwrap();
        }
        let bytes = stream.finish(960 * lens.len() as u64).unwrap();
        let got: Vec<Vec<u8>> = pages(&bytes)[2..].iter().flat_map(|p| p.packets.clone()).collect();
        assert_eq!(got, sent);
    }

    /// A page holds a second of sound; pages are numbered in order, their granule positions count the packets
    /// completed on them, and only the last is marked so.
    #[test]
    fn audio_pages_hold_a_second() {
        let mut stream = OggOpus::start(Vec::new(), 1, 312, 16_000).unwrap();
        for _ in 0..120 {
            stream.packet(&[0; 40], 960).unwrap();
        }
        let bytes = stream.finish(312 + 120 * 960 - 500).unwrap();
        let pages = pages(&bytes);
        let audio = &pages[2..];
        assert_eq!(audio.iter().map(|p| p.packets.len()).collect::<Vec<_>>(), [50, 50, 20]);
        assert_eq!(audio.iter().map(|p| p.granule).collect::<Vec<_>>(), [48_000, 96_000, 312 + 120 * 960 - 500]);
        assert!(pages.iter().enumerate().all(|(i, p)| p.sequence == i as u32));
        assert_eq!(pages.iter().filter(|p| p.flags & LAST_PAGE != 0).count(), 1);
        assert_eq!(pages.last().unwrap().flags, LAST_PAGE);
    }

    /// Large packets go on a new page before the 255 lacing values run out.
    #[test]
    fn a_page_never_overflows_its_lacing() {
        let mut stream = OggOpus::start(Vec::new(), 1, 0, 16_000).unwrap();
        for _ in 0..70 {
            stream.packet(&[9; 1000], 120).unwrap(); // 4 lacing values each, 2.5 ms: far from a second
        }
        let bytes = stream.finish(70 * 120).unwrap();
        let pages = pages(&bytes);
        assert!(pages.iter().all(|p| p.lacing <= MAX_LACING));
        assert_eq!(pages[2..].iter().map(|p| p.packets.len()).collect::<Vec<_>>(), [63, 7]);
        assert_eq!(pages[2].lacing, 252);
    }

    #[test]
    fn a_wrong_end_or_packet_is_refused() {
        let empty = OggOpus::start(Vec::new(), 1, 0, 16_000).unwrap();
        assert!(empty.finish(0).is_err(), "no sound at all");
        let mut stream = OggOpus::start(Vec::new(), 1, 0, 16_000).unwrap();
        stream.packet(&[0; 10], 960).unwrap();
        assert!(stream.packet(&vec![0; 255 * 255], 960).is_err(), "a packet larger than a page");
        assert!(stream.finish(961).is_err(), "more samples than the packets hold");
        let mut stream = OggOpus::start(Vec::new(), 1, 0, 16_000).unwrap();
        for _ in 0..51 {
            stream.packet(&[0; 10], 960).unwrap();
        }
        assert!(stream.finish(47_999).is_err(), "before the page written before it");
    }
}
