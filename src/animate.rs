//! The cloud building up: every word flies in and settles on its spot with a damped spring, then
//! the finished cloud holds. Written as an animated GIF or an MP4 by flipcard's `filmstrip`.
//!
//! The spots come from [`crate::generate`], so the words end exactly where the static cloud has
//! them, and the last frame is the PNG on a background. The spring is integrated at a fixed step
//! with plain arithmetic and `sqrt`, so the same cloud and seed give the same frames everywhere.

use anyhow::{Context, Result, bail};
use filmstrip::Frame;
use tiny_skia::{FillRule, Paint, Pixmap, Transform};

use crate::color::Rgb;
use crate::layout::{Cloud, Rng};

/// The background of a light film: GIF and MP4 frames have no transparency.
pub const LIGHT_BACKGROUND: &str = "#ffffff";
/// The background of a dark film: GitHub's dark theme.
pub const DARK_BACKGROUND: &str = "#0d1117";

/// Physics steps per second, whatever the frame rate.
const STEPS: u32 = 600;
/// Damping ratio of the spring: below 1, so a word overshoots its spot a little.
const DAMPING: f64 = 0.55;
/// The longest flight of one word, in seconds; shorter when the build-up is short.
const FLIGHT: f64 = 1.4;
/// Share of the flight in which a word fades in.
const FADE: f64 = 0.2;

/// How the cloud builds up.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    /// Frames per second.
    pub fps: u16,
    /// Seconds until the last word has settled.
    pub seconds: f32,
    /// Seconds the finished cloud stays before the film loops.
    pub hold: f32,
    /// Seed for the start points and delays.
    pub seed: u64,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            fps: 30,
            seconds: 4.0,
            hold: 2.0,
            seed: 90,
        }
    }
}

/// One word on its way: its outline at the spot, where it starts and its spring.
#[derive(Debug)]
struct Flight {
    path: tiny_skia::Path,
    centre: (f32, f32),
    /// Offset of the start from the spot, canvas px.
    from: (f32, f32),
    /// The step it starts at.
    delay: usize,
    /// The share of `from` left at each step after the start; 0 after the last.
    curve: Vec<f32>,
    colour: Rgb,
}

/// A planned film: every word's flight, and the frame count of the build-up and of the hold.
#[derive(Debug)]
struct Film {
    flights: Vec<Flight>,
    size: (u32, u32),
    background: Rgb,
    fps: u16,
    /// Steps a word takes to fade in.
    fade: usize,
    frames: usize,
    hold: usize,
}

impl Film {
    /// Plan the flights for `cloud`: heaviest words first, fill copies last. `even` rounds the
    /// frame up to even sides (H.264 needs them).
    fn plan(
        cloud: &Cloud,
        motion: &Motion,
        background: &str,
        dark: bool,
        even: bool,
    ) -> Result<Self> {
        if motion.fps == 0 {
            bail!("fps: expected 1 or more, got \"0\"");
        }
        if !(motion.seconds.is_finite() && motion.seconds > 0.0) {
            bail!(
                "seconds: expected a number above 0, got \"{}\"",
                motion.seconds
            );
        }
        if !(motion.hold.is_finite() && motion.hold >= 0.0) {
            bail!("hold: expected 0 or more, got \"{}\"", motion.hold);
        }
        let background: Rgb = background.parse()?;
        let (width, height) = (cloud.width as f32, cloud.height as f32);
        let seconds = f64::from(motion.seconds);
        let flight = FLIGHT.min(0.45 * seconds);
        let span = seconds - flight;
        let mut order: Vec<_> = cloud.placed.iter().collect();
        order.sort_by(|a, b| a.copy.cmp(&b.copy).then(b.size().total_cmp(&a.size())));
        let last = order.len().saturating_sub(1).max(1) as f64;
        let mut rng = Rng(motion.seed ^ 0xF11E_C10D);
        let mut flights = Vec::with_capacity(order.len());
        for (rank, placed) in order.into_iter().enumerate() {
            let Some(path) = cloud.path(placed) else {
                continue;
            };
            let bounds = path.bounds();
            let centre = (
                (bounds.left() + bounds.right()) / 2.0,
                (bounds.top() + bounds.bottom()) / 2.0,
            );
            // Out from the canvas centre, a little to the side, from beyond the neighbours.
            let (mut dx, mut dy) = (centre.0 / width - 0.5, centre.1 / height - 0.5);
            let length = (dx * dx + dy * dy).sqrt();
            if length < 0.02 {
                (dx, dy) = (rng.f32() - 0.5, rng.f32() - 0.5);
            }
            let side = (rng.f32() - 0.5) * 0.6;
            (dx, dy) = (dx - side * dy, dy + side * dx);
            let length = (dx * dx + dy * dy).sqrt().max(1e-6);
            let distance = (0.25 + 0.15 * rng.f32()) * (width + height) / 2.0;
            let from = (dx / length * distance, dy / length * distance);
            // Light words wait longer; the square root leaves the heavy ones room to land.
            let jitter = f64::from(rng.f32() - 0.5) * 1.5;
            let start = ((rank as f64 + jitter) / last).clamp(0.0, 1.0).sqrt() * span;
            let omega = 6.0 / (DAMPING * flight) * (0.9 + 0.2 * f64::from(rng.f32()));
            flights.push(Flight {
                path,
                centre,
                from,
                delay: (start * f64::from(STEPS)) as usize,
                curve: spring(omega, (flight * f64::from(STEPS)) as usize),
                colour: placed.fill(dark).parse()?,
            });
        }
        let round_up = |side: u32| if even { side + side % 2 } else { side };
        Ok(Self {
            flights,
            size: (round_up(cloud.width), round_up(cloud.height)),
            background,
            fps: motion.fps,
            fade: (FADE * flight * f64::from(STEPS)) as usize,
            frames: (seconds * f64::from(motion.fps)).ceil() as usize + 1,
            hold: (f64::from(motion.hold) * f64::from(motion.fps)).round() as usize,
        })
    }

    /// Draw frame `index` of the build-up; from the last one on, the finished cloud.
    fn frame(&self, index: usize) -> Result<Frame> {
        let (width, height) = self.size;
        let mut pixmap = Pixmap::new(width, height).context("allocate the frame")?;
        let Rgb { red, green, blue } = self.background;
        pixmap.fill(tiny_skia::Color::from_rgba8(red, green, blue, 255));
        let step = index * STEPS as usize / usize::from(self.fps);
        for flight in &self.flights {
            let Some(local) = step.checked_sub(flight.delay) else {
                continue;
            };
            let left = flight.curve.get(local).copied().unwrap_or(0.0);
            let alpha = if local >= self.fade {
                255
            } else {
                (255 * local / self.fade) as u8
            };
            if alpha == 0 {
                continue;
            }
            let mut paint = Paint::default();
            let Rgb { red, green, blue } = flight.colour;
            paint.set_color_rgba8(red, green, blue, alpha);
            paint.anti_alias = true;
            // Scaled about its centre, small at the start, a touch big on the overshoot.
            let scale = 1.0 - 0.5 * left;
            let (cx, cy) = flight.centre;
            let transform = Transform::from_row(
                scale,
                0.0,
                0.0,
                scale,
                cx + flight.from.0 * left - scale * cx,
                cy + flight.from.1 * left - scale * cy,
            );
            pixmap.fill_path(&flight.path, &paint, FillRule::Winding, transform, None);
        }
        let rgb = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [px[0], px[1], px[2]])
            .collect();
        Ok(Frame { width, height, rgb })
    }

    /// Hand every frame to `push` with how often it shows: the build-up once each, the last
    /// frame also for the hold.
    fn run(&self, mut push: impl FnMut(&Frame, usize) -> Result<()>) -> Result<()> {
        for index in 0..self.frames {
            let count = if index + 1 == self.frames {
                1 + self.hold
            } else {
                1
            };
            push(&self.frame(index)?, count)?;
        }
        Ok(())
    }
}

/// The share of the start offset left at each step of a damped spring released from 1 at rest,
/// `steps` long; semi-implicit Euler at the fixed step.
fn spring(omega: f64, steps: usize) -> Vec<f32> {
    let dt = 1.0 / f64::from(STEPS);
    let (mut position, mut velocity) = (1.0f64, 0.0f64);
    (0..steps)
        .map(|_| {
            let now = position as f32;
            velocity += (-omega * omega * position - 2.0 * DAMPING * omega * velocity) * dt;
            position += velocity * dt;
            now
        })
        .collect()
}

impl Cloud {
    /// Write the cloud building up as an animated GIF that loops, on `background` (`#rrggbb`,
    /// e.g. [`LIGHT_BACKGROUND`]); `dark` picks the `[colours-dark]` palette.
    pub fn gif(&self, motion: &Motion, background: &str, dark: bool) -> Result<Vec<u8>> {
        let film = Film::plan(self, motion, background, dark, false)?;
        let last = film.frame(film.frames - 1)?;
        let (width, height) = film.size;
        let mut writer = filmstrip::gif::Writer::new(width, height, &[last], film.fps, 255)?;
        film.run(|frame, count| writer.push(frame, count))?;
        writer.finish().context("write the gif")
    }

    /// Write the cloud building up as an H.264 MP4, like [`Cloud::gif`]. An odd side gets one
    /// more px of background.
    pub fn mp4(&self, motion: &Motion, background: &str, dark: bool) -> Result<Vec<u8>> {
        let film = Film::plan(self, motion, background, dark, true)?;
        let (width, height) = film.size;
        let mut writer = filmstrip::video::Writer::new(width, height, film.fps)?;
        film.run(|frame, count| writer.push(frame, count))?;
        writer.finish().context("write the mp4")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Options, Word};

    fn cloud(seed: u64) -> Cloud {
        let words: Vec<Word> = [
            ("Kalsarikännit", 10.0),
            ("Sisu", 7.0),
            ("Kac", 5.0),
            ("Gapa", 3.0),
        ]
        .into_iter()
        .map(|(text, weight)| Word {
            text: text.into(),
            weight,
            color: "#ff4f00".into(),
            color_dark: "#e6e7ea".into(),
            related: vec![],
            variants: vec![],
        })
        .collect();
        let options = Options {
            width: 161,
            height: 91,
            seed,
            ..Options::default()
        };
        crate::generate(&words, crate::DEFAULT_FONT, &options, None).expect("layout")
    }

    fn motion() -> Motion {
        Motion {
            fps: 10,
            seconds: 1.0,
            hold: 0.5,
            seed: 7,
        }
    }

    fn frames(cloud: &Cloud, motion: &Motion) -> Vec<(Vec<u8>, usize)> {
        let film = Film::plan(cloud, motion, DARK_BACKGROUND, true, false).expect("plan");
        let mut frames = Vec::new();
        film.run(|frame, count| {
            frames.push((frame.rgb.clone(), count));
            Ok(())
        })
        .expect("frames");
        frames
    }

    #[test]
    fn same_seed_same_frames_and_another_seed_moves_them() {
        let cloud = cloud(3);
        let a = frames(&cloud, &motion());
        assert_eq!(a, frames(&cloud, &motion()));
        let other = Motion {
            seed: 8,
            ..motion()
        };
        assert_ne!(a, frames(&cloud, &other));
        // 1 s at 10 fps: frames 0 to 10, the last one held 5 more.
        assert_eq!(a.len(), 11);
        assert_eq!(a.last().map(|f| f.1), Some(6));
    }

    #[test]
    fn the_film_starts_empty_and_ends_on_the_static_cloud() {
        let cloud = cloud(3);
        let frames = frames(&cloud, &motion());
        let background = [0x0d, 0x11, 0x17];
        assert!(
            frames[0]
                .0
                .as_chunks::<3>()
                .0
                .iter()
                .all(|px| *px == background)
        );
        let png = cloud.png(1.0, Some(DARK_BACKGROUND), true).expect("png");
        let decoded = image::load_from_memory(&png).expect("decode").to_rgb8();
        assert_eq!(frames.last().map(|f| &f.0), Some(decoded.as_raw()));
    }

    #[test]
    fn the_spring_overshoots_a_little_and_comes_to_rest() {
        let curve = spring(
            6.0 / (DAMPING * FLIGHT),
            (FLIGHT * f64::from(STEPS)) as usize,
        );
        let lowest = curve.iter().copied().fold(f32::MAX, f32::min);
        assert!((-0.2..-0.05).contains(&lowest), "overshoot {lowest}");
        assert!(curve.last().is_some_and(|e| e.abs() < 0.01));
    }

    #[test]
    fn gif_and_mp4_come_out_and_bad_values_are_named() {
        let cloud = cloud(3);
        let gif = cloud.gif(&motion(), LIGHT_BACKGROUND, false).expect("gif");
        assert!(gif.starts_with(b"GIF89a"));
        // 161 x 91 is odd: the MP4 pads it.
        let mp4 = cloud.mp4(&motion(), DARK_BACKGROUND, true).expect("mp4");
        assert_eq!(&mp4[4..8], b"ftyp");
        let error = cloud
            .gif(
                &Motion {
                    seconds: 0.0,
                    ..motion()
                },
                LIGHT_BACKGROUND,
                false,
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "seconds: expected a number above 0, got \"0\""
        );
    }
}
