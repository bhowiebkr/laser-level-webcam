//! A camera on its own thread.
//!
//! [`Capture::start`] opens the camera inside a new thread (a source stays on the thread that
//! opened it) and sends [`Event`]s back. When the receiver falls behind, frames are dropped
//! and counted rather than queued, so the newest frame is never more than a few frames old;
//! every other event waits for the receiver.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use laser_core::camera::{CameraSettings, ControlInfo, ControlSetting, VideoMode};

use crate::{Applied, CameraError, DeviceInfo, Frame, FrameSource};

/// Frames that may wait for the receiver.
const QUEUE: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Set one control; answered by an [`Event::Control`].
    Set(ControlSetting),
    /// Set several; one [`Event::Control`] each.
    Apply(Vec<ControlSetting>),
    /// Read every control again (automatic ones move); answered by [`Event::Controls`].
    Refresh,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The camera is open and the settings' controls were applied.
    Opened {
        device: DeviceInfo,
        modes: Vec<VideoMode>,
        mode: VideoMode,
        controls: Vec<ControlInfo>,
        applied: Vec<Applied>,
        /// What the camera itself says it sends, from its USB descriptors.
        usb: Option<laser_core::uvc::UvcDevice>,
        /// The synthetic camera's simulated block.
        block: Option<crate::synthetic::Block>,
    },
    Frame(Frame),
    Controls(Vec<ControlInfo>),
    Control(Applied),
    /// The camera could not open, or stopped. Nothing follows.
    Error(CameraError),
}

pub struct Capture {
    commands: Sender<Command>,
    events: Receiver<Event>,
    dropped: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    /// Open the camera the settings name on a new thread. Errors opening it arrive as an
    /// [`Event::Error`].
    pub fn start(settings: CameraSettings) -> Result<Capture, CameraError> {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::sync_channel(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&dropped);
        let thread = thread::Builder::new()
            .name("camera".into())
            .spawn(move || run(&settings, &command_rx, &event_tx, &counter))
            .map_err(|e| CameraError::Os(format!("no camera thread: {e}")))?;
        Ok(Capture { commands, events, dropped, thread: Some(thread) })
    }

    pub fn send(&self, command: Command) {
        // A stopped thread has already sent its error.
        let _ = self.commands.send(command);
    }

    /// The next event; `None` once the camera has stopped.
    pub fn recv(&self) -> Option<Event> {
        self.events.recv().ok()
    }

    /// The next event within `wait`; `Err(true)` once the camera has stopped.
    pub fn recv_timeout(&self, wait: Duration) -> Result<Event, bool> {
        self.events.recv_timeout(wait).map_err(|e| e == RecvTimeoutError::Disconnected)
    }

    /// The events waiting now.
    pub fn try_iter(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }

    /// Frames dropped because the receiver was behind. Frames the camera lost show as gaps in
    /// [`Frame::seq`].
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Stop the camera and wait for its thread (at most one frame time).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let Some(thread) = self.thread.take() else { return };
        // A closed command channel stops the thread; draining the events unblocks it if it
        // is waiting to send, and ends when it has gone.
        let (closed, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.commands, closed));
        while self.events.recv().is_ok() {}
        let _ = thread.join();
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(settings: &CameraSettings, commands: &Receiver<Command>, events: &SyncSender<Event>, dropped: &AtomicU64) {
    let (mut source, applied) = match crate::open(settings) {
        Ok(opened) => opened,
        Err(e) => {
            let _ = events.send(Event::Error(e));
            return;
        }
    };
    let opened = Event::Opened {
        device: source.device().clone(),
        modes: source.modes().to_vec(),
        mode: source.mode(),
        controls: source.controls(),
        applied,
        usb: source.usb().cloned(),
        block: source.block(),
    };
    if events.send(opened).is_err() {
        return;
    }
    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    if !handle(source.as_mut(), command, events) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        match source.next_frame() {
            Ok(frame) => match events.try_send(Event::Frame(frame)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => return,
            },
            Err(e) => {
                let _ = events.send(Event::Error(e));
                return;
            }
        }
    }
}

/// Carry out one command; false once the receiver has gone.
fn handle(source: &mut dyn FrameSource, command: Command, events: &SyncSender<Event>) -> bool {
    let out: Vec<Event> = match command {
        Command::Set(setting) => crate::apply(source, &[setting]).into_iter().map(Event::Control).collect(),
        Command::Apply(settings) => crate::apply(source, &settings).into_iter().map(Event::Control).collect(),
        Command::Refresh => vec![Event::Controls(source.controls())],
    };
    out.into_iter().all(|e| events.send(e).is_ok())
}

#[cfg(test)]
mod tests {
    use laser_core::camera::{ControlId, ControlMode};

    use super::*;
    use crate::synthetic;

    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn synthetic_capture_runs_and_stops() {
        let settings = CameraSettings { device: Some(synthetic::NAME.into()), ..CameraSettings::default() };
        let cap = Capture::start(settings).unwrap();
        let Ok(Event::Opened { mode, controls, .. }) = cap.recv_timeout(WAIT) else { panic!("not opened") };
        assert!(controls.iter().any(|c| c.id == ControlId::Gain));
        let mut frames = Vec::new();
        while frames.len() < 3 {
            if let Ok(Event::Frame(f)) = cap.recv_timeout(WAIT) {
                assert_eq!((f.width, f.height), (mode.width as usize, mode.height as usize));
                frames.push(f.seq);
            }
        }
        assert!(frames.windows(2).all(|w| w[1] > w[0]), "{frames:?}");
        cap.send(Command::Set(ControlSetting { id: ControlId::Gain, mode: ControlMode::Manual(0) }));
        let applied = loop {
            match cap.recv_timeout(WAIT) {
                Ok(Event::Control(a)) => break a,
                Ok(_) => {}
                Err(_) => panic!("no answer"),
            }
        };
        assert!(applied.held(), "{applied:?}");
        cap.stop();
    }

    #[test]
    fn unknown_camera_reports_and_ends() {
        let settings = CameraSettings { device: Some("no such camera".into()), ..CameraSettings::default() };
        let cap = Capture::start(settings).unwrap();
        assert!(matches!(cap.recv_timeout(WAIT), Ok(Event::Error(CameraError::NotFound(_)))));
        assert_eq!(cap.recv_timeout(WAIT), Err(true));
    }
}
