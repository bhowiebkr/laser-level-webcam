//! Linux cameras through Video4Linux2, the kernel's own camera interface.
//!
//! A camera is a `/dev/video*` node with the video capture capability. Frames come by
//! memory-mapped streaming, as the camera sends them (no conversion happens in the kernel), and
//! the controls are the node's V4L2 controls. The camera's USB descriptors come from sysfs.
//! The structures below follow `linux/videodev2.h` on 64-bit targets; their sizes are checked
//! at compile time against the sizes the kernel's ioctl numbers encode.
#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use laser_core::camera::{
    ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, PixelFormat, VideoMode, pick_mode,
};
use laser_core::frame::LumaImage;
use laser_core::uvc::{UsbSpeed, UvcDevice};

use crate::{Backend, CameraError, Clock, DeviceInfo, Frame, FrameSource, jpeg_luma};

// The kernel's structures.

#[repr(C)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct FmtDesc {
    index: u32,
    kind: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

/// `v4l2_frmsizeenum`. Discrete: `u` = width, height. A range: min, max and step of the width,
/// then of the height.
#[repr(C)]
struct FrmSizeEnum {
    index: u32,
    pixel_format: u32,
    kind: u32,
    u: [u32; 6],
    reserved: [u32; 2],
}

/// `v4l2_frmivalenum`. Discrete: `u` = the frame time as numerator, denominator (s). A range:
/// the shortest frame time, the longest, then the step, each as numerator, denominator.
#[repr(C)]
struct FrmIvalEnum {
    index: u32,
    pixel_format: u32,
    width: u32,
    height: u32,
    kind: u32,
    u: [u32; 6],
    reserved: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

/// `v4l2_format`: the buffer type, then a 200-byte union holding pointers, so it starts at 8.
#[repr(C, align(8))]
struct Format {
    kind: u32,
    _pad: u32,
    pix: PixFormat,
    _rest: [u8; 152],
}

#[repr(C)]
struct RequestBuffers {
    count: u32,
    kind: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

/// `v4l2_buffer`, with its `timeval` as two 64-bit fields and the `m` union as the mmap offset.
#[repr(C)]
struct Buffer {
    index: u32,
    kind: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    _pad: u32,
    sec: i64,
    usec: i64,
    timecode: [u32; 4],
    sequence: u32,
    memory: u32,
    offset: u32,
    _m_high: u32,
    length: u32,
    reserved2: u32,
    request_fd: i32,
    _pad2: u32,
}

/// `v4l2_streamparm` for capture: the time per frame as numerator, denominator (s).
#[repr(C)]
struct StreamParm {
    kind: u32,
    capability: u32,
    capturemode: u32,
    num: u32,
    den: u32,
    _rest: [u8; 184],
}

#[repr(C)]
struct QueryCtrl {
    id: u32,
    kind: u32,
    name: [u8; 32],
    minimum: i32,
    maximum: i32,
    step: i32,
    default_value: i32,
    flags: u32,
    reserved: [u32; 2],
}

#[repr(C)]
struct Control {
    id: u32,
    value: i32,
}

const _: () = {
    use std::mem::size_of;
    assert!(size_of::<Capability>() == 104);
    assert!(size_of::<FmtDesc>() == 64);
    assert!(size_of::<FrmSizeEnum>() == 44);
    assert!(size_of::<FrmIvalEnum>() == 52);
    assert!(size_of::<Format>() == 208);
    assert!(size_of::<RequestBuffers>() == 20);
    assert!(size_of::<Buffer>() == 88);
    assert!(size_of::<StreamParm>() == 204);
    assert!(size_of::<QueryCtrl>() == 68);
    assert!(size_of::<Control>() == 8);
};

/// A kernel structure for which all zero bytes is a valid value.
///
/// # Safety
/// Only for `repr(C)` structures of integers and integer arrays.
unsafe trait Plain: Sized {
    fn zero() -> Self {
        // SAFETY: the implementor is made of integers, for which zero bytes are valid.
        unsafe { std::mem::zeroed() }
    }
}

// SAFETY: each is a repr(C) structure of integers and integer arrays.
unsafe impl Plain for Capability {}
unsafe impl Plain for FmtDesc {}
unsafe impl Plain for FrmSizeEnum {}
unsafe impl Plain for FrmIvalEnum {}
unsafe impl Plain for Format {}
unsafe impl Plain for RequestBuffers {}
unsafe impl Plain for Buffer {}
unsafe impl Plain for StreamParm {}
unsafe impl Plain for QueryCtrl {}
unsafe impl Plain for Control {}
unsafe impl Plain for i32 {}

/// An ioctl number in the generic layout x86-64 and ARM64 use.
const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | nr
}
const WRITE: u32 = 1;
const READ: u32 = 2;
const RW: u32 = READ | WRITE;

const QUERYCAP: u32 = ioc(READ, 0, size_of::<Capability>());
const ENUM_FMT: u32 = ioc(RW, 2, size_of::<FmtDesc>());
const S_FMT: u32 = ioc(RW, 5, size_of::<Format>());
const REQBUFS: u32 = ioc(RW, 8, size_of::<RequestBuffers>());
const QUERYBUF: u32 = ioc(RW, 9, size_of::<Buffer>());
const QBUF: u32 = ioc(RW, 15, size_of::<Buffer>());
const DQBUF: u32 = ioc(RW, 17, size_of::<Buffer>());
const STREAMON: u32 = ioc(WRITE, 18, size_of::<i32>());
const STREAMOFF: u32 = ioc(WRITE, 19, size_of::<i32>());
const S_PARM: u32 = ioc(RW, 22, size_of::<StreamParm>());
const G_CTRL: u32 = ioc(RW, 27, size_of::<Control>());
const S_CTRL: u32 = ioc(RW, 28, size_of::<Control>());
const QUERYCTRL: u32 = ioc(RW, 36, size_of::<QueryCtrl>());
const ENUM_FRAMESIZES: u32 = ioc(RW, 74, size_of::<FrmSizeEnum>());
const ENUM_FRAMEINTERVALS: u32 = ioc(RW, 75, size_of::<FrmIvalEnum>());
const _: () = assert!(QUERYCAP == 0x8068_5600 && DQBUF == 0xc058_5611 && S_FMT == 0xc0d0_5605);

const CAPTURE: u32 = 1;
const MMAP: u32 = 1;
const FIELD_NONE: u32 = 1;
const DISCRETE: u32 = 1;
const CAP_VIDEO_CAPTURE: u32 = 0x1;
const CAP_STREAMING: u32 = 0x0400_0000;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
const BUF_FLAG_ERROR: u32 = 0x40;
const CTRL_FLAG_DISABLED: u32 = 0x1;

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*s)
}
const YUYV: u32 = fourcc(b"YUYV");
const MJPG: u32 = fourcc(b"MJPG");
const JPEG: u32 = fourcc(b"JPEG");
const GREY: u32 = fourcc(b"GREY");
const NV12: u32 = fourcc(b"NV12");

fn pixel_format(code: u32) -> Option<PixelFormat> {
    match code {
        YUYV => Some(PixelFormat::Yuyv),
        MJPG | JPEG => Some(PixelFormat::Mjpeg),
        GREY => Some(PixelFormat::Grey),
        NV12 => Some(PixelFormat::Nv12),
        _ => None,
    }
}

// Control ids.
const BRIGHTNESS: u32 = 0x0098_0900;
const CONTRAST: u32 = 0x0098_0901;
const SATURATION: u32 = 0x0098_0902;
const HUE: u32 = 0x0098_0903;
const AUTO_WHITE_BALANCE: u32 = 0x0098_090c;
const GAMMA: u32 = 0x0098_0910;
const AUTOGAIN: u32 = 0x0098_0912;
const GAIN: u32 = 0x0098_0913;
const POWER_LINE_FREQUENCY: u32 = 0x0098_0918;
const HUE_AUTO: u32 = 0x0098_0919;
const WHITE_BALANCE_TEMPERATURE: u32 = 0x0098_091a;
const SHARPNESS: u32 = 0x0098_091b;
const BACKLIGHT_COMPENSATION: u32 = 0x0098_091c;
const EXPOSURE_AUTO: u32 = 0x009a_0901;
const EXPOSURE_ABSOLUTE: u32 = 0x009a_0902;
const FOCUS_ABSOLUTE: u32 = 0x009a_090a;
const FOCUS_AUTO: u32 = 0x009a_090c;
const ZOOM_ABSOLUTE: u32 = 0x009a_090d;
// The auto exposure menu.
const EXPOSURE_FULL_AUTO: i32 = 0;
const EXPOSURE_MANUAL: i32 = 1;
const EXPOSURE_APERTURE_PRIORITY: i32 = 3;

fn ioctl<T: Plain>(fd: &File, request: u32, arg: &mut T) -> io::Result<()> {
    loop {
        // SAFETY: `arg` is a live, exclusively borrowed structure of the size the request number
        // encodes (the asserts above check each one), and the kernel writes only within it.
        let r = unsafe { libc::ioctl(fd.as_raw_fd(), request as _, std::ptr::from_mut(arg)) };
        if r != -1 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn os(e: io::Error) -> CameraError {
    CameraError::Os(e.to_string())
}

fn lost(e: io::Error) -> CameraError {
    if e.raw_os_error() == Some(libc::ENODEV) {
        CameraError::Lost("the camera was unplugged".into())
    } else {
        CameraError::Lost(e.to_string())
    }
}

fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

fn open_node(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK).open(path)
}

/// Why a node would not open, in words that say what to do.
fn open_error(e: io::Error, device: &DeviceInfo) -> CameraError {
    match e.raw_os_error() {
        Some(libc::ENOENT | libc::ENODEV) => CameraError::NotFound(device.name.clone()),
        Some(libc::EACCES | libc::EPERM) => CameraError::Os(format!(
            "no permission to open {}: add yourself to the video group (sudo usermod -aG video $USER), then log out and back in",
            device.id
        )),
        Some(libc::EBUSY) => CameraError::Os(format!("{} is in use by another program", device.name)),
        _ => os(e),
    }
}

/// The node's name if it is a camera that streams.
fn capture_card(fd: &File) -> Option<String> {
    let mut c = Capability::zero();
    ioctl(fd, QUERYCAP, &mut c).ok()?;
    let caps = if c.capabilities & CAP_DEVICE_CAPS != 0 { c.device_caps } else { c.capabilities };
    (caps & CAP_VIDEO_CAPTURE != 0 && caps & CAP_STREAMING != 0).then(|| text(&c.card))
}

/// One mode as the driver lists it, with what it takes to ask for it.
#[derive(Clone, Copy)]
struct Native {
    mode: VideoMode,
    fourcc: u32,
    /// Frame time, s, as numerator, denominator.
    interval: (u32, u32),
}

fn natives(fd: &File) -> Vec<Native> {
    let mut out: Vec<Native> = Vec::new();
    for index in 0u32.. {
        let mut d = FmtDesc::zero();
        d.index = index;
        d.kind = CAPTURE;
        if ioctl(fd, ENUM_FMT, &mut d).is_err() {
            break;
        }
        let Some(format) = pixel_format(d.pixelformat) else { continue };
        for (width, height) in sizes(fd, d.pixelformat) {
            for (num, den) in intervals(fd, d.pixelformat, width, height) {
                if num == 0 || den == 0 {
                    continue;
                }
                let mode = VideoMode { width, height, fps: f64::from(den) / f64::from(num), format };
                if !out.iter().any(|n| n.mode == mode) {
                    out.push(Native { mode, fourcc: d.pixelformat, interval: (num, den) });
                }
            }
        }
    }
    out
}

fn sizes(fd: &File, code: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for index in 0u32.. {
        let mut s = FrmSizeEnum::zero();
        s.index = index;
        s.pixel_format = code;
        if ioctl(fd, ENUM_FRAMESIZES, &mut s).is_err() {
            break;
        }
        if s.kind == DISCRETE {
            out.push((s.u[0], s.u[1]));
            continue;
        }
        // A range of sizes: the usual ones inside it, and its largest.
        let [min_w, max_w, step_w, min_h, max_h, step_h] = s.u;
        let fits = |w: u32, h: u32| {
            (min_w..=max_w).contains(&w)
                && (min_h..=max_h).contains(&h)
                && (w - min_w).is_multiple_of(step_w.max(1))
                && (h - min_h).is_multiple_of(step_h.max(1))
        };
        out.extend([(640, 480), (1280, 720), (1920, 1080)].into_iter().filter(|&(w, h)| fits(w, h)));
        out.push((max_w, max_h));
        break;
    }
    out
}

fn intervals(fd: &File, code: u32, width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for index in 0u32.. {
        let mut v = FrmIvalEnum::zero();
        v.index = index;
        v.pixel_format = code;
        v.width = width;
        v.height = height;
        if ioctl(fd, ENUM_FRAMEINTERVALS, &mut v).is_err() {
            break;
        }
        if v.kind == DISCRETE {
            out.push((v.u[0], v.u[1]));
            continue;
        }
        // A range of frame times: its fastest and slowest ends.
        out.push((v.u[0], v.u[1]));
        out.push((v.u[2], v.u[3]));
        break;
    }
    out
}

/// The `/dev/video*` nodes, by number.
fn nodes() -> Vec<PathBuf> {
    let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let n = name.strip_prefix("video")?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// A sysfs attribute of a video node.
fn sysfs(node: &Path, attr: &str) -> Option<String> {
    let name = node.file_name()?;
    let s = std::fs::read_to_string(Path::new("/sys/class/video4linux").join(name).join(attr)).ok()?;
    Some(s.trim().to_string())
}

pub fn devices() -> Vec<DeviceInfo> {
    let mut out: Vec<DeviceInfo> = Vec::new();
    for path in nodes() {
        let name = match open_node(&path) {
            Ok(fd) => match capture_card(&fd) {
                Some(card) if !natives(&fd).is_empty() => card,
                _ => continue,
            },
            // Listed anyway, so that opening it says what permission is missing. A UVC
            // camera's first node is its picture; the next is its metadata.
            Err(e) if e.raw_os_error() == Some(libc::EACCES) && sysfs(&path, "index").as_deref() == Some("0") => {
                match sysfs(&path, "name") {
                    Some(n) => n,
                    None => continue,
                }
            }
            Err(_) => continue,
        };
        // Two of the same camera need names apart, since a camera is chosen by name.
        let mut unique = name.clone();
        let mut k = 2;
        while out.iter().any(|d| d.name == unique) {
            unique = format!("{name} ({k})");
            k += 1;
        }
        out.push(DeviceInfo { name: unique, id: path.to_string_lossy().into_owned(), backend: Backend::V4l2 });
    }
    out
}

/// The camera's USB descriptors from sysfs. The video node's device is the USB interface; the
/// directory above it is the USB device, whose `descriptors` file holds the device descriptor
/// followed by the configuration descriptors, as the camera sent them.
pub fn uvc(node: &str) -> Result<UvcDevice, CameraError> {
    let err = |what: &str| CameraError::Os(format!("USB descriptor: {what}"));
    let name = Path::new(node).file_name().ok_or_else(|| err("not a video node"))?;
    let interface = std::fs::canonicalize(Path::new("/sys/class/video4linux").join(name).join("device"))
        .map_err(|e| err(&e.to_string()))?;
    let usb = interface.parent().ok_or_else(|| err("no USB device above the camera"))?;
    let bytes = std::fs::read(usb.join("descriptors")).map_err(|_| err("not a USB camera"))?;
    if bytes.len() < 18 + 9 || bytes[1] != 1 {
        return Err(err("no device descriptor"));
    }
    let speed = std::fs::read_to_string(usb.join("speed")).ok().and_then(|s| match s.trim() {
        "1.5" => Some(UsbSpeed::Low),
        "12" => Some(UsbSpeed::Full),
        "480" => Some(UsbSpeed::High),
        "5000" | "10000" | "20000" => Some(UsbSpeed::Super),
        _ => None,
    });
    Ok(UvcDevice::parse(Some(&bytes[..18]), &bytes[18..], speed))
}

/// A control's V4L2 ids: its value, and the switch that puts it on automatic, if any.
fn ids(id: ControlId) -> (u32, Option<u32>) {
    match id {
        ControlId::Exposure => (EXPOSURE_ABSOLUTE, Some(EXPOSURE_AUTO)),
        ControlId::Gain => (GAIN, Some(AUTOGAIN)),
        ControlId::WhiteBalance => (WHITE_BALANCE_TEMPERATURE, Some(AUTO_WHITE_BALANCE)),
        ControlId::PowerLineFrequency => (POWER_LINE_FREQUENCY, None),
        ControlId::Sharpness => (SHARPNESS, None),
        ControlId::BacklightCompensation => (BACKLIGHT_COMPENSATION, None),
        ControlId::Focus => (FOCUS_ABSOLUTE, Some(FOCUS_AUTO)),
        ControlId::Gamma => (GAMMA, None),
        ControlId::Brightness => (BRIGHTNESS, None),
        ControlId::Contrast => (CONTRAST, None),
        ControlId::Saturation => (SATURATION, None),
        ControlId::Hue => (HUE, Some(HUE_AUTO)),
        ControlId::Zoom => (ZOOM_ABSOLUTE, None),
    }
}

fn unit(id: ControlId) -> ControlUnit {
    match id {
        ControlId::Exposure => ControlUnit::HundredMicroseconds,
        ControlId::WhiteBalance => ControlUnit::Kelvin,
        ControlId::PowerLineFrequency => ControlUnit::PowerLineMenu,
        _ => ControlUnit::Raw,
    }
}

/// One of the driver's buffers, mapped into this process.
struct Mapped {
    ptr: *mut libc::c_void,
    len: usize,
}

pub struct V4l2Camera {
    device: DeviceInfo,
    modes: Vec<VideoMode>,
    mode: VideoMode,
    usb: Option<UvcDevice>,
    fd: File,
    /// Bytes per row of the frame as the driver lays it out.
    stride: usize,
    buffers: Vec<Mapped>,
    streaming: bool,
    clock: Clock,
}

impl V4l2Camera {
    pub fn open(device: &DeviceInfo, wanted: Option<&VideoMode>) -> Result<V4l2Camera, CameraError> {
        let fd = open_node(Path::new(&device.id)).map_err(|e| open_error(e, device))?;
        capture_card(&fd).ok_or_else(|| CameraError::Os(format!("{} is not a camera", device.id)))?;
        let natives = natives(&fd);
        let modes: Vec<VideoMode> = natives.iter().map(|n| n.mode).collect();
        let mode = pick_mode(&modes, wanted).ok_or(CameraError::NoMode)?;
        let native = natives.iter().find(|n| n.mode == mode).copied().ok_or(CameraError::NoMode)?;

        let mut f = Format::zero();
        f.kind = CAPTURE;
        f.pix.width = mode.width;
        f.pix.height = mode.height;
        f.pix.pixelformat = native.fourcc;
        f.pix.field = FIELD_NONE;
        ioctl(&fd, S_FMT, &mut f).map_err(|e| open_error(e, device))?;
        if (f.pix.width, f.pix.height, f.pix.pixelformat) != (mode.width, mode.height, native.fourcc) {
            return Err(CameraError::Os(format!("{} would not run in {mode}", device.name)));
        }
        let row = if mode.format == PixelFormat::Yuyv { 2 * mode.width } else { mode.width };
        let stride = f.pix.bytesperline.max(row) as usize;
        let mut p = StreamParm::zero();
        p.kind = CAPTURE;
        (p.num, p.den) = native.interval;
        // Not every driver takes a frame rate; it then runs at its own.
        let _ = ioctl(&fd, S_PARM, &mut p);

        let mut req = RequestBuffers::zero();
        req.count = 4;
        req.kind = CAPTURE;
        req.memory = MMAP;
        ioctl(&fd, REQBUFS, &mut req).map_err(|e| open_error(e, device))?;
        if req.count == 0 {
            return Err(CameraError::Os(format!("{} gave no frame buffers", device.name)));
        }
        let mut cam = V4l2Camera {
            device: device.clone(),
            modes,
            mode,
            usb: uvc(&device.id).ok(),
            fd,
            stride,
            buffers: Vec::new(),
            streaming: false,
            clock: Clock::default(),
        };
        for index in 0..req.count {
            let mut b = Buffer::zero();
            b.index = index;
            b.kind = CAPTURE;
            b.memory = MMAP;
            ioctl(&cam.fd, QUERYBUF, &mut b).map_err(os)?;
            // SAFETY: maps buffer `index` at the offset and length the driver gave for it;
            // Drop unmaps it.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    b.length as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    cam.fd.as_raw_fd(),
                    libc::off_t::from(b.offset),
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(os(io::Error::last_os_error()));
            }
            cam.buffers.push(Mapped { ptr, len: b.length as usize });
        }
        for index in 0..req.count {
            cam.queue(index)?;
        }
        let mut kind = CAPTURE as i32;
        ioctl(&cam.fd, STREAMON, &mut kind).map_err(|e| open_error(e, device))?;
        cam.streaming = true;
        Ok(cam)
    }

    /// Hand buffer `index` back to the driver to fill.
    fn queue(&self, index: u32) -> Result<(), CameraError> {
        let mut b = Buffer::zero();
        b.index = index;
        b.kind = CAPTURE;
        b.memory = MMAP;
        ioctl(&self.fd, QBUF, &mut b).map_err(lost)
    }

    /// The luma plane of a filled buffer; empty if it does not fit the mode.
    fn luma(&self, bytes: &[u8]) -> Vec<u8> {
        let (w, h, s) = (self.mode.width as usize, self.mode.height as usize, self.stride);
        match self.mode.format {
            PixelFormat::Yuyv => LumaImage::from_yuyv(w, h, s, bytes).map(|i| i.data).unwrap_or_default(),
            PixelFormat::Mjpeg => jpeg_luma(bytes, w, h),
            // Grey, or NV12's luma plane, which comes first.
            _ if h > 0 && bytes.len() >= s * (h - 1) + w => {
                let mut out = Vec::with_capacity(w * h);
                for y in 0..h {
                    out.extend_from_slice(&bytes[y * s..y * s + w]);
                }
                out
            }
            _ => Vec::new(),
        }
    }

    fn query(&self, id: u32) -> Option<QueryCtrl> {
        let mut q = QueryCtrl::zero();
        q.id = id;
        ioctl(&self.fd, QUERYCTRL, &mut q).ok()?;
        (q.flags & CTRL_FLAG_DISABLED == 0).then_some(q)
    }

    fn get(&self, id: u32) -> Option<i32> {
        let mut c = Control { id, value: 0 };
        ioctl(&self.fd, G_CTRL, &mut c).ok()?;
        Some(c.value)
    }

    fn set(&self, id: u32, value: i32) -> io::Result<()> {
        let mut c = Control { id, value };
        ioctl(&self.fd, S_CTRL, &mut c)
    }

    fn read(&self, id: ControlId) -> Option<ControlInfo> {
        let (value_id, auto_id) = ids(id);
        let q = self.query(value_id)?;
        let value = self.get(value_id).unwrap_or(q.default_value);
        let auto = auto_id.and_then(|a| {
            self.query(a)?;
            let v = self.get(a)?;
            Some(if a == EXPOSURE_AUTO { v != EXPOSURE_MANUAL } else { v != 0 })
        });
        Some(ControlInfo {
            id,
            min: i64::from(q.minimum),
            max: i64::from(q.maximum),
            step: i64::from(q.step.max(1)),
            default: i64::from(q.default_value),
            value: i64::from(value),
            auto,
            unit: unit(id),
        })
    }
}

impl FrameSource for V4l2Camera {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn modes(&self) -> &[VideoMode] {
        &self.modes
    }

    fn mode(&self) -> VideoMode {
        self.mode
    }

    fn usb(&self) -> Option<&UvcDevice> {
        self.usb.as_ref()
    }

    fn controls(&mut self) -> Vec<ControlInfo> {
        ControlId::ALL.iter().filter_map(|&id| self.read(id)).collect()
    }

    fn set_control(&mut self, setting: ControlSetting) -> Result<ControlInfo, CameraError> {
        let id = setting.id;
        let now = self.read(id).ok_or(CameraError::NoControl(id))?;
        let (value_id, auto_id) = ids(id);
        let auto_id = auto_id.filter(|_| now.auto.is_some());
        match setting.mode {
            ControlMode::Manual(v) => {
                // A UVC camera refuses a manual value while the control is on automatic.
                if let Some(a) = auto_id {
                    self.set(a, if a == EXPOSURE_AUTO { EXPOSURE_MANUAL } else { 0 }).map_err(os)?;
                }
                self.set(value_id, now.snap(v) as i32).map_err(os)?;
            }
            ControlMode::Auto => {
                let a = auto_id.ok_or_else(|| {
                    CameraError::Os(format!("the camera has no automatic {}", id.label().to_lowercase()))
                })?;
                if a == EXPOSURE_AUTO {
                    // UVC cameras offer aperture priority (automatic exposure, fixed iris); other
                    // cameras full automatic.
                    if self.set(a, EXPOSURE_APERTURE_PRIORITY).is_err() {
                        self.set(a, EXPOSURE_FULL_AUTO).map_err(os)?;
                    }
                } else {
                    self.set(a, 1).map_err(os)?;
                }
            }
        }
        self.read(id).ok_or(CameraError::NoControl(id))
    }

    fn next_frame(&mut self) -> Result<Frame, CameraError> {
        let (w, h) = (self.mode.width as usize, self.mode.height as usize);
        // Long enough for the camera to start, and for a few frames at its slowest.
        let wait_s = if self.clock.started() { (5.0 / self.mode.fps.max(0.1)).max(3.0) } else { 8.0 };
        let mut bad = 0;
        loop {
            let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one pollfd, live across the call.
            let n = unsafe { libc::poll(&mut pfd, 1, (wait_s * 1000.0) as libc::c_int) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(lost(e));
            }
            if n == 0 {
                return Err(CameraError::Lost(format!("no frame for {wait_s:.0} s")));
            }
            let mut b = Buffer::zero();
            b.kind = CAPTURE;
            b.memory = MMAP;
            if let Err(e) = ioctl(&self.fd, DQBUF, &mut b) {
                if e.raw_os_error() == Some(libc::EAGAIN) {
                    continue;
                }
                return Err(lost(e));
            }
            let Some(m) = self.buffers.get(b.index as usize) else {
                return Err(CameraError::Lost(format!("the driver gave back buffer {}", b.index)));
            };
            let used = (b.bytesused as usize).min(m.len);
            // SAFETY: the driver filled `used` bytes of this buffer and leaves it to us until it
            // is queued again, below; nothing keeps the slice past that.
            let bytes = unsafe { std::slice::from_raw_parts(m.ptr.cast::<u8>(), used) };
            let data = if b.flags & BUF_FLAG_ERROR != 0 { Vec::new() } else { self.luma(bytes) };
            self.queue(b.index)?;
            if data.len() != w * h {
                // A damaged frame now and then (a JPEG cut short) is skipped.
                bad += 1;
                if bad >= 10 {
                    return Err(CameraError::Lost(format!("{bad} frames in a row did not fit {w}×{h}")));
                }
                continue;
            }
            let (t_s, seq) = self.clock.stamp(b.sec as f64 + b.usec as f64 * 1e-6, self.mode.fps);
            return Ok(Frame { width: w, height: h, data, t_s, seq });
        }
    }
}

impl Drop for V4l2Camera {
    fn drop(&mut self) {
        if self.streaming {
            let mut kind = CAPTURE as i32;
            let _ = ioctl(&self.fd, STREAMOFF, &mut kind);
        }
        for m in &self.buffers {
            // SAFETY: each was mapped in `open` with this length and is not used after this.
            unsafe { libc::munmap(m.ptr, m.len) };
        }
        // Free the driver's buffers, so the next open can ask for its own.
        let mut req = RequestBuffers::zero();
        req.kind = CAPTURE;
        req.memory = MMAP;
        let _ = ioctl(&self.fd, REQBUFS, &mut req);
    }
}

/// Opens the first real camera, if there is one. Run with `--ignored` and a camera plugged in.
#[cfg(test)]
mod tests {
    use laser_core::camera::{Mains, for_measuring};

    use super::*;
    use crate::{Applied, apply};

    #[test]
    #[ignore = "needs a camera"]
    fn real_camera_locks() {
        let dev = devices().into_iter().next().expect("a camera");
        let mut cam = V4l2Camera::open(&dev, None).unwrap();
        eprintln!("{} in {}; modes {:?}; usb {:?}", dev.name, cam.mode(), cam.modes(), cam.usb().map(|u| u.vendor_id));
        let infos = cam.controls();
        let applied = apply(&mut cam, &for_measuring(&infos, Mains::Hz60));
        for a in &applied {
            eprintln!("{:?} held {} -> {:?}", a.setting, a.held(), a.result);
        }
        let t0 = cam.next_frame().unwrap();
        let mut last = t0.clone();
        for _ in 0..30 {
            last = cam.next_frame().unwrap();
        }
        eprintln!("{:.1} fps by camera time", (last.seq - t0.seq) as f64 / (last.t_s - t0.t_s));
        assert!(applied.iter().all(Applied::held));
    }
}
