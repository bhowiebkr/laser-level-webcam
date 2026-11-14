//! Media Foundation camera (Windows).
//!
//! Frames come from an `IMFSourceReader` in the camera's own format, with the converters off,
//! so NV12 and YUY2 arrive as the sensor's Y plane. Controls go through the DirectShow
//! `IAMCameraControl` and `IAMVideoProcAmp` interfaces of the media source, always with an
//! explicit manual or auto flag (a setter that keeps the current flag leaves an automatic
//! control on automatic). The power-line filter is the kernel-streaming video proc amp
//! property 13, which neither interface lists.
#![allow(unsafe_code)]

use std::ffi::c_void;

use laser_core::camera::{
    ControlId, ControlInfo, ControlMode, ControlSetting, ControlUnit, PixelFormat, VideoMode, pick_mode,
};
use laser_core::frame::LumaImage;
use windows::Win32::Media::DirectShow::{
    CameraControl_Flags_Auto, CameraControl_Flags_Manual, IAMCameraControl, IAMVideoProcAmp, VideoProcAmp_Flags_Auto,
    VideoProcAmp_Flags_Manual,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0, KSPROPERTY_TYPE_GET, KSPROPERTY_TYPE_SET,
    KSPROPERTY_VIDEOPROCAMP_FLAGS_MANUAL, KSPROPERTY_VIDEOPROCAMP_POWERLINE_FREQUENCY, KSPROPERTY_VIDEOPROCAMP_S,
    PROPSETID_VIDCAP_VIDEOPROCAMP,
};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSourceReader, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_SUBTYPE,
    MF_READWRITE_DISABLE_CONVERTERS, MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_VERSION, MFCreateAttributes,
    MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFSTARTUP_NOSOCKET, MFShutdown, MFStartup,
    MFVideoFormat_L8, MFVideoFormat_NV12, MFVideoFormat_YUY2,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::core::{GUID, Interface, PWSTR};

use crate::{Backend, CameraError, DeviceInfo, Frame, FrameSource};

const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

fn os(e: windows::core::Error) -> CameraError {
    CameraError::Os(format!("{} ({:#010x})", e.message(), e.code().0))
}

/// Media Foundation started on this thread; stopped when dropped.
struct Mf;

impl Mf {
    fn start() -> Result<Mf, CameraError> {
        // SAFETY: COM and MF start-ups are counted per thread and per process; each successful
        // MFStartup is matched by the MFShutdown in Drop. A thread already in COM keeps its mode.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(os)?;
        }
        Ok(Mf)
    }
}

impl Drop for Mf {
    fn drop(&mut self) {
        // SAFETY: matches the MFStartup in `start`.
        unsafe {
            let _ = MFShutdown();
        }
    }
}

fn video_devices() -> Result<Vec<IMFActivate>, CameraError> {
    // SAFETY: the attribute store is created and released by the windows crate. The array from
    // MFEnumDeviceSources holds `n` activation objects; each is taken out (so we own its
    // reference) before the array itself is freed with CoTaskMemFree.
    unsafe {
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1).map_err(os)?;
        let attrs = attrs.ok_or_else(|| CameraError::Os("no attribute store".into()))?;
        attrs
            .SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)
            .map_err(os)?;
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut n = 0u32;
        MFEnumDeviceSources(&attrs, &mut list, &mut n).map_err(os)?;
        let mut out = Vec::with_capacity(n as usize);
        if !list.is_null() {
            for i in 0..n as usize {
                if let Some(a) = (*list.add(i)).take() {
                    out.push(a);
                }
            }
            CoTaskMemFree(Some(list as *const c_void));
        }
        Ok(out)
    }
}

fn string(a: &IMFActivate, key: &GUID) -> Result<String, CameraError> {
    // SAFETY: GetAllocatedString gives a CoTaskMem string that we copy and then free.
    unsafe {
        let mut p = PWSTR::null();
        let mut len = 0u32;
        a.GetAllocatedString(key, &mut p, &mut len).map_err(os)?;
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const c_void));
        Ok(s)
    }
}

fn device_info(a: &IMFActivate) -> Result<DeviceInfo, CameraError> {
    Ok(DeviceInfo {
        name: string(a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME)?,
        id: string(a, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK)?,
        backend: Backend::MediaFoundation,
    })
}

pub fn devices() -> Result<Vec<DeviceInfo>, CameraError> {
    let _mf = Mf::start()?;
    video_devices()?.iter().map(device_info).collect()
}

fn pixel_format(subtype: GUID) -> Option<PixelFormat> {
    if subtype == MFVideoFormat_NV12 {
        Some(PixelFormat::Nv12)
    } else if subtype == MFVideoFormat_YUY2 {
        Some(PixelFormat::Yuyv)
    } else if subtype == MFVideoFormat_L8 {
        Some(PixelFormat::Grey)
    } else {
        None
    }
}

fn mode_of(t: &IMFMediaType) -> Option<VideoMode> {
    // SAFETY: plain attribute reads from a media type the reader gave us.
    unsafe {
        let format = pixel_format(t.GetGUID(&MF_MT_SUBTYPE).ok()?)?;
        let size = t.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let rate = t.GetUINT64(&MF_MT_FRAME_RATE).ok()?;
        let (num, den) = ((rate >> 32) as u32, rate as u32);
        Some(VideoMode {
            width: (size >> 32) as u32,
            height: size as u32,
            fps: if den == 0 { 0.0 } else { f64::from(num) / f64::from(den) },
            format,
        })
    }
}

/// Where a control lives.
#[derive(Clone, Copy)]
enum Place {
    Camera(i32),
    Amp(i32),
    PowerLine,
}

fn place(id: ControlId) -> Place {
    match id {
        ControlId::Zoom => Place::Camera(3),
        ControlId::Exposure => Place::Camera(4),
        ControlId::Focus => Place::Camera(6),
        ControlId::Brightness => Place::Amp(0),
        ControlId::Contrast => Place::Amp(1),
        ControlId::Hue => Place::Amp(2),
        ControlId::Saturation => Place::Amp(3),
        ControlId::Sharpness => Place::Amp(4),
        ControlId::Gamma => Place::Amp(5),
        ControlId::WhiteBalance => Place::Amp(7),
        ControlId::BacklightCompensation => Place::Amp(8),
        ControlId::Gain => Place::Amp(9),
        ControlId::PowerLineFrequency => Place::PowerLine,
    }
}

fn unit(id: ControlId) -> ControlUnit {
    match id {
        ControlId::Exposure => ControlUnit::Log2Seconds,
        ControlId::WhiteBalance => ControlUnit::Kelvin,
        ControlId::PowerLineFrequency => ControlUnit::PowerLineMenu,
        _ => ControlUnit::Raw,
    }
}

pub struct MfCamera {
    device: DeviceInfo,
    modes: Vec<VideoMode>,
    mode: VideoMode,
    reader: IMFSourceReader,
    source: IMFMediaSource,
    camera: Option<IAMCameraControl>,
    amp: Option<IAMVideoProcAmp>,
    ks: Option<IKsControl>,
    first_ts: Option<i64>,
    last_ts: i64,
    seq: u64,
    frames: u64,
    // Last, so Media Foundation stops after the objects above are released.
    _mf: Mf,
}

impl MfCamera {
    pub fn open(device: &DeviceInfo, wanted: Option<&VideoMode>) -> Result<MfCamera, CameraError> {
        let mf = Mf::start()?;
        let activate = video_devices()?
            .into_iter()
            .find(|a| device_info(a).is_ok_and(|d| d.id == device.id))
            .ok_or_else(|| CameraError::NotFound(device.name.clone()))?;
        // SAFETY: COM calls on objects owned by the windows crate's smart pointers; each result
        // is checked before use.
        unsafe {
            let source: IMFMediaSource = activate.ActivateObject().map_err(os)?;
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 1).map_err(os)?;
            let attrs = attrs.ok_or_else(|| CameraError::Os("no attribute store".into()))?;
            attrs.SetUINT32(&MF_READWRITE_DISABLE_CONVERTERS, 1).map_err(os)?;
            let reader = MFCreateSourceReaderFromMediaSource(&source, &attrs).map_err(os)?;

            let mut found: Vec<(VideoMode, u32)> = Vec::new();
            let mut i = 0;
            while let Ok(t) = reader.GetNativeMediaType(VIDEO, i) {
                if let Some(m) = mode_of(&t)
                    && !found.iter().any(|(f, _)| *f == m)
                {
                    found.push((m, i));
                }
                i += 1;
            }
            let modes: Vec<VideoMode> = found.iter().map(|(m, _)| *m).collect();
            let mode = pick_mode(&modes, wanted).ok_or(CameraError::NoMode)?;
            let index = found.iter().find(|(m, _)| *m == mode).map(|(_, i)| *i).ok_or(CameraError::NoMode)?;
            let t = reader.GetNativeMediaType(VIDEO, index).map_err(os)?;
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).map_err(os)?;
            reader.SetStreamSelection(VIDEO, true).map_err(os)?;
            reader.SetCurrentMediaType(VIDEO, None, &t).map_err(os)?;

            Ok(MfCamera {
                device: device.clone(),
                modes,
                mode,
                camera: source.cast().ok(),
                amp: source.cast().ok(),
                ks: source.cast().ok(),
                reader,
                source,
                first_ts: None,
                last_ts: 0,
                seq: 0,
                frames: 0,
                _mf: mf,
            })
        }
    }

    fn read(&self, id: ControlId) -> Option<ControlInfo> {
        let (mut min, mut max, mut step, mut default, mut caps, mut value, mut flags) = (0, 0, 0, 0, 0, 0, 0);
        // SAFETY: the out-parameters are locals that live across each call.
        let (auto_flag, has) = unsafe {
            match place(id) {
                Place::Camera(p) => {
                    let c = self.camera.as_ref()?;
                    c.GetRange(p, &mut min, &mut max, &mut step, &mut default, &mut caps).ok()?;
                    c.Get(p, &mut value, &mut flags).ok()?;
                    (CameraControl_Flags_Auto.0, true)
                }
                Place::Amp(p) => {
                    let a = self.amp.as_ref()?;
                    a.GetRange(p, &mut min, &mut max, &mut step, &mut default, &mut caps).ok()?;
                    a.Get(p, &mut value, &mut flags).ok()?;
                    (VideoProcAmp_Flags_Auto.0, true)
                }
                Place::PowerLine => {
                    value = self.power_line(None)?;
                    (min, max, step, default) = (0, 2, 1, value);
                    (0, false)
                }
            }
        };
        let auto = (has && caps & auto_flag != 0).then_some(flags & auto_flag != 0);
        Some(ControlInfo {
            id,
            min: i64::from(min),
            max: i64::from(max),
            step: i64::from(step.max(1)),
            default: i64::from(default),
            value: i64::from(value),
            auto,
            unit: unit(id),
        })
    }

    /// Read the power-line filter, or set it to `set` first.
    fn power_line(&self, set: Option<i32>) -> Option<i32> {
        let ks = self.ks.as_ref()?;
        let size = std::mem::size_of::<KSPROPERTY_VIDEOPROCAMP_S>() as u32;
        let call = |flags: u32, value: i32| -> Option<i32> {
            let mut s = KSPROPERTY_VIDEOPROCAMP_S {
                Property: KSIDENTIFIER {
                    Anonymous: KSIDENTIFIER_0 {
                        Anonymous: KSIDENTIFIER_0_0 {
                            Set: PROPSETID_VIDCAP_VIDEOPROCAMP,
                            Id: KSPROPERTY_VIDEOPROCAMP_POWERLINE_FREQUENCY.0 as u32,
                            Flags: flags,
                        },
                    },
                },
                Value: value,
                Flags: KSPROPERTY_VIDEOPROCAMP_FLAGS_MANUAL as u32,
                Capabilities: 0,
            };
            let mut returned = 0u32;
            let p = &raw mut s;
            // SAFETY: the property header is the first field of `s`, and `s` is both the
            // property and the data buffer of `size` bytes, as the video proc amp set expects.
            unsafe { ks.KsProperty(p.cast::<KSIDENTIFIER>(), size, p.cast::<c_void>(), size, &mut returned).ok()? };
            Some(s.Value)
        };
        if let Some(v) = set {
            call(KSPROPERTY_TYPE_SET, v)?;
        }
        call(KSPROPERTY_TYPE_GET, 0)
    }
}

impl FrameSource for MfCamera {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn modes(&self) -> &[VideoMode] {
        &self.modes
    }

    fn mode(&self) -> VideoMode {
        self.mode
    }

    fn controls(&mut self) -> Vec<ControlInfo> {
        ControlId::ALL.iter().filter_map(|&id| self.read(id)).collect()
    }

    fn set_control(&mut self, setting: ControlSetting) -> Result<ControlInfo, CameraError> {
        let id = setting.id;
        let now = self.read(id).ok_or(CameraError::NoControl(id))?;
        let (value, manual) = match setting.mode {
            ControlMode::Manual(v) => (now.snap(v) as i32, true),
            ControlMode::Auto => (now.value as i32, false),
        };
        // SAFETY: COM calls with plain integer arguments.
        let done = unsafe {
            match place(id) {
                Place::Camera(p) => {
                    let flag = if manual { CameraControl_Flags_Manual } else { CameraControl_Flags_Auto };
                    self.camera.as_ref().map(|c| c.Set(p, value, flag.0))
                }
                Place::Amp(p) => {
                    let flag = if manual { VideoProcAmp_Flags_Manual } else { VideoProcAmp_Flags_Auto };
                    self.amp.as_ref().map(|a| a.Set(p, value, flag.0))
                }
                Place::PowerLine => Some(match self.power_line(Some(value)) {
                    Some(_) => Ok(()),
                    None => Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL)),
                }),
            }
        };
        done.ok_or(CameraError::NoControl(id))?.map_err(os)?;
        self.read(id).ok_or(CameraError::NoControl(id))
    }

    fn next_frame(&mut self) -> Result<Frame, CameraError> {
        let (w, h) = (self.mode.width as usize, self.mode.height as usize);
        loop {
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample = None;
            // SAFETY: ReadSample blocks until a sample or an event; the out-parameters are locals.
            unsafe { self.reader.ReadSample(VIDEO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample)) }
                .map_err(|e| CameraError::Lost(os(e).to_string()))?;
            if flags & (MF_SOURCE_READERF_ERROR.0 | MF_SOURCE_READERF_ENDOFSTREAM.0) as u32 != 0 {
                return Err(CameraError::Lost("the camera ended the stream".into()));
            }
            let Some(sample) = sample else { continue };
            let mut data = Vec::new();
            // SAFETY: Lock gives a pointer to `len` valid bytes until Unlock; they are copied
            // into a slice of that length and nothing keeps the pointer after Unlock.
            unsafe {
                let buf = sample.ConvertToContiguousBuffer().map_err(os)?;
                let mut p: *mut u8 = std::ptr::null_mut();
                let mut len = 0u32;
                buf.Lock(&mut p, None, Some(&mut len)).map_err(os)?;
                if !p.is_null() {
                    let bytes = std::slice::from_raw_parts(p, len as usize);
                    data = match self.mode.format {
                        PixelFormat::Yuyv => {
                            LumaImage::from_yuyv(w, h, 2 * w, bytes).map(|i| i.data).unwrap_or_default()
                        }
                        _ if bytes.len() >= w * h => bytes[..w * h].to_vec(),
                        _ => Vec::new(),
                    };
                }
                let _ = buf.Unlock();
            }
            if data.len() != w * h {
                return Err(CameraError::Lost(format!("a frame of {} bytes does not fit {w}×{h}", data.len())));
            }
            let first = *self.first_ts.get_or_insert(ts);
            if self.frames > 0 {
                let period = 1e7 / self.mode.fps.max(1.0);
                self.seq += ((ts - self.last_ts) as f64 / period).round().max(1.0) as u64;
            }
            self.last_ts = ts;
            self.frames += 1;
            return Ok(Frame { width: w, height: h, data, t_s: (ts - first) as f64 * 1e-7, seq: self.seq });
        }
    }
}

impl Drop for MfCamera {
    fn drop(&mut self) {
        // SAFETY: stops the camera before the reader and source are released.
        unsafe {
            let _ = self.source.Shutdown();
        }
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
        let dev = devices().unwrap().into_iter().next().expect("a camera");
        let mut cam = MfCamera::open(&dev, None).unwrap();
        eprintln!("{} in {}; modes {:?}", dev.name, cam.mode(), cam.modes());
        let infos = cam.controls();
        for c in &infos {
            eprintln!("{c:?}");
        }
        let applied = apply(&mut cam, &for_measuring(&infos, Mains::Hz60));
        for a in &applied {
            eprintln!("{:?} held {} -> {:?}", a.setting, a.held(), a.result);
        }
        let t0 = cam.next_frame().unwrap();
        let mut last = t0.clone();
        for _ in 0..30 {
            last = cam.next_frame().unwrap();
        }
        let fps = (last.seq - t0.seq) as f64 / (last.t_s - t0.t_s);
        eprintln!("{fps:.1} fps by camera time, seq {}", last.seq);
        assert!(applied.iter().all(Applied::held));
        // Leave the camera as it was.
        for c in &infos {
            let mode = if c.auto == Some(true) { ControlMode::Auto } else { ControlMode::Manual(c.value) };
            cam.set_control(ControlSetting { id: c.id, mode }).unwrap();
        }
        let after = cam.controls();
        assert_eq!(
            after.iter().map(|c| (c.value, c.auto)).collect::<Vec<_>>(),
            infos.iter().map(|c| (c.value, c.auto)).collect::<Vec<_>>()
        );
    }
}
