//! A view of an 8-bit luma plane: the Y plane of NV12 or YUYV, or a grey image.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Which way the laser line runs on the sensor. The reading is the line's position across it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineAxis {
    /// The line runs top to bottom; the reading is along x.
    #[default]
    Vertical,
    /// The line runs left to right; the reading is along y.
    Horizontal,
}

/// How the camera's image is turned to show the world: the laser line level across the view,
/// and up in the view up in the room. The image is mirrored left to right first (when
/// `mirrored`), then turned clockwise by `quarter_turns` × 90°.
///
/// Turning the camera 90° in its block puts the sensor's long side up and down, so the heights
/// range over the long side. The orientation says which way the line runs on the sensor (so the
/// finder reads across it) and which way is up: a block that rises takes the sensor up past the
/// beam, so the line moves down the view, and the height goes up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Orientation {
    /// Clockwise quarter turns, 0 to 3.
    pub quarter_turns: u8,
    pub mirrored: bool,
}

impl Default for Orientation {
    /// Turned 90°: the long side of a 16:9 sensor measures the heights.
    fn default() -> Self {
        Self { quarter_turns: 1, mirrored: false }
    }
}

impl Orientation {
    /// The orientation that matches a line axis and a height sign from before orientations.
    pub fn from_axis(axis: LineAxis, invert: bool) -> Self {
        let quarter_turns = match (axis, invert) {
            (LineAxis::Horizontal, false) => 0,
            (LineAxis::Vertical, false) => 1,
            (LineAxis::Horizontal, true) => 2,
            (LineAxis::Vertical, true) => 3,
        };
        Self { quarter_turns, mirrored: false }
    }

    fn turns(self) -> u8 {
        self.quarter_turns % 4
    }

    /// Clockwise turn of the view, degrees.
    pub fn degrees(self) -> u16 {
        u16::from(self.turns()) * 90
    }

    /// The view turned a quarter clockwise.
    pub fn turned_cw(self) -> Self {
        Self { quarter_turns: (self.turns() + 1) % 4, ..self }
    }

    /// The view turned a quarter anticlockwise.
    pub fn turned_ccw(self) -> Self {
        Self { quarter_turns: (self.turns() + 3) % 4, ..self }
    }

    /// The view mirrored left to right, as it is shown now.
    pub fn flipped_left_right(self) -> Self {
        // Mirror ∘ turn(q) = turn(−q) ∘ mirror.
        Self { quarter_turns: (4 - self.turns()) % 4, mirrored: !self.mirrored }
    }

    /// The view mirrored top to bottom, as it is shown now.
    pub fn flipped_up_down(self) -> Self {
        // Mirroring top to bottom is mirroring left to right and turning half a turn.
        let lr = self.flipped_left_right();
        Self { quarter_turns: (lr.turns() + 2) % 4, ..lr }
    }

    /// Which way the laser line runs on the image when it is level across the view.
    pub fn axis(self) -> LineAxis {
        if self.turns() % 2 == 1 { LineAxis::Vertical } else { LineAxis::Horizontal }
    }

    /// Heights go up when the reading in image px goes down: the line moving down the view
    /// is the reading getting smaller.
    pub fn invert(self) -> bool {
        match self.turns() {
            0 => false,
            1 => self.mirrored,
            2 => true,
            _ => !self.mirrored,
        }
    }

    /// The view's size for an image `width` × `height`.
    pub fn view_size(self, width: f64, height: f64) -> (f64, f64) {
        if self.turns() % 2 == 1 { (height, width) } else { (width, height) }
    }

    /// A point on the image (`x` from 0 to `width`) as a point on the view.
    pub fn to_view(self, x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
        let x = if self.mirrored { width - x } else { x };
        match self.turns() {
            0 => (x, y),
            1 => (height - y, x),
            2 => (width - x, height - y),
            _ => (y, width - x),
        }
    }

    /// A point on the view as a point on the image `width` × `height`.
    pub fn to_image(self, vx: f64, vy: f64, width: f64, height: f64) -> (f64, f64) {
        let (x, y) = match self.turns() {
            0 => (vx, vy),
            1 => (vy, height - vx),
            2 => (width - vx, height - vy),
            _ => (width - vy, vx),
        };
        (if self.mirrored { width - x } else { x }, y)
    }

    /// Words for the orientation.
    pub fn describe(self) -> String {
        let turn = match self.turns() {
            0 => "Not turned",
            1 => "Turned 90° clockwise",
            2 => "Turned 180°",
            _ => "Turned 90° anticlockwise",
        };
        if self.mirrored { format!("{turn}, mirrored") } else { turn.to_owned() }
    }
}

/// A borrowed luma plane. Rows may have padding (`stride` ≥ `width`).
#[derive(Clone, Copy, Debug)]
pub struct Luma<'a> {
    width: usize,
    height: usize,
    stride: usize,
    data: &'a [u8],
}

impl<'a> Luma<'a> {
    pub fn new(width: usize, height: usize, stride: usize, data: &'a [u8]) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::Empty);
        }
        if stride < width {
            return Err(FrameError::Stride { width, stride });
        }
        let need = stride * (height - 1) + width;
        if data.len() < need {
            return Err(FrameError::Short { need, got: data.len() });
        }
        Ok(Self { width, height, stride, data })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Row `y`, without padding.
    pub fn row(&self, y: usize) -> &'a [u8] {
        let start = y * self.stride;
        &self.data[start..start + self.width]
    }

    /// The part of the frame inside `roi`, which must lie inside the frame (see [`Roi::clamped`]).
    pub fn crop(&self, roi: Roi) -> Luma<'a> {
        let start = roi.y * self.stride + roi.x;
        Luma { width: roi.width, height: roi.height, stride: self.stride, data: &self.data[start..] }
    }
}

/// A rectangle of the frame, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roi {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Roi {
    /// The part inside a frame `width` × `height`, or `None` if nothing is left.
    pub fn clamped(self, width: usize, height: usize) -> Option<Roi> {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let w = self.width.min(width - x);
        let h = self.height.min(height - y);
        (w > 0 && h > 0).then_some(Roi { x, y, width: w, height: h })
    }
}

/// Why a buffer is not a valid luma plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    Empty,
    Stride { width: usize, stride: usize },
    Short { need: usize, got: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the frame has no pixels"),
            Self::Stride { width, stride } => write!(f, "row stride {stride} is less than the width {width}"),
            Self::Short { need, got } => write!(f, "the frame needs {need} bytes but has {got}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// An owned luma image with no row padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LumaImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl LumaImage {
    pub fn new(width: usize, height: usize, fill: u8) -> Self {
        Self { width, height, data: vec![fill; width * height] }
    }

    pub fn view(&self) -> Result<Luma<'_>, FrameError> {
        Luma::new(self.width, self.height, self.width, &self.data)
    }

    /// The Y bytes of packed YUYV (Y0 U Y1 V), rows `stride` bytes apart.
    pub fn from_yuyv(width: usize, height: usize, stride: usize, data: &[u8]) -> Result<Self, FrameError> {
        Self::check(width, height, stride, 2 * width, data)?;
        let mut out = Vec::with_capacity(width * height);
        for y in 0..height {
            let row = &data[y * stride..y * stride + 2 * width];
            out.extend(row.iter().step_by(2));
        }
        Ok(Self { width, height, data: out })
    }

    /// Luma of RGBA pixels with the BT.601 weights a webcam uses for its Y plane, rows `stride`
    /// bytes apart. A browser camera gives RGBA.
    pub fn from_rgba(width: usize, height: usize, stride: usize, data: &[u8]) -> Result<Self, FrameError> {
        Self::check(width, height, stride, 4 * width, data)?;
        let mut out = Vec::with_capacity(width * height);
        for y in 0..height {
            let row = &data[y * stride..y * stride + 4 * width];
            out.extend(row.chunks_exact(4).map(|p| {
                let (r, g, b) = (u32::from(p[0]), u32::from(p[1]), u32::from(p[2]));
                // 16 + (65.738 R + 129.057 G + 25.064 B) / 256, rounded.
                ((16 * 256 + 128 + 66 * r + 129 * g + 25 * b) >> 8) as u8
            }));
        }
        Ok(Self { width, height, data: out })
    }

    fn check(width: usize, height: usize, stride: usize, row_bytes: usize, data: &[u8]) -> Result<(), FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::Empty);
        }
        if stride < row_bytes {
            return Err(FrameError::Stride { width: row_bytes, stride });
        }
        let need = stride * (height - 1) + row_bytes;
        if data.len() < need {
            return Err(FrameError::Short { need, got: data.len() });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_buffer() {
        let data = [0u8; 10];
        assert_eq!(Luma::new(4, 3, 4, &data).unwrap_err(), FrameError::Short { need: 12, got: 10 });
        assert_eq!(Luma::new(4, 2, 3, &data).unwrap_err(), FrameError::Stride { width: 4, stride: 3 });
        let f = Luma::new(4, 2, 6, &data).unwrap();
        assert_eq!(f.row(1).len(), 4);
    }

    #[test]
    fn crop_and_conversions() {
        let data: Vec<u8> = (0..24).collect();
        let f = Luma::new(4, 3, 8, &data).unwrap();
        let roi = Roi { x: 1, y: 1, width: 9, height: 9 }.clamped(4, 3).unwrap();
        assert_eq!(roi, Roi { x: 1, y: 1, width: 3, height: 2 });
        let c = f.crop(roi);
        assert_eq!((c.width(), c.height()), (3, 2));
        assert_eq!(c.row(1), &[17, 18, 19]);
        assert_eq!(Roi { x: 4, y: 0, width: 2, height: 2 }.clamped(4, 3), None);

        let yuyv = [10, 128, 20, 128, 0, 0, 30, 1, 40, 2, 0, 0];
        let y = LumaImage::from_yuyv(2, 2, 6, &yuyv).unwrap();
        assert_eq!(y.data, [10, 20, 30, 40]);
        assert!(LumaImage::from_yuyv(2, 2, 3, &yuyv).is_err());

        let rgba = [0, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 255];
        let y = LumaImage::from_rgba(3, 1, 12, &rgba).unwrap();
        assert_eq!(y.data, [16, 235, 82]);
    }

    fn all_orientations() -> impl Iterator<Item = Orientation> {
        (0..8).map(|i| Orientation { quarter_turns: i % 4, mirrored: i >= 4 })
    }

    #[test]
    fn orientations_turn_and_flip_the_view_as_shown() {
        let (w, h) = (16.0, 9.0);
        let p = (3.0, 2.0);
        for o in all_orientations() {
            let (vw, vh) = o.view_size(w, h);
            let (vx, vy) = o.to_view(p.0, p.1, w, h);
            assert_eq!(o.to_image(vx, vy, w, h), p, "{o:?}");
            assert_eq!(o.turned_cw().to_view(p.0, p.1, w, h), (vh - vy, vx), "{o:?}");
            assert_eq!(o.turned_ccw().to_view(p.0, p.1, w, h), (vy, vw - vx), "{o:?}");
            assert_eq!(o.flipped_left_right().to_view(p.0, p.1, w, h), (vw - vx, vy), "{o:?}");
            assert_eq!(o.flipped_up_down().to_view(p.0, p.1, w, h), (vx, vh - vy), "{o:?}");
            assert_eq!(o.turned_cw().turned_ccw(), o);
            assert_eq!(o.flipped_left_right().flipped_left_right(), o);
        }
        // Four orientations put the long side of a 16:9 image up and down the view.
        assert_eq!(all_orientations().filter(|o| o.view_size(w, h).1 > o.view_size(w, h).0).count(), 4);
    }

    #[test]
    fn heights_go_up_as_the_line_moves_down_the_view() {
        let (w, h) = (16.0, 9.0);
        for o in all_orientations() {
            // One px further along the reading, on the image.
            let (a, b) = match o.axis() {
                LineAxis::Vertical => ((4.0, 5.0), (5.0, 5.0)),
                LineAxis::Horizontal => ((5.0, 4.0), (5.0, 5.0)),
            };
            let down = o.to_view(b.0, b.1, w, h).1 - o.to_view(a.0, a.1, w, h).1;
            // The line level across the view: moving along the reading moves it up or down only.
            assert_eq!(down.abs(), 1.0, "{o:?}");
            assert_eq!(o.invert(), down < 0.0, "{o:?}");
        }
        for axis in [LineAxis::Vertical, LineAxis::Horizontal] {
            for invert in [false, true] {
                let o = Orientation::from_axis(axis, invert);
                assert_eq!((o.axis(), o.invert()), (axis, invert));
            }
        }
    }
}
