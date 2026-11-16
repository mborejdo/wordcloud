//! The packer: shaping, coverage sprites, spiral placement, SVG and PNG output.

use std::fmt::{self, Write as _};
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Transform};

use crate::color::Rgb;
use crate::source::{Style, Word};

// ---------------------------------------------------------------- options

/// Canvas and layout settings for [`generate`].
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Canvas width in px.
    pub width: u32,
    /// Canvas height in px.
    pub height: u32,
    /// Layout seed. Same words + same seed = same cloud.
    pub seed: u64,
    /// Probability that a word (outside the top tier) is set vertically.
    pub vertical: f32,
    /// Minimum distance between two words, canvas px (plus 8% of the font size).
    pub gap: f32,
    /// Size contrast: `size = scale * (weight / max) ^ contrast`.
    pub contrast: f32,
    /// Grid pixels per canvas px (resolution). 2 is plenty; higher is slower and tighter.
    pub res: u32,
    /// Smallest font size in canvas px.
    pub min_size: f32,
    /// Fill the canvas, or the shape, edge to edge: a vertical word that does not fit is set
    /// horizontally instead, and small copies of the words fill every gap that is left. Off by
    /// default.
    pub fill: bool,
}

impl Options {
    /// Build the options from a word list's `[style]`, with defaults for what it leaves out.
    #[must_use]
    pub fn from_style(style: &Style) -> Self {
        Self {
            width: style.width.unwrap_or(800),
            height: style.height.unwrap_or(418),
            seed: style.seed.unwrap_or(90),
            vertical: style.vertical.unwrap_or(0.18).clamp(0.0, 1.0),
            gap: style.gap.unwrap_or(2.0),
            contrast: style.contrast.unwrap_or(1.5),
            res: 2,
            min_size: 8.0,
            fill: style.fill.unwrap_or(false),
        }
    }

    /// The layout grid in cells: the canvas times `res`.
    const fn grid_size(&self) -> (usize, usize) {
        (
            (self.width * self.res) as usize,
            (self.height * self.res) as usize,
        )
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::from_style(&Style::default())
    }
}

/// The built-in shape names, as an error message lists them.
const SHAPES: &str = "rect | ellipse | circle";

/// A built-in canvas shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    /// The plain canvas.
    #[default]
    Rect,
    /// The ellipse inscribed in the canvas.
    Ellipse,
    /// Same as `Ellipse`; use a square canvas for a circle.
    Circle,
}

impl Shape {
    /// Return the name used in the word list and on the command line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Ellipse => "ellipse",
            Self::Circle => "circle",
        }
    }
}

impl FromStr for Shape {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        match name {
            "rect" => Ok(Self::Rect),
            "ellipse" => Ok(Self::Ellipse),
            "circle" => Ok(Self::Circle),
            other => bail!("unknown shape `{other}` ({SHAPES})"),
        }
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Command-line values: the same names as in the word list.
#[cfg(feature = "cli")]
impl clap::ValueEnum for Shape {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Rect, Self::Ellipse, Self::Circle]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.name()))
    }
}

/// Blocked cells, sized to the layout grid (`res` included). Words only go where it is free.
pub struct MaskImage {
    blocked: Vec<bool>,
}

impl fmt::Debug for MaskImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let blocked = self.blocked.iter().filter(|&&b| b).count();
        f.debug_struct("MaskImage")
            .field("cells", &self.blocked.len())
            .field("blocked", &blocked)
            .finish()
    }
}

impl MaskImage {
    /// Open a shape by the name a word list or a command line gives it: a built-in one
    /// (`rect`, `ellipse`, `circle`) or an image file, relative to `base`. `invert` flips an
    /// image, see [`MaskImage::from_bytes`].
    pub fn open(shape: &str, base: &Path, options: &Options, invert: bool) -> Result<Self> {
        shape.parse().map_or_else(
            |_| {
                Self::load(&base.join(shape), options, invert)
                    .with_context(|| format!("unknown shape `{shape}` ({SHAPES} | image file)"))
            },
            |shape| Ok(Self::from_shape(shape, options)),
        )
    }

    /// Read a shape image from a file, see [`MaskImage::from_bytes`].
    pub fn load(path: &Path, options: &Options, invert: bool) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes, options, invert)
            .with_context(|| format!("decode {}", path.display()))
    }

    /// Build the mask from an encoded image (PNG or JPEG), stretched to the canvas: dark,
    /// opaque pixels are where words may go; `invert` flips that.
    pub fn from_bytes(bytes: &[u8], options: &Options, invert: bool) -> Result<Self> {
        let (width, height) = options.grid_size();
        let image = image::load_from_memory(bytes)?.to_luma_alpha8();
        let image = image::imageops::resize(
            &image,
            width as u32,
            height as u32,
            image::imageops::FilterType::Triangle,
        );
        let blocked = image
            .pixels()
            .map(|p| (p.0[0] < 128 && p.0[1] >= 128) == invert)
            .collect();
        Ok(Self { blocked })
    }

    /// Build the mask for a [`Shape`]: `Ellipse` and `Circle` block everything outside the
    /// inscribed ellipse, `Rect` blocks nothing.
    ///
    /// ```
    /// use wordcloud::{MaskImage, Options, Shape};
    ///
    /// let mask = MaskImage::from_shape(Shape::Circle, &Options::default());
    /// assert!(format!("{mask:?}").starts_with("MaskImage"));
    /// ```
    #[must_use]
    pub fn from_shape(shape: Shape, options: &Options) -> Self {
        let (width, height) = options.grid_size();
        let (rx, ry) = (width as f32 / 2.0, height as f32 / 2.0);
        let blocked = match shape {
            Shape::Rect => vec![false; width * height],
            Shape::Ellipse | Shape::Circle => (0..width * height)
                .map(|i| {
                    let (x, y) = ((i % width) as f32 + 0.5 - rx, (i / width) as f32 + 0.5 - ry);
                    (x / rx).powi(2) + (y / ry).powi(2) > 1.0
                })
                .collect(),
        };
        Self { blocked }
    }

    /// Build the mask for a shape given by name (`rect`, `ellipse`, `circle`). Same as
    /// [`MaskImage::from_shape`] after parsing the name.
    pub fn shape(name: &str, options: &Options) -> Result<Self> {
        Ok(Self::from_shape(name.parse()?, options))
    }
}

// ---------------------------------------------------------------- rng

/// `SplitMix64`: tiny, seedable, good enough for layout jitter.
#[derive(Debug)]
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

// ---------------------------------------------------------------- shaping + outlines

#[derive(Debug, Clone, Copy)]
enum Segment {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// One shaped word in font units (y up), pen starting at 0.
#[derive(Debug)]
struct Shaped {
    segments: Vec<Segment>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

#[derive(Debug)]
struct Collector {
    segments: Vec<Segment>,
    dx: f32,
    dy: f32,
    bounds: [f32; 4],
}

impl Collector {
    fn point(&mut self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = (x + self.dx, y + self.dy);
        let bounds = &mut self.bounds;
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
        (x, y)
    }
}

impl OutlineBuilder for Collector {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.segments.push(Segment::Move(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.segments.push(Segment::Line(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x, y) = self.point(x, y);
        self.segments.push(Segment::Quad(x1, y1, x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x2, y2) = self.point(x2, y2);
        let (x, y) = self.point(x, y);
        self.segments.push(Segment::Cubic(x1, y1, x2, y2, x, y));
    }

    fn close(&mut self) {
        self.segments.push(Segment::Close);
    }
}

fn shape(face: &rustybuzz::Face<'_>, text: &str) -> Option<Shaped> {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    let glyphs = rustybuzz::shape(face, &[], buffer);
    let mut collector = Collector {
        segments: Vec::new(),
        dx: 0.0,
        dy: 0.0,
        bounds: [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
    };
    let mut pen = 0.0;
    for (info, position) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
        collector.dx = pen + position.x_offset as f32;
        collector.dy = position.y_offset as f32;
        face.outline_glyph(GlyphId(info.glyph_id as u16), &mut collector);
        pen += position.x_advance as f32;
    }
    let [x0, y0, x1, y1] = collector.bounds;
    (x1 > x0 && y1 > y0).then_some(Shaped {
        segments: collector.segments,
        x0,
        y0,
        x1,
        y1,
    })
}

/// Font units -> sprite/canvas pixels, with an optional 90 degree turn (reads bottom to top).
#[derive(Debug, Clone, Copy)]
struct Transformation {
    k: f32,
    x0: f32,
    y1: f32,
    pad: f32,
    wu: f32,
    rotated: bool,
    ox: f32,
    oy: f32,
    div: f32,
}

impl Transformation {
    fn map(&self, fx: f32, fy: f32) -> (f32, f32) {
        let u = (fx - self.x0) * self.k + self.pad;
        let v = (self.y1 - fy) * self.k + self.pad;
        let (x, y) = if self.rotated {
            (v, self.wu - u)
        } else {
            (u, v)
        };
        ((x + self.ox) / self.div, (y + self.oy) / self.div)
    }
}

fn skia_path(segments: &[Segment], transformation: &Transformation) -> Option<tiny_skia::Path> {
    let map = |x, y| transformation.map(x, y);
    let mut builder = PathBuilder::new();
    for segment in segments {
        match *segment {
            Segment::Move(x, y) => {
                let (x, y) = map(x, y);
                builder.move_to(x, y);
            },
            Segment::Line(x, y) => {
                let (x, y) = map(x, y);
                builder.line_to(x, y);
            },
            Segment::Quad(x1, y1, x, y) => {
                let ((x1, y1), (x, y)) = (map(x1, y1), map(x, y));
                builder.quad_to(x1, y1, x, y);
            },
            Segment::Cubic(x1, y1, x2, y2, x, y) => {
                let ((x1, y1), (x2, y2), (x, y)) = (map(x1, y1), map(x2, y2), map(x, y));
                builder.cubic_to(x1, y1, x2, y2, x, y);
            },
            Segment::Close => builder.close(),
        }
    }
    builder.finish()
}

fn svg_path(segments: &[Segment], transformation: &Transformation) -> String {
    let mut data = String::new();
    let mut push = |command: char, points: &[(f32, f32)]| {
        data.push(command);
        for (i, &(x, y)) in points.iter().enumerate() {
            let (x, y) = transformation.map(x, y);
            let _ = write!(data, "{}{:.2} {:.2}", if i > 0 { " " } else { "" }, x, y);
        }
    };
    for segment in segments {
        match *segment {
            Segment::Move(x, y) => push('M', &[(x, y)]),
            Segment::Line(x, y) => push('L', &[(x, y)]),
            Segment::Quad(x1, y1, x, y) => push('Q', &[(x1, y1), (x, y)]),
            Segment::Cubic(x1, y1, x2, y2, x, y) => push('C', &[(x1, y1), (x2, y2), (x, y)]),
            Segment::Close => push('Z', &[]),
        }
    }
    data
}

// ---------------------------------------------------------------- bit sprites + occupancy

/// A word's coverage as rows of 64-px words, LSB = leftmost pixel.
#[derive(Debug)]
struct Sprite {
    width: usize,
    height: usize,
    stride: usize,
    bits: Vec<u64>,
    /// Non-empty (row, word) cells, middle rows first - ink is densest there, so a collision
    /// usually shows up on the first few tests.
    cells: Vec<(usize, usize, u64)>,
    transformation: Transformation,
}

impl Sprite {
    fn new(shaped: &Shaped, size: f32, upem: f32, rotated: bool, gap: usize) -> Option<Self> {
        let k = size / upem;
        let pad = gap as f32 + 1.0;
        let wu = ((shaped.x1 - shaped.x0) * k + 2.0 * pad).ceil();
        let hu = ((shaped.y1 - shaped.y0) * k + 2.0 * pad).ceil();
        let (width, height) = if rotated {
            (hu as usize, wu as usize)
        } else {
            (wu as usize, hu as usize)
        };
        let transformation = Transformation {
            k,
            x0: shaped.x0,
            y1: shaped.y1,
            pad,
            wu,
            rotated,
            ox: 0.0,
            oy: 0.0,
            div: 1.0,
        };
        let mut mask = Mask::new(width as u32, height as u32)?;
        mask.fill_path(
            &skia_path(&shaped.segments, &transformation)?,
            FillRule::Winding,
            true,
            Transform::identity(),
        );
        let mut on: Vec<bool> = mask.data().iter().map(|&a| a > 0).collect();
        dilate(&mut on, width, height, gap);
        let stride = width.div_ceil(64);
        let mut bits = vec![0u64; stride * height];
        for y in 0..height {
            for x in 0..width {
                if on[y * width + x] {
                    bits[y * stride + x / 64] |= 1 << (x % 64);
                }
            }
        }
        Some(Self::with_cells(
            width,
            height,
            stride,
            bits,
            transformation,
        ))
    }

    /// The number of pixels the sprite covers.
    fn ink(&self) -> u32 {
        self.bits.iter().map(|b| b.count_ones()).sum()
    }

    fn with_cells(
        width: usize,
        height: usize,
        stride: usize,
        bits: Vec<u64>,
        transformation: Transformation,
    ) -> Self {
        let mut cells: Vec<(usize, usize, u64)> = (0..height)
            .flat_map(|row| (0..stride).map(move |column| (row, column)))
            .map(|(row, column)| (row, column, bits[row * stride + column]))
            .filter(|cell| cell.2 != 0)
            .collect();
        cells.sort_by_key(|cell| (cell.0 as isize - height as isize / 2).abs());
        Self {
            width,
            height,
            stride,
            bits,
            cells,
            transformation,
        }
    }
}

/// Square dilation by `radius` px (separable max filter).
fn dilate(on: &mut [bool], width: usize, height: usize, radius: usize) {
    if radius == 0 {
        return;
    }
    let source = on.to_vec();
    let mut rows = vec![false; on.len()];
    for y in 0..height {
        for x in 0..width {
            let (a, b) = (x.saturating_sub(radius), (x + radius).min(width - 1));
            rows[y * width + x] = source[y * width + a..=y * width + b].iter().any(|&v| v);
        }
    }
    for y in 0..height {
        let (a, b) = (y.saturating_sub(radius), (y + radius).min(height - 1));
        for x in 0..width {
            on[y * width + x] = (a..=b).any(|row| rows[row * width + x]);
        }
    }
}

#[derive(Debug, Clone)]
struct Grid {
    width: usize,
    height: usize,
    stride: usize,
    bits: Vec<u64>,
}

impl Grid {
    fn new(width: usize, height: usize) -> Self {
        let stride = width.div_ceil(64) + 1;
        Self {
            width,
            height,
            stride,
            bits: vec![0; stride * height],
        }
    }

    fn set(&mut self, x: usize, y: usize) {
        self.bits[y * self.stride + x / 64] |= 1 << (x % 64);
    }

    /// Does `sprite` at (x, y) touch anything? Caller guarantees it lies inside the grid.
    fn collides(&self, sprite: &Sprite, x: usize, y: usize) -> bool {
        let (offset, shift) = (x / 64, x % 64);
        sprite.cells.iter().any(|&(row, column, bits)| {
            let i = (y + row) * self.stride + offset + column;
            self.bits[i] & (bits << shift) != 0
                || (shift > 0 && self.bits[i + 1] & (bits >> (64 - shift)) != 0)
        })
    }

    fn place(&mut self, sprite: &Sprite, x: usize, y: usize) {
        let (offset, shift) = (x / 64, x % 64);
        for row in 0..sprite.height {
            let base = (y + row) * self.stride + offset;
            for column in 0..sprite.stride {
                let bits = sprite.bits[row * sprite.stride + column];
                self.bits[base + column] |= bits << shift;
                if shift > 0 {
                    self.bits[base + column + 1] |= bits >> (64 - shift);
                }
            }
        }
    }

    #[cfg(test)]
    fn count(&self) -> u32 {
        self.bits.iter().map(|b| b.count_ones()).sum()
    }
}

// ---------------------------------------------------------------- layout

/// A word with everything random already drawn, so every bisection pass sees the same word.
#[derive(Debug)]
struct Prepared {
    index: usize,
    shaped: Shaped,
    relative: f32,
    rotated: bool,
    start: (f32, f32),
    theta0: f32,
    direction: f32,
}

/// A word that made it onto the canvas.
#[derive(Debug)]
pub struct Placed {
    /// The word as it was given to [`generate`].
    pub word: Word,
    /// A small extra copy that fills a gap ([`Options::fill`]), not the word's own spot.
    pub copy: bool,
    shape: usize,
    transformation: Transformation,
}

impl Placed {
    /// The fill colour for the light or the dark variant.
    fn fill(&self, dark: bool) -> &str {
        if dark {
            &self.word.color_dark
        } else {
            &self.word.color
        }
    }
}

/// A finished layout. Write it with [`Cloud::svg`] or [`Cloud::png`].
#[derive(Debug)]
pub struct Cloud {
    /// Canvas width in px.
    pub width: u32,
    /// Canvas height in px.
    pub height: u32,
    /// The placed words, largest first, then the fill copies ([`Options::fill`]).
    pub placed: Vec<Placed>,
    /// Words that found no room even at a smaller size.
    pub dropped: Vec<String>,
    /// The global scale found by bisection: font size in px of the heaviest word.
    pub scale: f32,
    shapes: Vec<Shaped>,
    #[cfg(test)]
    grid: Grid,
    #[cfg(test)]
    sprite_bits: u32,
}

/// One pass at a fixed scale.
#[derive(Debug)]
struct Pass {
    placed: Vec<Placed>,
    dropped: Vec<String>,
    #[cfg_attr(not(test), allow(dead_code, reason = "read by the overlap test only"))]
    grid: Grid,
    #[cfg_attr(not(test), allow(dead_code, reason = "read by the overlap test only"))]
    sprite_bits: u32,
}

/// Everything a pass needs, fixed for the whole bisection.
#[derive(Debug)]
struct Packer<'a> {
    words: &'a [Word],
    prepared: Vec<Prepared>,
    base: Grid,
    options: &'a Options,
    upem: f32,
}

/// Lay out `words` with the font in `font_data` (TTF/OTF bytes, e.g. [`crate::DEFAULT_FONT`]).
///
/// # Examples
///
/// ```
/// use wordcloud::{Options, Shape, MaskImage, Word};
///
/// let words: Vec<Word> = [("Sisu", 10.0), ("Löyly", 6.0), ("Kac", 4.0)]
///     .into_iter()
///     .map(|(text, weight)| Word {
///         text: text.into(),
///         weight,
///         color: "#111318".into(),
///         color_dark: "#e6e7ea".into(),
///         related: vec![],
///     })
///     .collect();
/// let options = Options { width: 300, height: 300, ..Options::default() };
/// let mask = MaskImage::from_shape(Shape::Circle, &options);
/// let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &options, Some(&mask))?;
/// assert_eq!(cloud.placed.len(), 3);
/// # anyhow::Ok(())
/// ```
pub fn generate(
    words: &[Word],
    font_data: &[u8],
    options: &Options,
    mask: Option<&MaskImage>,
) -> Result<Cloud> {
    let face = rustybuzz::Face::from_slice(font_data, 0).context("not a usable font")?;
    let packer = Packer {
        words,
        prepared: prepare(words, &face, options)?,
        base: blocked_grid(options, mask),
        options,
        upem: face.units_per_em() as f32,
    };

    // Bisection on the global scale: the largest one where every word fits at nominal size.
    let (mut lo, mut hi) = (options.min_size, options.height as f32);
    for _ in 0..11 {
        let mid = (lo + hi) / 2.0;
        if packer.pass(mid, true).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let pass = packer.pass(lo, false).context("layout failed")?;
    Ok(Cloud {
        width: options.width,
        height: options.height,
        placed: pass.placed,
        dropped: pass.dropped,
        scale: lo,
        shapes: packer.prepared.into_iter().map(|p| p.shaped).collect(),
        #[cfg(test)]
        grid: pass.grid,
        #[cfg(test)]
        sprite_bits: pass.sprite_bits,
    })
}

/// How often [`Options::fill`] copies one word at most: enough to trace a shape, few enough
/// that no short word takes over.
const MAX_COPIES: usize = 4;

/// The largest a copy gets, as a share of its word's size: it reads as an echo, not a twin.
const COPY_SIZE: f32 = 0.7;

/// The empty grid with the mask's blocked cells set.
fn blocked_grid(options: &Options, mask: Option<&MaskImage>) -> Grid {
    let (width, height) = options.grid_size();
    let mut grid = Grid::new(width, height);
    if let Some(mask) = mask {
        for (i, &blocked) in mask.blocked.iter().enumerate() {
            if blocked {
                grid.set(i % width, i / width);
            }
        }
    }
    grid
}

/// Shape every word and draw its randomness once, heaviest first.
fn prepare(words: &[Word], face: &rustybuzz::Face<'_>, options: &Options) -> Result<Vec<Prepared>> {
    let mut rng = Rng(options.seed);
    let max_weight = words.iter().map(|w| w.weight).fold(f32::MIN, f32::max);
    let mut order: Vec<(usize, u64)> = (0..words.len()).map(|i| (i, rng.next())).collect();
    order.sort_by(|a, b| {
        words[b.0]
            .weight
            .total_cmp(&words[a.0].weight)
            .then(a.1.cmp(&b.1))
    });
    let mut prepared = Vec::new();
    for (index, _) in order {
        let word = &words[index];
        let Some(shaped) = shape(face, &word.text) else {
            bail!("font has no outlines for `{}`", word.text)
        };
        let rotated = word.weight < max_weight && rng.f32() < options.vertical;
        let start = ((rng.f32() - 0.5) * 0.4, (rng.f32() - 0.5) * 0.4);
        let theta0 = rng.f32() * std::f32::consts::TAU;
        let direction = if rng.f32() < 0.5 { 1.0 } else { -1.0 };
        prepared.push(Prepared {
            index,
            shaped,
            relative: (word.weight / max_weight).powf(options.contrast),
            rotated,
            start,
            theta0,
            direction,
        });
    }
    Ok(prepared)
}

impl Packer<'_> {
    /// The sprite of `prepared` at `size` canvas px. Each sprite carries half the gap, so two
    /// neighbours sit `gap` apart. Big words get more air: plus 8% of their size.
    fn sprite(&self, prepared: &Prepared, size: f32, rotated: bool) -> Option<Sprite> {
        let resolution = self.options.res as f32;
        let gap = ((self.options.gap + 0.08 * size) * resolution / 2.0).round() as usize;
        Sprite::new(&prepared.shaped, size * resolution, self.upem, rotated, gap)
    }

    /// Mark `sprite` on the grid at (x, y) and return the word placed there.
    fn settle(
        &self,
        grid: &mut Grid,
        sprite: &Sprite,
        (x, y): (usize, usize),
        shape: usize,
        copy: bool,
    ) -> Placed {
        grid.place(sprite, x, y);
        Placed {
            word: self.words[self.prepared[shape].index].clone(),
            copy,
            shape,
            transformation: Transformation {
                ox: x as f32,
                oy: y as f32,
                div: self.options.res as f32,
                ..sprite.transformation
            },
        }
    }

    /// [`Options::fill`]: drop copies of the words into the gaps left after a pass. Only words
    /// up to the median size are copied, [`MAX_COPIES`] times at most, each at [`COPY_SIZE`] of
    /// the word or smaller and a quarter of the canvas away from the word and its other copies.
    /// Sizes run from the median down to `min_size`, lightest words first at each size; spots
    /// are tried from the centre outwards. Returns the bits the copies took.
    fn fill_gaps(&self, grid: &mut Grid, placed: &mut Vec<Placed>, scale: f32) -> Option<u32> {
        let mut relative: Vec<f32> = self.prepared.iter().map(|p| p.relative).collect();
        relative.sort_by(f32::total_cmp);
        let floor = self.options.min_size;
        let median = *relative.get(relative.len() / 2)?;
        let mut size = (COPY_SIZE * scale * median).max(floor);
        let spots = Spots::new(grid);
        // Where each word and its copies are (top left, grid px): the next copy keeps away from
        // all of them.
        let mut set = vec![Vec::new(); self.prepared.len()];
        for word in placed.iter() {
            set[word.shape].push((word.transformation.ox, word.transformation.oy));
        }
        let mut bits = 0;
        loop {
            if size < floor {
                break;
            }
            // Sweep the words until a whole sweep sets nothing: one copy per word and sweep, so
            // the copies of a word spread out.
            let mut placing = true;
            while placing {
                placing = false;
                for (shape_index, prepared) in self.prepared.iter().enumerate().rev() {
                    // A copy stays clearly smaller than its word (down to the smallest size), and
                    // the heavy words are not copied at all.
                    if set[shape_index].len() > MAX_COPIES
                        || prepared.relative > median
                        || size > (COPY_SIZE * scale * prepared.relative).max(floor)
                    {
                        continue;
                    }
                    // Alternate the turn by word, so the copies fill tall gaps as well as wide
                    // ones.
                    let first = shape_index % 3 == 1;
                    for rotated in [first, !first] {
                        let sprite = self.sprite(prepared, size, rotated)?;
                        if let Some(at) = spots.find(grid, &sprite, &set[shape_index]) {
                            set[shape_index].push((at.0 as f32, at.1 as f32));
                            placed.push(self.settle(grid, &sprite, at, shape_index, true));
                            bits += sprite.ink();
                            placing = true;
                            break;
                        }
                    }
                }
            }
            size *= 0.85;
        }
        Some(bits)
    }

    /// Place every word at `scale`. `strict`: fail as soon as one word does not fit at its
    /// nominal size; otherwise shrink it up to 8 times, then drop it.
    fn pass(&self, scale: f32, strict: bool) -> Option<Pass> {
        let resolution = self.options.res as f32;
        let mut grid = self.base.clone();
        let (mut placed, mut dropped, mut sprite_bits) = (Vec::new(), Vec::new(), 0);
        for (shape_index, prepared) in self.prepared.iter().enumerate() {
            let mut size = (scale * prepared.relative).max(self.options.min_size);
            let mut fits = false;
            for _ in 0..if strict { 1 } else { 8 } {
                // With `fill`, a vertical word that finds no spot is tried horizontally too.
                let turns: &[bool] = if self.options.fill && prepared.rotated {
                    &[true, false]
                } else {
                    &[prepared.rotated]
                };
                let mut spot = None;
                for &rotated in turns {
                    let sprite = self.sprite(prepared, size, rotated)?;
                    if let Some(at) = find(&grid, &sprite, prepared, resolution) {
                        spot = Some((sprite, at));
                        break;
                    }
                }
                if let Some((sprite, at)) = spot {
                    placed.push(self.settle(&mut grid, &sprite, at, shape_index, false));
                    sprite_bits += sprite.ink();
                    fits = true;
                    break;
                }
                size *= 0.9;
                if size < self.options.min_size {
                    break;
                }
            }
            if !fits {
                if strict {
                    return None;
                }
                dropped.push(self.words[prepared.index].text.clone());
            }
        }
        if self.options.fill && !strict {
            sprite_bits += self.fill_gaps(&mut grid, &mut placed, scale)?;
        }
        Some(Pass {
            placed,
            dropped,
            grid,
            sprite_bits,
        })
    }
}

/// Walk the spiral from the word's seeded start; the first free spot wins.
fn find(
    grid: &Grid,
    sprite: &Sprite,
    prepared: &Prepared,
    resolution: f32,
) -> Option<(usize, usize)> {
    if sprite.width > grid.width || sprite.height > grid.height {
        return None;
    }
    let (grid_width, grid_height) = (grid.width as f32, grid.height as f32);
    let aspect = grid_width / grid_height; // stretch the spiral to the canvas aspect
    let (cx, cy) = (
        grid_width * (0.5 + prepared.start.0),
        grid_height * (0.5 + prepared.start.1),
    );
    // Far enough that the ellipse covers every corner.
    let radius_max = [
        (0.0, 0.0),
        (grid_width, 0.0),
        (0.0, grid_height),
        (grid_width, grid_height),
    ]
    .iter()
    .map(|&(x, y): &(f32, f32)| (((x - cx) / aspect).powi(2) + (y - cy).powi(2)).sqrt())
    .fold(0.0, f32::max);
    // One canvas px between rings and between steps (grid px = canvas px * resolution).
    let (spacing, step) = (resolution, resolution);
    let (x_max, y_max) = (
        (grid.width - sprite.width) as f32,
        (grid.height - sprite.height) as f32,
    );
    let mut t = 0.0f32;
    loop {
        let radius = spacing * t / std::f32::consts::TAU;
        if radius > radius_max {
            return None;
        }
        let theta = prepared.theta0 + prepared.direction * t;
        let x = (cx + aspect * radius * theta.cos() - sprite.width as f32 / 2.0).round();
        let y = (cy + radius * theta.sin() - sprite.height as f32 / 2.0).round();
        if (0.0..=x_max).contains(&x)
            && (0.0..=y_max).contains(&y)
            && !grid.collides(sprite, x as usize, y as usize)
        {
            return Some((x as usize, y as usize));
        }
        t += step / (radius * aspect.max(1.0)).max(step);
    }
}

/// Candidate centres for [`Options::fill`]: a lattice of 2 grid px, nearest to the canvas
/// centre first (distance relative to the canvas size, so a wide canvas fills evenly).
#[derive(Debug)]
struct Spots(Vec<(usize, usize)>);

impl Spots {
    fn new(grid: &Grid) -> Self {
        let (width, height) = (grid.width as f32, grid.height as f32);
        let mut lattice: Vec<(f32, (usize, usize))> = (0..grid.height)
            .step_by(2)
            .flat_map(|y| (0..grid.width).step_by(2).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (dx, dy) = (x as f32 / width - 0.5, y as f32 / height - 0.5);
                (dx * dx + dy * dy, (x, y))
            })
            .collect();
        lattice.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self(lattice.into_iter().map(|(_, spot)| spot).collect())
    }

    /// The first spot where `sprite`, centred on it, lies inside the grid, touches nothing and
    /// is at least a quarter of the canvas away from every point in `away` (top left, grid px).
    fn find(&self, grid: &Grid, sprite: &Sprite, away: &[(f32, f32)]) -> Option<(usize, usize)> {
        let (width, height) = (grid.width as f32, grid.height as f32);
        let (half_width, half_height) = (sprite.width / 2, sprite.height / 2);
        self.0
            .iter()
            .filter_map(|&(x, y)| Some((x.checked_sub(half_width)?, y.checked_sub(half_height)?)))
            .filter(|&(x, y)| x + sprite.width <= grid.width && y + sprite.height <= grid.height)
            .filter(|&(x, y)| {
                away.iter().all(|&(ax, ay)| {
                    let (dx, dy) = ((x as f32 - ax) / width, (y as f32 - ay) / height);
                    dx * dx + dy * dy >= 0.25 * 0.25
                })
            })
            .find(|&(x, y)| !grid.collides(sprite, x, y))
    }
}

// ---------------------------------------------------------------- output

impl Cloud {
    /// Write the glyph outlines as SVG paths, one per word, with the word as `<title>`.
    ///
    /// No font is needed to display it. Transparent unless `background` is given; `dark`
    /// picks the `[colors_dark]` palette.
    #[must_use]
    pub fn svg(&self, background: Option<&str>, dark: bool) -> String {
        let (w, h) = (self.width, self.height);
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n"
        );
        if let Some(background) = background {
            let background = attr_escape(background);
            let _ = writeln!(
                svg,
                "<rect width=\"{w}\" height=\"{h}\" fill=\"{background}\"/>"
            );
        }
        for placed in &self.placed {
            let data = svg_path(&self.shapes[placed.shape].segments, &placed.transformation);
            let fill = attr_escape(placed.fill(dark));
            let _ = writeln!(
                svg,
                "<path fill=\"{fill}\" d=\"{data}\"><title>{}</title></path>",
                xml_escape(&placed.word.text)
            );
        }
        svg.push_str("</svg>\n");
        svg
    }

    /// Render a PNG with alpha at `scale` device px per canvas px.
    ///
    /// Transparent unless `background` is given. Every colour must be `#rrggbb`.
    pub fn png(&self, scale: f32, background: Option<&str>, dark: bool) -> Result<Vec<u8>> {
        let (width, height) = (
            (self.width as f32 * scale).round() as u32,
            (self.height as f32 * scale).round() as u32,
        );
        let mut pixmap = Pixmap::new(width, height).context("allocate the png canvas")?;
        if let Some(background) = background {
            let rgb: Rgb = background.parse()?;
            pixmap.fill(tiny_skia::Color::from_rgba8(
                rgb.red, rgb.green, rgb.blue, 255,
            ));
        }
        for placed in &self.placed {
            let rgb: Rgb = placed.fill(dark).parse()?;
            let mut paint = Paint::default();
            paint.set_color_rgba8(rgb.red, rgb.green, rgb.blue, 255);
            paint.anti_alias = true;
            if let Some(path) =
                skia_path(&self.shapes[placed.shape].segments, &placed.transformation)
            {
                pixmap.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::from_scale(scale, scale),
                    None,
                );
            }
        }
        pixmap.encode_png().context("encode png")
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape `text` for a double-quoted attribute: a colour is written as given, so a quote in
/// it must not end the attribute.
fn attr_escape(text: &str) -> String {
    xml_escape(text).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_escape_keeps_a_colour_and_closes_no_attribute() {
        assert_eq!(attr_escape("#1a2b3c"), "#1a2b3c");
        assert_eq!(attr_escape("red\" onload=\"x"), "red&quot; onload=&quot;x");
        assert_eq!(attr_escape("a<b&c"), "a&lt;b&amp;c");
    }

    fn options(seed: u64) -> Options {
        Options {
            width: 240,
            height: 140,
            seed,
            vertical: 0.3,
            gap: 2.0,
            contrast: 1.5,
            res: 2,
            min_size: 6.0,
            fill: false,
        }
    }

    fn words() -> Vec<Word> {
        [
            ("Kalsarikännit", 10.0),
            ("Sisu", 9.0),
            ("Kopfkino", 9.0),
            ("Fajrant", 7.0),
            ("Gapa", 6.0),
            ("Kac", 5.0),
            ("C#", 4.0),
            ("Ohrwurm", 3.0),
        ]
        .iter()
        .map(|(text, weight)| Word {
            text: (*text).into(),
            weight: *weight,
            color: "#111318".into(),
            color_dark: "#e6e7ea".into(),
            related: vec![],
        })
        .collect()
    }

    #[test]
    fn grid_collides_respects_word_boundary_shift() {
        let mut grid = Grid::new(200, 4);
        grid.set(130, 1);
        let transformation = Transformation {
            k: 1.0,
            x0: 0.0,
            y1: 0.0,
            pad: 0.0,
            wu: 0.0,
            rotated: false,
            ox: 0.0,
            oy: 0.0,
            div: 1.0,
        };
        let sprite = Sprite::with_cells(3, 1, 1, vec![0b111], transformation);
        assert!(grid.collides(&sprite, 128, 1)); // spans 128..=130
        assert!(!grid.collides(&sprite, 131, 1));
        assert!(!grid.collides(&sprite, 128, 0));
        assert!(!grid.collides(&sprite, 126, 1) && !grid.collides(&sprite, 127, 1));
        grid.place(&sprite, 62, 2); // straddles a word boundary
        assert!(
            grid.collides(&sprite, 63, 2)
                && grid.collides(&sprite, 61, 2)
                && !grid.collides(&sprite, 65, 2)
        );
    }

    #[test]
    fn generate_same_seed_is_deterministic_and_never_overlaps() {
        let font = crate::DEFAULT_FONT;
        let a = generate(&words(), font, &options(7), None).expect("layout");
        let b = generate(&words(), font, &options(7), None).expect("layout");
        assert_eq!(a.svg(None, false), b.svg(None, false));
        assert!(a.dropped.is_empty(), "dropped: {:?}", a.dropped);
        // OR of all sprites == sum of their bits <=> no two sprites share a pixel.
        assert_eq!(a.grid.count(), a.sprite_bits);
        let c = generate(&words(), font, &options(8), None).expect("layout");
        assert_ne!(a.svg(None, false), c.svg(None, false));
    }

    #[test]
    fn generate_fill_adds_light_copies_without_overlap() {
        let options = Options {
            width: 480,
            height: 160,
            fill: true,
            ..options(7)
        };
        let a = generate(&words(), crate::DEFAULT_FONT, &options, None).expect("layout");
        let b = generate(&words(), crate::DEFAULT_FONT, &options, None).expect("layout");
        assert_eq!(a.svg(None, false), b.svg(None, false));
        assert!(a.dropped.is_empty(), "dropped: {:?}", a.dropped);
        assert_eq!(a.grid.count(), a.sprite_bits);
        let copies: Vec<_> = a.placed.iter().filter(|p| p.copy).collect();
        assert!(!copies.is_empty());
        // A light word is copied up to `MAX_COPIES` times, the heaviest word never.
        let most = (0..a.placed.len())
            .map(|shape| copies.iter().filter(|p| p.shape == shape).count())
            .max();
        assert_eq!(most, Some(MAX_COPIES));
        assert!(copies.iter().all(|p| p.word.text != "Kalsarikännit"));
        // Originals come first, then the copies.
        assert!(
            a.placed[..a.placed.len() - copies.len()]
                .iter()
                .all(|p| !p.copy)
        );
    }

    #[test]
    fn generate_fill_off_adds_no_copies() {
        let cloud = generate(&words(), crate::DEFAULT_FONT, &options(7), None).expect("layout");
        assert!(cloud.placed.iter().all(|p| !p.copy));
    }

    #[test]
    fn spots_first_is_centre_last_is_corner() {
        let spots = Spots::new(&Grid::new(40, 20));
        assert_eq!(spots.0.first(), Some(&(20, 10)));
        assert_eq!(spots.0.last(), Some(&(0, 0)));
    }

    /// A PNG of the grid's size whose left half is black.
    fn left_half(options: &Options) -> Vec<u8> {
        let (width, height) = options.grid_size();
        let image = image::GrayImage::from_fn(width as u32, height as u32, |x, _| {
            image::Luma([if (x as usize) < width / 2 { 0 } else { 255 }])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encode");
        png.into_inner()
    }

    #[test]
    fn from_bytes_frees_the_dark_pixels_and_invert_the_light_ones() {
        let options = options(7);
        let (width, _) = options.grid_size();
        let png = left_half(&options);
        for invert in [false, true] {
            let mask = MaskImage::from_bytes(&png, &options, invert).expect("mask");
            assert_eq!(mask.blocked[0], invert);
            assert_eq!(mask.blocked[width - 1], !invert);
        }
    }

    #[test]
    fn generate_with_fill_in_a_shape_image_sets_no_word_outside_it() {
        let options = Options {
            fill: true,
            ..options(7)
        };
        let mask = MaskImage::from_bytes(&left_half(&options), &options, false).expect("mask");
        let cloud = generate(&words(), crate::DEFAULT_FONT, &options, Some(&mask)).expect("cloud");
        assert_eq!(
            cloud.grid.count(),
            cloud.sprite_bits + mask.blocked.len() as u32 / 2
        );
        let half = (options.width * options.res / 2) as f32;
        assert!(cloud.placed.iter().all(|p| p.transformation.ox < half));
    }

    #[test]
    fn open_takes_a_built_in_name_or_an_image_file_and_names_the_shape_otherwise() {
        let options = options(7);
        let dir = std::env::temp_dir();
        assert!(MaskImage::open("circle", &dir, &options, false).is_ok());
        let error = MaskImage::open("heart", &dir, &options, false).expect_err("no such shape");
        assert!(
            format!("{error:#}")
                .starts_with("unknown shape `heart` (rect | ellipse | circle | image file): read "),
            "{error:#}"
        );
    }

    #[test]
    fn options_from_style_fill_defaults_off() {
        assert!(!Options::default().fill);
        let style = Style {
            fill: Some(true),
            ..Style::default()
        };
        assert!(Options::from_style(&style).fill);
    }

    #[test]
    fn shape_parse_known_names_roundtrip() {
        for shape in [Shape::Rect, Shape::Ellipse, Shape::Circle] {
            assert_eq!(shape.name().parse::<Shape>().expect("known"), shape);
        }
        assert!("heart".parse::<Shape>().is_err());
    }

    #[test]
    fn png_named_colour_errors() {
        let mut words = words();
        words[0].color = "ink".into();
        let cloud = generate(&words, crate::DEFAULT_FONT, &options(7), None).expect("layout");
        let error = cloud.png(1.0, None, false).expect_err("not #rrggbb");
        assert_eq!(error.to_string(), "colour `ink` is not #rrggbb");
    }
}
