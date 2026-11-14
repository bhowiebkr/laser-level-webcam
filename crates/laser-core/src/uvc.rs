//! What a USB video class (UVC) camera says it sends.
//!
//! A UVC camera lists its formats and frame sizes in the configuration descriptor it hands the
//! host when it is plugged in: one format descriptor per pixel encoding (uncompressed, MJPEG,
//! frame based), each followed by its frame descriptors (size and frame intervals). That is the
//! camera's own list; anything else the operating system offers is converted from one of these.
//! The isochronous endpoints say how many bytes the link can carry, which is why a large
//! uncompressed size only comes at a low frame rate.

use serde::{Deserialize, Serialize};

use crate::camera::{PixelFormat, VideoMode};

/// The negotiated USB link speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbSpeed {
    Low,
    Full,
    High,
    Super,
}

impl UsbSpeed {
    /// From the hub's speed code (0 low, 1 full, 2 high, 3 super).
    pub fn from_code(code: u8) -> Option<UsbSpeed> {
        Some(match code {
            0 => Self::Low,
            1 => Self::Full,
            2 => Self::High,
            3 => Self::Super,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "USB 1.0 low speed (1.5 Mbit/s)",
            Self::Full => "USB 1.1 full speed (12 Mbit/s)",
            Self::High => "USB 2.0 high speed (480 Mbit/s)",
            Self::Super => "USB 3 SuperSpeed (5 Gbit/s)",
        }
    }

    /// Bus intervals per second: frames at low and full speed, microframes above.
    fn intervals_per_s(self) -> u64 {
        match self {
            Self::Low | Self::Full => 1000,
            Self::High | Self::Super => 8000,
        }
    }
}

/// How a format's pixels are encoded on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UvcEncoding {
    /// Raw pixels, named by the first four bytes of the format GUID (YUY2, NV12, Y800, …).
    Uncompressed { fourcc: String },
    /// Each frame a JPEG, compressed in the camera.
    Mjpeg,
    /// A compressed stream (H.264 and the like), named by its GUID's first four bytes.
    FrameBased { fourcc: String },
}

impl UvcEncoding {
    pub fn name(&self) -> String {
        match self {
            Self::Uncompressed { fourcc } => format!("Uncompressed {fourcc}"),
            Self::Mjpeg => "MJPEG (compressed in the camera)".into(),
            Self::FrameBased { fourcc } => format!("Compressed stream {fourcc}"),
        }
    }

    /// The pixel format this tool reads it as, if it can.
    pub fn pixel_format(&self) -> Option<PixelFormat> {
        match self {
            Self::Uncompressed { fourcc } => match fourcc.trim_end() {
                "YUY2" | "YUYV" => Some(PixelFormat::Yuyv),
                "NV12" => Some(PixelFormat::Nv12),
                "Y800" | "Y8" | "GREY" => Some(PixelFormat::Grey),
                _ => None,
            },
            Self::Mjpeg => Some(PixelFormat::Mjpeg),
            Self::FrameBased { .. } => None,
        }
    }
}

/// One frame size of a format, with the frame rates it comes at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UvcFrame {
    pub width: u16,
    pub height: u16,
    /// Frame rates, fps, from the listed frame intervals, fastest first. A continuous range is
    /// given by its two ends.
    pub fps: Vec<f64>,
    pub default_fps: f64,
    /// The most bits per second the camera says this size needs.
    pub max_bit_rate: u32,
}

/// One format and its frame sizes, in the camera's order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UvcFormat {
    pub index: u8,
    pub encoding: UvcEncoding,
    /// Bits per pixel for raw and stream formats; 0 for MJPEG.
    pub bits_per_pixel: u8,
    pub frames: Vec<UvcFrame>,
}

/// A UVC camera as its descriptors describe it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UvcDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    /// The USB version the device declares, e.g. 0x0200.
    pub usb_version: u16,
    pub speed: Option<UsbSpeed>,
    pub formats: Vec<UvcFormat>,
    /// The most video bytes per second its isochronous endpoints carry, over the alternate
    /// settings; None for a bulk endpoint, which has no fixed share of the bus.
    pub max_bytes_per_s: Option<u64>,
    /// The configuration descriptor as read.
    pub raw: Vec<u8>,
}

const CS_INTERFACE: u8 = 0x24;
const INTERFACE: u8 = 0x04;
const ENDPOINT: u8 = 0x05;
const SS_ENDPOINT_COMPANION: u8 = 0x30;
const CLASS_VIDEO: u8 = 0x0e;
const SUBCLASS_STREAMING: u8 = 0x02;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

fn fourcc(guid: &[u8]) -> String {
    guid.iter().take(4).map(|&c| if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' }).collect()
}

fn fps_of(interval_100ns: u32) -> f64 {
    if interval_100ns == 0 { 0.0 } else { 1e7 / f64::from(interval_100ns) }
}

/// Frame descriptor fields: (width, height, max bit rate, default interval, interval-type byte
/// offset). Frame-based frames have no buffer-size field, so their tail sits four bytes earlier.
fn frame(d: &[u8], frame_based: bool) -> Option<UvcFrame> {
    let width = u16_at(d, 5)?;
    let height = u16_at(d, 7)?;
    let max_bit_rate = u32_at(d, 13)?;
    let (default_at, kind_at, list_at) = if frame_based { (17, 21, 26) } else { (21, 25, 26) };
    let default_fps = fps_of(u32_at(d, default_at)?);
    let kind = *d.get(kind_at)?;
    let mut fps: Vec<f64> = if kind == 0 {
        // Continuous: min, max, step. The two ends are enough to show.
        vec![fps_of(u32_at(d, list_at)?), fps_of(u32_at(d, list_at + 4)?)]
    } else {
        (0..usize::from(kind)).filter_map(|k| u32_at(d, list_at + 4 * k)).map(fps_of).collect()
    };
    fps.sort_by(|a, b| b.total_cmp(a));
    fps.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    Some(UvcFrame { width, height, fps, default_fps, max_bit_rate })
}

impl UvcDevice {
    /// Read the video streaming formats and endpoint bandwidth out of a configuration
    /// descriptor. `device` is the 18-byte device descriptor, if there is one.
    pub fn parse(device: Option<&[u8]>, config: &[u8], speed: Option<UsbSpeed>) -> UvcDevice {
        let mut formats: Vec<UvcFormat> = Vec::new();
        let mut max_bytes_per_s: Option<u64> = None;
        let mut streaming = false;
        // The isochronous endpoint seen last, waiting for a SuperSpeed companion: (bytes per
        // interval, interval exponent).
        let mut pending: Option<(u64, u32)> = None;
        let per_s = speed.unwrap_or(UsbSpeed::High).intervals_per_s();
        let flush = |pending: &mut Option<(u64, u32)>, max: &mut Option<u64>| {
            if let Some((bytes, exp)) = pending.take() {
                let rate = (bytes * per_s) >> exp;
                *max = Some(max.map_or(rate, |m| m.max(rate)));
            }
        };

        let mut i = 0;
        while i + 2 <= config.len() {
            let len = usize::from(config[i]);
            if len < 2 || i + len > config.len() {
                break;
            }
            let d = &config[i..i + len];
            match d[1] {
                INTERFACE if len >= 7 => {
                    flush(&mut pending, &mut max_bytes_per_s);
                    streaming = d[5] == CLASS_VIDEO && d[6] == SUBCLASS_STREAMING;
                }
                ENDPOINT if streaming && len >= 7 => {
                    flush(&mut pending, &mut max_bytes_per_s);
                    let isochronous = d[3] & 0x03 == 0x01;
                    if isochronous && let Some(w) = u16_at(d, 4) {
                        let size = u64::from(w & 0x07ff);
                        let extra = u64::from((w >> 11) & 0x03);
                        let exp = u32::from(d[6].clamp(1, 16)) - 1;
                        pending = Some((size * (1 + extra), exp));
                    }
                }
                SS_ENDPOINT_COMPANION if streaming && len >= 6 => {
                    if let (Some((_, exp)), Some(per_interval)) = (pending, u16_at(d, 4)) {
                        pending = Some((u64::from(per_interval), exp));
                    }
                }
                CS_INTERFACE if streaming && len >= 4 => match d[2] {
                    // VS_FORMAT_UNCOMPRESSED and VS_FORMAT_FRAME_BASED.
                    0x04 | 0x10 if len >= 22 => {
                        let code = fourcc(&d[5..21]);
                        let encoding = if d[2] == 0x04 {
                            UvcEncoding::Uncompressed { fourcc: code }
                        } else {
                            UvcEncoding::FrameBased { fourcc: code }
                        };
                        formats.push(UvcFormat { index: d[3], encoding, bits_per_pixel: d[21], frames: Vec::new() });
                    }
                    // VS_FORMAT_MJPEG.
                    0x06 => {
                        formats.push(UvcFormat {
                            index: d[3],
                            encoding: UvcEncoding::Mjpeg,
                            bits_per_pixel: 0,
                            frames: Vec::new(),
                        });
                    }
                    // VS_FRAME_UNCOMPRESSED, VS_FRAME_MJPEG, VS_FRAME_FRAME_BASED.
                    0x05 | 0x07 | 0x11 => {
                        if let (Some(f), Some(last)) = (frame(d, d[2] == 0x11), formats.last_mut()) {
                            last.frames.push(f);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
            i += len;
        }
        flush(&mut pending, &mut max_bytes_per_s);

        let dev = device.unwrap_or(&[]);
        UvcDevice {
            vendor_id: u16_at(dev, 8).unwrap_or(0),
            product_id: u16_at(dev, 10).unwrap_or(0),
            usb_version: u16_at(dev, 2).unwrap_or(0),
            speed,
            formats,
            max_bytes_per_s,
            raw: config.to_vec(),
        }
    }

    /// The camera sends this mode itself: its encoding, size and frame rate are in the
    /// descriptors. A mode the operating system offers that is not here is converted.
    pub fn sends(&self, m: &VideoMode) -> bool {
        self.formats.iter().any(|f| {
            f.encoding.pixel_format() == Some(m.format)
                && f.frames.iter().any(|fr| {
                    u32::from(fr.width) == m.width
                        && u32::from(fr.height) == m.height
                        && fr.fps.iter().any(|&r| (r - m.fps).abs() < 0.5)
                })
        })
    }

    /// The format a mode comes in on the wire, if the camera sends it.
    pub fn format_of(&self, m: &VideoMode) -> Option<&UvcFormat> {
        self.formats.iter().find(|f| f.encoding.pixel_format() == Some(m.format))
    }

    /// Video bytes per second a raw mode needs on the wire; None for a compressed one, whose
    /// size depends on the picture.
    pub fn raw_bytes_per_s(&self, m: &VideoMode) -> Option<u64> {
        let f = self.format_of(m)?;
        match f.encoding {
            UvcEncoding::Uncompressed { .. } => {
                Some((m.pixels() as f64 * f64::from(f.bits_per_pixel) / 8.0 * m.fps).round() as u64)
            }
            _ => None,
        }
    }

    /// A hex dump of the descriptor, 16 bytes a line.
    pub fn hex(&self) -> String {
        self.raw
            .chunks(16)
            .enumerate()
            .map(|(k, row)| {
                let bytes: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
                format!("{:04x}  {}", k * 16, bytes.join(" "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guid(code: &[u8; 4]) -> [u8; 16] {
        let mut g = [0u8; 16];
        g[..4].copy_from_slice(code);
        g[4..].copy_from_slice(&[0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71]);
        g
    }

    fn frame_desc(subtype: u8, w: u16, h: u16, intervals: &[u32]) -> Vec<u8> {
        let mut d = vec![0, CS_INTERFACE, subtype, 1, 0];
        d.extend(w.to_le_bytes());
        d.extend(h.to_le_bytes());
        d.extend(1_000_000u32.to_le_bytes());
        d.extend(90_000_000u32.to_le_bytes());
        d.extend((u32::from(w) * u32::from(h) * 2).to_le_bytes());
        d.extend(intervals[0].to_le_bytes());
        d.push(intervals.len() as u8);
        for i in intervals {
            d.extend(i.to_le_bytes());
        }
        d[0] = d.len() as u8;
        d
    }

    fn config() -> Vec<u8> {
        let mut c = vec![9, 2, 0, 0, 2, 1, 0, 0x80, 250];
        // Video control interface: not streaming, so nothing below it counts until the next.
        c.extend([9, INTERFACE, 0, 0, 1, CLASS_VIDEO, 0x01, 0, 0]);
        // Streaming interface, alternate 0.
        c.extend([9, INTERFACE, 1, 0, 0, CLASS_VIDEO, SUBCLASS_STREAMING, 0, 0]);
        let mut yuy2 = vec![27, CS_INTERFACE, 0x04, 1, 2];
        yuy2.extend(guid(b"YUY2"));
        yuy2.extend([16, 1, 0, 0, 0, 0]);
        c.extend(&yuy2);
        c.extend(frame_desc(0x05, 1920, 1080, &[2_000_000]));
        c.extend(frame_desc(0x05, 1280, 720, &[1_000_000, 2_000_000]));
        c.extend([11, CS_INTERFACE, 0x06, 2, 1, 1, 1, 0, 0, 0, 0]);
        c.extend(frame_desc(0x07, 1920, 1080, &[333_333, 666_666]));
        // Alternate 1: 3 × 1024 bytes a microframe; alternate 2: 1 × 512.
        c.extend([9, INTERFACE, 1, 1, 1, CLASS_VIDEO, SUBCLASS_STREAMING, 0, 0]);
        c.extend([7, ENDPOINT, 0x81, 0x05, 0x00, 0x14, 1]);
        c.extend([9, INTERFACE, 1, 2, 1, CLASS_VIDEO, SUBCLASS_STREAMING, 0, 0]);
        c.extend([7, ENDPOINT, 0x81, 0x05, 0x00, 0x02, 1]);
        let total = c.len() as u16;
        c[2..4].copy_from_slice(&total.to_le_bytes());
        c
    }

    #[test]
    fn reads_formats_frames_and_bandwidth() {
        let mut dev = [0u8; 18];
        dev[2..4].copy_from_slice(&0x0200u16.to_le_bytes());
        dev[8..10].copy_from_slice(&0x0c45u16.to_le_bytes());
        dev[10..12].copy_from_slice(&0x6366u16.to_le_bytes());
        let u = UvcDevice::parse(Some(&dev), &config(), Some(UsbSpeed::High));
        assert_eq!((u.vendor_id, u.product_id, u.usb_version), (0x0c45, 0x6366, 0x0200));
        assert_eq!(u.formats.len(), 2);
        assert_eq!(u.formats[0].encoding, UvcEncoding::Uncompressed { fourcc: "YUY2".into() });
        assert_eq!(u.formats[0].bits_per_pixel, 16);
        assert_eq!(u.formats[0].frames.len(), 2);
        assert_eq!(u.formats[0].frames[1].fps, vec![10.0, 5.0]);
        assert_eq!(u.formats[1].encoding, UvcEncoding::Mjpeg);
        assert!((u.formats[1].frames[0].fps[0] - 30.0).abs() < 0.01);
        assert_eq!(u.max_bytes_per_s, Some(3 * 1024 * 8000));

        let m = |w, h, fps, format| VideoMode { width: w, height: h, fps, format };
        assert!(u.sends(&m(1920, 1080, 5.0, PixelFormat::Yuyv)));
        assert!(!u.sends(&m(1920, 1080, 30.0, PixelFormat::Yuyv)));
        assert!(u.sends(&m(1920, 1080, 30.0, PixelFormat::Mjpeg)));
        assert!(!u.sends(&m(1920, 1080, 30.0, PixelFormat::Nv12)));
        assert_eq!(u.raw_bytes_per_s(&m(1920, 1080, 5.0, PixelFormat::Yuyv)), Some(1920 * 1080 * 2 * 5));
        assert!(u.hex().starts_with("0000  09 02"));
    }

    #[test]
    fn a_short_or_broken_descriptor_gives_what_it_can() {
        let mut c = config();
        c.truncate(60);
        let u = UvcDevice::parse(None, &c, None);
        assert_eq!(u.formats.len(), 1);
        assert!(u.formats[0].frames.is_empty());
        assert_eq!(UvcDevice::parse(None, &[0, 0, 0], None).formats.len(), 0);
    }
}
