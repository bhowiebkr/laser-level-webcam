//! The camera's own USB descriptors (Windows), read from the hub it is plugged into.
//!
//! The hub driver answers `IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION` with the bytes the
//! device itself sends for a standard GET_DESCRIPTOR request, so this is the camera's list, not
//! the operating system's. It works while the camera is streaming and needs no driver change.
#![allow(unsafe_code)]

use laser_core::uvc::{UsbSpeed, UvcDevice};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_DevNode_PropertyW, CM_Get_Device_IDW,
    CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_PropertyW,
    CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_Address, DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::Win32::Devices::Usb::{
    GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION,
    IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::{HSTRING, PCWSTR};

use crate::CameraError;

fn err(what: &str) -> CameraError {
    CameraError::Os(format!("USB descriptor: {what}"))
}

fn wide(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

/// The device instance ID behind a camera's symbolic link.
fn instance_id(link: &str) -> Option<String> {
    let link = HSTRING::from(link);
    let mut kind = DEVPROPTYPE::default();
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the buffer is `size` bytes and outlives the call.
    let r = unsafe {
        CM_Get_Device_Interface_PropertyW(
            PCWSTR(link.as_ptr()),
            &DEVPKEY_Device_InstanceId,
            &mut kind,
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            0,
        )
    };
    (r == CR_SUCCESS).then(|| wide(&buf))
}

fn node_id(node: u32) -> Option<String> {
    let mut buf = [0u16; 512];
    // SAFETY: the buffer outlives the call.
    (unsafe { CM_Get_Device_IDW(node, &mut buf, 0) } == CR_SUCCESS).then(|| wide(&buf))
}

fn parent(node: u32) -> Option<u32> {
    let mut p = 0u32;
    // SAFETY: plain out-parameter.
    (unsafe { CM_Get_Parent(&mut p, node, 0) } == CR_SUCCESS).then_some(p)
}

/// The port a USB device sits on: its address property.
fn port(node: u32) -> Option<u32> {
    let mut kind = DEVPROPTYPE::default();
    let mut v = 0u32;
    let mut size = 4u32;
    // SAFETY: a four-byte buffer for a UINT32 property.
    let r = unsafe {
        CM_Get_DevNode_PropertyW(node, &DEVPKEY_Device_Address, &mut kind, Some((&raw mut v).cast()), &mut size, 0)
    };
    (r == CR_SUCCESS && size == 4).then_some(v)
}

/// The hub interface path of a device node, if it is a hub.
fn hub_path(id: &str) -> Option<String> {
    let id = HSTRING::from(id);
    let mut len = 0u32;
    // SAFETY: size query, then a list into a buffer of that many wide characters.
    unsafe {
        if CM_Get_Device_Interface_List_SizeW(
            &mut len,
            &GUID_DEVINTERFACE_USB_HUB,
            PCWSTR(id.as_ptr()),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
            || len <= 1
        {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_Interface_ListW(
            &GUID_DEVINTERFACE_USB_HUB,
            PCWSTR(id.as_ptr()),
            &mut buf,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
        {
            return None;
        }
        let path = wide(&buf);
        (!path.is_empty()).then_some(path)
    }
}

/// An open hub, closed when dropped.
struct Hub(HANDLE);

impl Drop for Hub {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateFileW and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Hub {
    fn open(path: &str) -> Result<Hub, CameraError> {
        // SAFETY: opens the hub's device interface; the handle is owned by `Hub`.
        let h = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                GENERIC_WRITE.0,
                FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .map_err(|e| err(&format!("cannot open the hub: {}", e.message())))?;
        Ok(Hub(h))
    }

    fn ioctl(&self, code: u32, buf: &mut [u8]) -> Result<usize, CameraError> {
        let mut got = 0u32;
        let n = buf.len() as u32;
        // SAFETY: METHOD_BUFFERED: the same buffer goes in and out, `n` bytes long.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                Some(buf.as_ptr().cast()),
                n,
                Some(buf.as_mut_ptr().cast()),
                n,
                Some(&mut got),
                None,
            )
        }
        .map_err(|e| err(&e.message()))?;
        Ok(got as usize)
    }

    /// A standard GET_DESCRIPTOR request to the device on `port`: the bytes it returns.
    fn descriptor(&self, port: u32, kind: u8, index: u8, len: u16) -> Result<Vec<u8>, CameraError> {
        // USB_DESCRIPTOR_REQUEST, packed: ConnectionIndex, then the setup packet, then data.
        let mut buf = vec![0u8; 12 + usize::from(len)];
        buf[0..4].copy_from_slice(&port.to_le_bytes());
        buf[4] = 0x80; // device to host, standard, device
        buf[5] = 0x06; // GET_DESCRIPTOR
        buf[6..8].copy_from_slice(&(u16::from(kind) << 8 | u16::from(index)).to_le_bytes());
        buf[10..12].copy_from_slice(&len.to_le_bytes());
        let got = self.ioctl(IOCTL_USB_GET_DESCRIPTOR_FROM_NODE_CONNECTION, &mut buf)?;
        Ok(buf[12..got.max(12)].to_vec())
    }

    /// The device descriptor and link speed of the device on `port`.
    fn connection(&self, port: u32) -> Result<(Vec<u8>, Option<UsbSpeed>), CameraError> {
        // USB_NODE_CONNECTION_INFORMATION_EX, packed: ConnectionIndex (4), DeviceDescriptor
        // (18), CurrentConfigurationValue (1), Speed (1), … then the open pipes.
        let mut buf = vec![0u8; 35 + 32 * 11];
        buf[0..4].copy_from_slice(&port.to_le_bytes());
        self.ioctl(IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX, &mut buf)?;
        Ok((buf[4..22].to_vec(), UsbSpeed::from_code(buf[23])))
    }
}

/// Read the descriptors of the camera behind a Media Foundation symbolic link.
pub fn uvc(link: &str) -> Result<UvcDevice, CameraError> {
    let id = instance_id(link).ok_or_else(|| err("no device instance for the camera"))?;
    let mut node = 0u32;
    // SAFETY: plain out-parameter and a NUL-terminated string that outlives the call.
    if unsafe { CM_Locate_DevNodeW(&mut node, &HSTRING::from(id.as_str()), CM_LOCATE_DEVNODE_NORMAL) } != CR_SUCCESS {
        return Err(err("the camera's device node is gone"));
    }
    // Walk up from the camera's interface to the device that sits on a hub port.
    for _ in 0..6 {
        let up = parent(node).ok_or_else(|| err("no hub above the camera"))?;
        if let Some(path) = node_id(up).and_then(|i| hub_path(&i)) {
            let port = port(node).ok_or_else(|| err("no port number"))?;
            let hub = Hub::open(&path)?;
            let (device, speed) = hub.connection(port)?;
            let head = hub.descriptor(port, 2, 0, 9)?;
            let total = head.get(2..4).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
            if total < 9 {
                return Err(err("the configuration descriptor is empty"));
            }
            let config = hub.descriptor(port, 2, 0, total)?;
            return Ok(UvcDevice::parse(Some(&device), &config, speed));
        }
        node = up;
    }
    Err(err("no hub above the camera"))
}
