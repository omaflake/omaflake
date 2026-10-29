// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera frames into the one format the guest agent takes (spec §11.9, `0x0202`): 1280×720 NV12, video range, the
//! Y plane then the interleaved CbCr plane, no padding – 1,382,400 bytes, the same as the Mac's AVFoundation output.
//!
//! Inputs a V4L2 camera delivers: NV12 (copied), YUYV 4:2:2 (chroma of the even rows), MJPEG (decoded with zune-jpeg to
//! RGB, then BT.709 video range). Another size is scaled to 1280×720 by nearest neighbour (no aspect correction – a
//! 4:3 camera looks stretched; OPEN whether to letterbox instead). YUYV and NV12 are passed on in the colour space
//! the camera uses (webcams mostly BT.601) while the guest's device says Rec. 709 – a slight hue shift, accepted.

use crate::agent_protocol::AgentProtocol;

pub const WIDTH: usize = 1280;
pub const HEIGHT: usize = 720;
pub const FRAME_BYTES: usize = AgentProtocol::CAMERA_FRAME_BYTES;

/// A camera picture format, as V4L2's fourcc
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Nv12,
    Yuyv,
    Mjpeg,
}

impl PixelFormat {
    /// In the order a camera is asked for them: no conversion, a cheap one, decoding
    pub const PREFERRED: [PixelFormat; 3] = [PixelFormat::Nv12, PixelFormat::Yuyv, PixelFormat::Mjpeg];

    pub fn fourcc(self) -> u32 {
        u32::from_le_bytes(*match self {
            PixelFormat::Nv12 => b"NV12",
            PixelFormat::Yuyv => b"YUYV",
            PixelFormat::Mjpeg => b"MJPG",
        })
    }

    /// "NV12", "YUYV", "MJPEG" – for the camera's status
    pub fn name(self) -> &'static str {
        match self {
            PixelFormat::Nv12 => "NV12",
            PixelFormat::Yuyv => "YUYV",
            PixelFormat::Mjpeg => "MJPEG",
        }
    }

    pub fn from_fourcc(v: u32) -> Option<PixelFormat> {
        Self::PREFERRED.into_iter().find(|f| f.fourcc() == v)
    }
}

/// One captured picture as the camera delivered it
#[derive(Debug, Clone, Copy)]
pub struct Picture<'a> {
    pub format: PixelFormat,
    pub width: usize,
    pub height: usize,
    /// Bytes per row of the (first) plane; 0 = packed
    pub stride: usize,
    pub data: &'a [u8],
}

/// The picture as a 1280×720 NV12 frame; None when the data is too short for its size or does not decode
pub fn to_nv12(p: &Picture) -> Option<Vec<u8>> {
    if p.width == 0 || p.height == 0 {
        return None;
    }
    match p.format {
        PixelFormat::Nv12 => {
            let stride = if p.stride == 0 { p.width } else { p.stride };
            // Y plane, then the CbCr plane at half height with the same stride
            // the last row of the CbCr plane need not be padded
            let need = stride * p.height + stride * (p.height.div_ceil(2) - 1) + p.width;
            if stride < p.width || p.data.len() < need {
                return None;
            }
            let uv = stride * p.height;
            Some(sample(
                p.width,
                p.height,
                |x, y| p.data[y * stride + x],
                |x, y| {
                    let i = uv + (y / 2) * stride + (x & !1);
                    (p.data[i], p.data[i + 1])
                },
            ))
        }
        PixelFormat::Yuyv => {
            let stride = if p.stride == 0 { p.width * 2 } else { p.stride };
            if stride < p.width * 2
                || !p.width.is_multiple_of(2)
                || p.data.len() < stride * (p.height - 1) + p.width * 2
            {
                return None;
            }
            Some(sample(
                p.width,
                p.height,
                |x, y| p.data[y * stride + x * 2],
                |x, y| {
                    let i = y * stride + (x & !1) * 2;
                    (p.data[i + 1], p.data[i + 3])
                },
            ))
        }
        PixelFormat::Mjpeg => mjpeg_to_nv12(p.data),
    }
}

/// Decodes a JPEG (an MJPEG frame; zune-jpeg also takes those without Huffman tables) and converts it
pub fn mjpeg_to_nv12(jpeg: &[u8]) -> Option<Vec<u8>> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(jpeg), options);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = pixels.len().checked_div(w * h).unwrap_or(0);
    rgb_to_nv12(&pixels, w, h, channels)
}

/// Full-range RGB (or gray, `channels` 1; RGBA 4) to NV12 in BT.709 video range, scaled to 1280×720
pub fn rgb_to_nv12(pixels: &[u8], width: usize, height: usize, channels: usize) -> Option<Vec<u8>> {
    if !(1..=4).contains(&channels) || channels == 2 || pixels.len() < width * height * channels || width * height == 0
    {
        return None;
    }
    let rgb = |x: usize, y: usize| -> (i32, i32, i32) {
        let i = (y * width + x) * channels;
        if channels == 1 {
            let g = pixels[i] as i32;
            (g, g, g)
        } else {
            (pixels[i] as i32, pixels[i + 1] as i32, pixels[i + 2] as i32)
        }
    };
    Some(sample(
        width,
        height,
        |x, y| {
            let (r, g, b) = rgb(x, y);
            (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8
        },
        |x, y| {
            let (r, g, b) = rgb(x, y);
            let u = ((-26 * r - 86 * g + 112 * b + 128) >> 8) + 128; // rows sum to 0: gray stays neutral
            let v = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
            (u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
        },
    ))
}

/// Builds the 1280×720 NV12 frame from a source of `w`×`h` pixels: `luma(x, y)` and `chroma(x, y)` (Cb, Cr) at source
/// coordinates, picked by nearest neighbour (chroma at the top left pixel of each 2×2 block)
fn sample(w: usize, h: usize, luma: impl Fn(usize, usize) -> u8, chroma: impl Fn(usize, usize) -> (u8, u8)) -> Vec<u8> {
    let xs: Vec<usize> = (0..WIDTH).map(|x| x * w / WIDTH).collect();
    let ys: Vec<usize> = (0..HEIGHT).map(|y| y * h / HEIGHT).collect();
    let mut out = vec![0u8; FRAME_BYTES];
    let (y_plane, uv_plane) = out.split_at_mut(WIDTH * HEIGHT);
    for (row, &sy) in y_plane.as_chunks_mut::<WIDTH>().0.iter_mut().zip(&ys) {
        for (px, &sx) in row.iter_mut().zip(&xs) {
            *px = luma(sx, sy);
        }
    }
    for (row, sy) in uv_plane.as_chunks_mut::<WIDTH>().0.iter_mut().zip(ys.iter().step_by(2)) {
        for (pair, sx) in row.as_chunks_mut::<2>().0.iter_mut().zip(xs.iter().step_by(2)) {
            let (u, v) = chroma(*sx, *sy);
            pair[0] = u;
            pair[1] = v;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn y_at(f: &[u8], x: usize, y: usize) -> u8 {
        f[y * WIDTH + x]
    }

    fn uv_at(f: &[u8], x: usize, y: usize) -> (u8, u8) {
        let i = WIDTH * HEIGHT + (y / 2) * WIDTH + (x & !1);
        (f[i], f[i + 1])
    }

    #[test]
    fn fourcc_codes() {
        assert_eq!(PixelFormat::Yuyv.fourcc(), 0x5659_5559);
        assert_eq!(PixelFormat::Mjpeg.fourcc(), 0x4750_4a4d);
        assert_eq!(PixelFormat::from_fourcc(PixelFormat::Nv12.fourcc()), Some(PixelFormat::Nv12));
        assert_eq!(PixelFormat::from_fourcc(0), None);
    }

    #[test]
    fn nv12_of_the_right_size_is_copied() {
        let mut src = vec![0u8; FRAME_BYTES];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let p = Picture { format: PixelFormat::Nv12, width: WIDTH, height: HEIGHT, stride: 0, data: &src };
        assert_eq!(to_nv12(&p).unwrap(), src);
    }

    #[test]
    fn nv12_with_padded_rows() {
        let stride = WIDTH + 64;
        let mut src = vec![0xEEu8; stride * HEIGHT * 3 / 2];
        for y in 0..HEIGHT {
            src[y * stride..y * stride + WIDTH].fill(100);
        }
        for y in 0..HEIGHT / 2 {
            let row = stride * HEIGHT + y * stride;
            for x in (0..WIDTH).step_by(2) {
                src[row + x] = 90;
                src[row + x + 1] = 200;
            }
        }
        let f =
            to_nv12(&Picture { format: PixelFormat::Nv12, width: WIDTH, height: HEIGHT, stride, data: &src }).unwrap();
        assert!(f[..WIDTH * HEIGHT].iter().all(|&b| b == 100), "padding must not leak into the picture");
        assert!(f[WIDTH * HEIGHT..].chunks(2).all(|c| c == [90, 200]));
    }

    /// A synthetic YUYV picture: left half Y 50 / Cb 60 / Cr 70, right half Y 150 / Cb 160 / Cr 170
    fn yuyv(w: usize, h: usize) -> Vec<u8> {
        let mut src = Vec::with_capacity(w * h * 2);
        for _ in 0..h {
            for x in (0..w).step_by(2) {
                let (y, u, v) = if x < w / 2 { (50, 60, 70) } else { (150, 160, 170) };
                src.extend_from_slice(&[y, u, y + 1, v]);
            }
        }
        src
    }

    #[test]
    fn yuyv_720p() {
        let src = yuyv(WIDTH, HEIGHT);
        let f = to_nv12(&Picture { format: PixelFormat::Yuyv, width: WIDTH, height: HEIGHT, stride: 0, data: &src })
            .unwrap();
        assert_eq!(f.len(), FRAME_BYTES);
        assert_eq!((y_at(&f, 0, 0), y_at(&f, 1, 0)), (50, 51));
        assert_eq!(y_at(&f, WIDTH - 1, HEIGHT - 1), 151);
        assert_eq!(uv_at(&f, 10, 10), (60, 70));
        assert_eq!(uv_at(&f, WIDTH - 2, HEIGHT - 2), (160, 170));
    }

    #[test]
    fn yuyv_640x480_is_scaled() {
        let src = yuyv(640, 480);
        let f =
            to_nv12(&Picture { format: PixelFormat::Yuyv, width: 640, height: 480, stride: 0, data: &src }).unwrap();
        assert_eq!(f.len(), FRAME_BYTES);
        assert_eq!(y_at(&f, WIDTH / 4, HEIGHT / 2), 50);
        assert_eq!(y_at(&f, WIDTH * 3 / 4, HEIGHT / 2), 150);
        assert_eq!(uv_at(&f, WIDTH * 3 / 4, 0), (160, 170));
    }

    #[test]
    fn short_or_empty_data_is_refused() {
        let src = yuyv(WIDTH, HEIGHT);
        let short = &src[..src.len() - 1];
        assert!(
            to_nv12(&Picture { format: PixelFormat::Yuyv, width: WIDTH, height: HEIGHT, stride: 0, data: short })
                .is_none()
        );
        assert!(to_nv12(&Picture { format: PixelFormat::Nv12, width: 0, height: 0, stride: 0, data: &[] }).is_none());
        assert!(mjpeg_to_nv12(b"not a jpeg").is_none());
    }

    #[test]
    fn rgb_to_video_range() {
        // white, black, red: BT.709 video range (Y 16–235, chroma 16–240)
        let f = rgb_to_nv12(&[255, 255, 255], 1, 1, 3).unwrap();
        assert_eq!((y_at(&f, 0, 0), uv_at(&f, 0, 0)), (235, (128, 128)));
        let f = rgb_to_nv12(&[0, 0, 0], 1, 1, 3).unwrap();
        assert_eq!((y_at(&f, 5, 5), uv_at(&f, 5, 5)), (16, (128, 128)));
        let f = rgb_to_nv12(&[255, 0, 0], 1, 1, 3).unwrap();
        assert_eq!((y_at(&f, 0, 0), uv_at(&f, 0, 0)), (63, (102, 240)));
        assert!(rgb_to_nv12(&[0, 0], 1, 1, 2).is_none());
    }

    /// A real JPEG as a webcam's MJPEG frame is one (16×16, left half white, right half black, 4:2:0, made with
    /// `ffmpeg -f lavfi -i "color=white:s=16x16,drawbox=x=8:y=0:w=8:h=16:color=black:t=fill" -frames:v 1 -q:v 1
    /// -pix_fmt yuvj420p half.jpg`)
    #[test]
    fn mjpeg_frame() {
        let jpeg = include_bytes!("../../testdata/camera/half.jpg");
        let f = mjpeg_to_nv12(jpeg).unwrap();
        assert_eq!(f.len(), FRAME_BYTES);
        assert!(y_at(&f, 10, 300) > 225, "left half white");
        assert!(y_at(&f, WIDTH - 10, 300) < 26, "right half black");
        let (u, v) = uv_at(&f, 10, 300);
        assert!(u.abs_diff(128) < 4 && v.abs_diff(128) < 4, "gray has no colour");
    }
}
