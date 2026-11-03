//! The packer: shaping, coverage sprites, spiral placement, SVG/PNG output.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder};
use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, Transform};

#[cfg(test)]
use crate::source::Source;
use crate::source::Word;

// ---------------------------------------------------------------- options

pub struct Options {
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    /// Probability that a word (outside the top tier) is set vertically.
    pub vertical: f32,
    /// Minimum distance between two words, canvas px (plus 8% of the font size).
    pub gap: f32,
    /// size = scale * (weight / max) ^ contrast
    pub contrast: f32,
    /// Grid pixels per canvas px. 2 is plenty; higher is slower and tighter.
    pub res: u32,
    /// Smallest font size in canvas px.
    pub min_size: f32,
}

impl Options {
    /// Defaults, overridden by the word list's `[style]`.
    pub fn from_style(st: &crate::source::Style) -> Self {
        Self {
            width: st.width.unwrap_or(800),
            height: st.height.unwrap_or(418),
            seed: st.seed.unwrap_or(90),
            vertical: st.vertical.unwrap_or(0.18).clamp(0.0, 1.0),
            gap: st.gap.unwrap_or(2.0),
            contrast: st.contrast.unwrap_or(1.5),
            res: 2,
            min_size: 8.0,
        }
    }
}

/// Blocked cells from a mask image, sized to the grid (`res` included).
pub struct MaskImage {
    blocked: Vec<bool>,
}

impl MaskImage {
    /// Dark, opaque pixels are where words may go; `invert` flips that.
    pub fn load(path: &Path, opt: &Options, invert: bool) -> Result<Self> {
        let (w, h) = (opt.width * opt.res, opt.height * opt.res);
        let img = image::open(path).with_context(|| format!("read mask {}", path.display()))?.to_luma_alpha8();
        let img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
        let blocked = img.pixels().map(|p| (p.0[0] < 128 && p.0[1] >= 128) == invert).collect();
        Ok(Self { blocked })
    }

    /// Built-in shape, no file: `ellipse` fills the ellipse inscribed in the canvas (a circle
    /// on a square canvas), `rect` is the plain canvas.
    pub fn shape(name: &str, opt: &Options) -> Result<Self> {
        let (w, h) = ((opt.width * opt.res) as usize, (opt.height * opt.res) as usize);
        let (rx, ry) = (w as f32 / 2.0, h as f32 / 2.0);
        let blocked = match name {
            "rect" => vec![false; w * h],
            "ellipse" | "circle" => (0..w * h)
                .map(|i| {
                    let (x, y) = ((i % w) as f32 + 0.5 - rx, (i / w) as f32 + 0.5 - ry);
                    (x / rx).powi(2) + (y / ry).powi(2) > 1.0
                })
                .collect(),
            other => bail!("unknown shape `{other}` (rect | ellipse | circle)"),
        };
        Ok(Self { blocked })
    }
}

// ---------------------------------------------------------------- rng

/// SplitMix64 - tiny, seedable, good enough for layout jitter.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
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

#[derive(Clone, Copy)]
enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// One shaped word in font units (y up), pen starting at 0.
struct Shaped {
    segs: Vec<Seg>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

struct Collector {
    segs: Vec<Seg>,
    dx: f32,
    dy: f32,
    bounds: [f32; 4],
}

impl Collector {
    fn pt(&mut self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = (x + self.dx, y + self.dy);
        let b = &mut self.bounds;
        b[0] = b[0].min(x);
        b[1] = b[1].min(y);
        b[2] = b[2].max(x);
        b[3] = b[3].max(y);
        (x, y)
    }
}

impl OutlineBuilder for Collector {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.pt(x, y);
        self.segs.push(Seg::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.pt(x, y);
        self.segs.push(Seg::Line(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.pt(x1, y1);
        let (x, y) = self.pt(x, y);
        self.segs.push(Seg::Quad(x1, y1, x, y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.pt(x1, y1);
        let (x2, y2) = self.pt(x2, y2);
        let (x, y) = self.pt(x, y);
        self.segs.push(Seg::Cubic(x1, y1, x2, y2, x, y));
    }
    fn close(&mut self) {
        self.segs.push(Seg::Close);
    }
}

fn shape(face: &rustybuzz::Face, text: &str) -> Option<Shaped> {
    let mut buf = rustybuzz::UnicodeBuffer::new();
    buf.push_str(text);
    let glyphs = rustybuzz::shape(face, &[], buf);
    let mut c = Collector { segs: Vec::new(), dx: 0.0, dy: 0.0, bounds: [f32::MAX, f32::MAX, f32::MIN, f32::MIN] };
    let mut pen = 0.0;
    for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
        c.dx = pen + pos.x_offset as f32;
        c.dy = pos.y_offset as f32;
        face.outline_glyph(GlyphId(info.glyph_id as u16), &mut c);
        pen += pos.x_advance as f32;
    }
    let [x0, y0, x1, y1] = c.bounds;
    (x1 > x0 && y1 > y0).then_some(Shaped { segs: c.segs, x0, y0, x1, y1 })
}

/// Font units -> sprite/canvas pixels, with an optional 90 degree turn (reads bottom to top).
#[derive(Clone, Copy)]
struct Xf {
    k: f32,
    x0: f32,
    y1: f32,
    pad: f32,
    wu: f32,
    rot: bool,
    ox: f32,
    oy: f32,
    div: f32,
}

impl Xf {
    fn map(&self, fx: f32, fy: f32) -> (f32, f32) {
        let u = (fx - self.x0) * self.k + self.pad;
        let v = (self.y1 - fy) * self.k + self.pad;
        let (x, y) = if self.rot { (v, self.wu - u) } else { (u, v) };
        ((x + self.ox) / self.div, (y + self.oy) / self.div)
    }
}

fn skia_path(segs: &[Seg], xf: &Xf) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for s in segs {
        match *s {
            Seg::Move(x, y) => {
                let (x, y) = xf.map(x, y);
                pb.move_to(x, y);
            }
            Seg::Line(x, y) => {
                let (x, y) = xf.map(x, y);
                pb.line_to(x, y);
            }
            Seg::Quad(x1, y1, x, y) => {
                let ((x1, y1), (x, y)) = (xf.map(x1, y1), xf.map(x, y));
                pb.quad_to(x1, y1, x, y);
            }
            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                let ((x1, y1), (x2, y2), (x, y)) = (xf.map(x1, y1), xf.map(x2, y2), xf.map(x, y));
                pb.cubic_to(x1, y1, x2, y2, x, y);
            }
            Seg::Close => pb.close(),
        }
    }
    pb.finish()
}

fn svg_path(segs: &[Seg], xf: &Xf) -> String {
    let mut d = String::new();
    let mut p = |cmd: char, pts: &[(f32, f32)]| {
        d.push(cmd);
        for (i, &(x, y)) in pts.iter().enumerate() {
            let (x, y) = xf.map(x, y);
            let _ = write!(d, "{}{:.2} {:.2}", if i > 0 { " " } else { "" }, x, y);
        }
    };
    for s in segs {
        match *s {
            Seg::Move(x, y) => p('M', &[(x, y)]),
            Seg::Line(x, y) => p('L', &[(x, y)]),
            Seg::Quad(x1, y1, x, y) => p('Q', &[(x1, y1), (x, y)]),
            Seg::Cubic(x1, y1, x2, y2, x, y) => p('C', &[(x1, y1), (x2, y2), (x, y)]),
            Seg::Close => p('Z', &[]),
        }
    }
    d
}

// ---------------------------------------------------------------- bit sprites + occupancy

/// A word's coverage as rows of 64-px words, LSB = leftmost pixel.
struct Sprite {
    w: usize,
    h: usize,
    stride: usize,
    bits: Vec<u64>,
    /// Non-empty (row, word) cells, middle rows first - ink is densest there, so a collision
    /// usually shows up on the first few tests.
    cells: Vec<(usize, usize, u64)>,
    xf: Xf,
}

impl Sprite {
    fn new(shaped: &Shaped, size: f32, upem: f32, rot: bool, gap: usize) -> Option<Self> {
        let k = size / upem;
        let pad = gap as f32 + 1.0;
        let wu = ((shaped.x1 - shaped.x0) * k + 2.0 * pad).ceil();
        let hu = ((shaped.y1 - shaped.y0) * k + 2.0 * pad).ceil();
        let (w, h) = if rot { (hu as usize, wu as usize) } else { (wu as usize, hu as usize) };
        let xf = Xf { k, x0: shaped.x0, y1: shaped.y1, pad, wu, rot, ox: 0.0, oy: 0.0, div: 1.0 };
        let mut mask = Mask::new(w as u32, h as u32)?;
        mask.fill_path(&skia_path(&shaped.segs, &xf)?, FillRule::Winding, true, Transform::identity());
        let mut on: Vec<bool> = mask.data().iter().map(|&a| a > 0).collect();
        dilate(&mut on, w, h, gap);
        let stride = w.div_ceil(64);
        let mut bits = vec![0u64; stride * h];
        for y in 0..h {
            for x in 0..w {
                if on[y * w + x] {
                    bits[y * stride + x / 64] |= 1 << (x % 64);
                }
            }
        }
        Some(Self::with_cells(w, h, stride, bits, xf))
    }

    fn with_cells(w: usize, h: usize, stride: usize, bits: Vec<u64>, xf: Xf) -> Self {
        let mut cells: Vec<(usize, usize, u64)> =
            (0..h).flat_map(|r| (0..stride).map(move |j| (r, j))).map(|(r, j)| (r, j, bits[r * stride + j])).filter(|c| c.2 != 0).collect();
        cells.sort_by_key(|c| (c.0 as isize - h as isize / 2).abs());
        Self { w, h, stride, bits, cells, xf }
    }
}

/// Square dilation by `r` px (separable max filter).
fn dilate(on: &mut [bool], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let src = on.to_vec();
    let mut tmp = vec![false; on.len()];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = src[y * w + a..=y * w + b].iter().any(|&v| v);
        }
    }
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            on[y * w + x] = (a..=b).any(|yy| tmp[yy * w + x]);
        }
    }
}

#[derive(Clone)]
struct Grid {
    w: usize,
    h: usize,
    stride: usize,
    bits: Vec<u64>,
}

impl Grid {
    fn new(w: usize, h: usize) -> Self {
        let stride = w.div_ceil(64) + 1;
        Self { w, h, stride, bits: vec![0; stride * h] }
    }

    fn set(&mut self, x: usize, y: usize) {
        self.bits[y * self.stride + x / 64] |= 1 << (x % 64);
    }

    /// Does `s` at (x, y) touch anything? Caller guarantees it lies inside the grid.
    fn collides(&self, s: &Sprite, x: usize, y: usize) -> bool {
        let (off, sh) = (x / 64, x % 64);
        s.cells.iter().any(|&(r, j, b)| {
            let i = (y + r) * self.stride + off + j;
            self.bits[i] & (b << sh) != 0 || (sh > 0 && self.bits[i + 1] & (b >> (64 - sh)) != 0)
        })
    }

    fn place(&mut self, s: &Sprite, x: usize, y: usize) {
        let (off, sh) = (x / 64, x % 64);
        for r in 0..s.h {
            let base = (y + r) * self.stride + off;
            for j in 0..s.stride {
                let b = s.bits[r * s.stride + j];
                self.bits[base + j] |= b << sh;
                if sh > 0 {
                    self.bits[base + j + 1] |= b >> (64 - sh);
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

struct Prepared {
    idx: usize,
    shaped: Shaped,
    rel: f32,
    rot: bool,
    start: (f32, f32),
    theta0: f32,
    dir: f32,
}

pub struct Placed {
    pub word: Word,
    shape: usize,
    xf: Xf,
}

pub struct Cloud {
    pub width: u32,
    pub height: u32,
    pub placed: Vec<Placed>,
    pub dropped: Vec<String>,
    pub scale: f32,
    shapes: Vec<Shaped>,
    #[cfg(test)]
    grid: Grid,
    #[cfg(test)]
    sprite_bits: u32,
}

/// Lay out `words` with the font in `font_data`.
pub fn generate(words: &[Word], font_data: &[u8], opt: &Options, mask: Option<&MaskImage>) -> Result<Cloud> {
    let face = rustybuzz::Face::from_slice(font_data, 0).context("not a usable font")?;
    let upem = face.units_per_em() as f32;
    let res = opt.res as f32;
    let (gw, gh) = ((opt.width * opt.res) as usize, (opt.height * opt.res) as usize);
    let mut base = Grid::new(gw, gh);
    if let Some(m) = mask {
        for (i, &b) in m.blocked.iter().enumerate() {
            if b {
                base.set(i % gw, i / gw);
            }
        }
    }

    // Per-word randomness drawn once, so every bisection run sees the same words.
    let mut rng = Rng(opt.seed);
    let wmax = words.iter().map(|w| w.weight).fold(f32::MIN, f32::max);
    let mut order: Vec<(usize, u64)> = words.iter().enumerate().map(|(i, _)| (i, rng.next())).collect();
    order.sort_by(|a, b| words[b.0].weight.total_cmp(&words[a.0].weight).then(a.1.cmp(&b.1)));
    let mut prep = Vec::new();
    for (idx, _) in order {
        let w = &words[idx];
        let Some(shaped) = shape(&face, &w.text) else { bail!("font has no outlines for `{}`", w.text) };
        let rot = w.weight < wmax && rng.f32() < opt.vertical;
        let start = ((rng.f32() - 0.5) * 0.4, (rng.f32() - 0.5) * 0.4);
        let theta0 = rng.f32() * std::f32::consts::TAU;
        let dir = if rng.f32() < 0.5 { 1.0 } else { -1.0 };
        prep.push(Prepared { idx, shaped, rel: (w.weight / wmax).powf(opt.contrast), rot, start, theta0, dir });
    }

    let run = |scale: f32, strict: bool| -> Option<(Vec<Placed>, Vec<String>, Grid, u32)> {
        let mut grid = base.clone();
        let (mut placed, mut dropped, mut bits) = (Vec::new(), Vec::new(), 0);
        for (pi, p) in prep.iter().enumerate() {
            let mut size = (scale * p.rel).max(opt.min_size);
            let mut ok = false;
            for _ in 0..if strict { 1 } else { 8 } {
                // Each sprite carries half the gap, so two neighbours sit `gap` apart. Big words
                // get more air: plus 8% of their size.
                let gap = ((opt.gap + 0.08 * size) * res / 2.0).round() as usize;
                let sprite = Sprite::new(&p.shaped, size * res, upem, p.rot, gap)?;
                if let Some((x, y)) = find(&grid, &sprite, p, res) {
                    grid.place(&sprite, x, y);
                    bits += sprite.bits.iter().map(|b| b.count_ones()).sum::<u32>();
                    let xf = Xf { ox: x as f32, oy: y as f32, div: res, ..sprite.xf };
                    placed.push(Placed { word: words[p.idx].clone(), shape: pi, xf });
                    ok = true;
                    break;
                }
                size *= 0.9;
                if size < opt.min_size {
                    break;
                }
            }
            if !ok {
                if strict {
                    return None;
                }
                dropped.push(words[p.idx].text.clone());
            }
        }
        Some((placed, dropped, grid, bits))
    };

    // Bisection on the global scale: the largest one where every word fits at nominal size.
    let (mut lo, mut hi) = (opt.min_size, opt.height as f32);
    for _ in 0..11 {
        let mid = (lo + hi) / 2.0;
        if run(mid, true).is_some() { lo = mid } else { hi = mid }
    }
    let (placed, dropped, _grid, _bits) = run(lo, false).context("layout failed")?;
    Ok(Cloud {
        width: opt.width,
        height: opt.height,
        placed,
        dropped,
        scale: lo,
        shapes: prep.into_iter().map(|p| p.shaped).collect(),
        #[cfg(test)]
        grid: _grid,
        #[cfg(test)]
        sprite_bits: _bits,
    })
}

/// Walk the spiral from the word's seeded start; first free spot wins.
fn find(grid: &Grid, s: &Sprite, p: &Prepared, res: f32) -> Option<(usize, usize)> {
    if s.w > grid.w || s.h > grid.h {
        return None;
    }
    let (gw, gh) = (grid.w as f32, grid.h as f32);
    let e = gw / gh; // stretch the spiral to the canvas aspect
    let (cx, cy) = (gw * (0.5 + p.start.0), gh * (0.5 + p.start.1));
    // Far enough that the ellipse covers every corner.
    let rmax = [(0.0, 0.0), (gw, 0.0), (0.0, gh), (gw, gh)]
        .iter()
        .map(|&(x, y): &(f32, f32)| (((x - cx) / e).powi(2) + (y - cy).powi(2)).sqrt())
        .fold(0.0, f32::max);
    // One canvas px between rings and between steps (grid px = canvas px * res).
    let (spacing, step) = (res, res);
    let (xmax, ymax) = ((grid.w - s.w) as f32, (grid.h - s.h) as f32);
    let mut t = 0.0f32;
    loop {
        let r = spacing * t / std::f32::consts::TAU;
        if r > rmax {
            return None;
        }
        let th = p.theta0 + p.dir * t;
        let x = (cx + e * r * th.cos() - s.w as f32 / 2.0).round();
        let y = (cy + r * th.sin() - s.h as f32 / 2.0).round();
        if (0.0..=xmax).contains(&x) && (0.0..=ymax).contains(&y) && !grid.collides(s, x as usize, y as usize) {
            return Some((x as usize, y as usize));
        }
        t += step / (r * e.max(1.0)).max(step);
    }
}

// ---------------------------------------------------------------- output

impl Cloud {
    /// Glyph outlines as SVG paths, one per word. No font needed to display it. Transparent
    /// unless `background` is given; `dark` picks the `[colors_dark]` palette.
    pub fn svg(&self, background: Option<&str>, dark: bool) -> String {
        let (w, h) = (self.width, self.height);
        let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n");
        if let Some(bg) = background {
            let _ = writeln!(s, "<rect width=\"{w}\" height=\"{h}\" fill=\"{bg}\"/>");
        }
        for p in &self.placed {
            let d = svg_path(&self.shapes[p.shape].segs, &p.xf);
            let fill = if dark { &p.word.color_dark } else { &p.word.color };
            let _ = writeln!(s, "<path fill=\"{fill}\" d=\"{d}\"><title>{}</title></path>", xml_escape(&p.word.text));
        }
        s.push_str("</svg>\n");
        s
    }

    /// Raster version with alpha, `scale` device px per canvas px. Transparent unless `background`.
    pub fn png(&self, scale: f32, background: Option<&str>, dark: bool) -> Result<Vec<u8>> {
        let (w, h) = ((self.width as f32 * scale).round() as u32, (self.height as f32 * scale).round() as u32);
        let mut pm = Pixmap::new(w, h).context("png size")?;
        if let Some(bg) = background {
            let (r, g, b) = hex(bg)?;
            pm.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));
        }
        for p in &self.placed {
            let (r, g, b) = hex(if dark { &p.word.color_dark } else { &p.word.color })?;
            let mut paint = Paint::default();
            paint.set_color_rgba8(r, g, b, 255);
            paint.anti_alias = true;
            if let Some(path) = skia_path(&self.shapes[p.shape].segs, &p.xf) {
                pm.fill_path(&path, &paint, FillRule::Winding, Transform::from_scale(scale, scale), None);
            }
        }
        pm.encode_png().context("encode png")
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn hex(c: &str) -> Result<(u8, u8, u8)> {
    let h = c.trim_start_matches('#');
    if h.len() != 6 {
        bail!("colour `{c}` is not #rrggbb");
    }
    let v = u32::from_str_radix(h, 16).with_context(|| format!("colour `{c}`"))?;
    Ok(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(seed: u64) -> Options {
        Options { width: 240, height: 140, seed, vertical: 0.3, gap: 2.0, contrast: 1.5, res: 2, min_size: 6.0 }
    }

    fn words() -> Vec<Word> {
        [("TypeScript", 10.0), ("Rust", 9.0), ("Kubernetes", 9.0), ("GitOps", 7.0), ("PHP", 6.0), ("GxP", 5.0), ("C#", 4.0), ("Flash", 3.0)]
            .iter()
            .map(|(t, w)| Word { text: (*t).into(), weight: *w, color: "#111318".into(), color_dark: "#e6e7ea".into(), related: vec![] })
            .collect()
    }

    #[test]
    fn bitset_collision_respects_shift() {
        let mut g = Grid::new(200, 4);
        g.set(130, 1);
        let xf = Xf { k: 1.0, x0: 0.0, y1: 0.0, pad: 0.0, wu: 0.0, rot: false, ox: 0.0, oy: 0.0, div: 1.0 };
        let s = Sprite::with_cells(3, 1, 1, vec![0b111], xf);
        assert!(g.collides(&s, 128, 1)); // spans 128..=130
        assert!(!g.collides(&s, 131, 1));
        assert!(!g.collides(&s, 128, 0));
        assert!(!g.collides(&s, 126, 1) && !g.collides(&s, 127, 1));
        g.place(&s, 62, 2); // straddles a word boundary
        assert!(g.collides(&s, 63, 2) && g.collides(&s, 61, 2) && !g.collides(&s, 65, 2));
    }

    #[test]
    fn layout_is_deterministic_and_never_overlaps() {
        let font = crate::DEFAULT_FONT;
        let a = generate(&words(), font, &opt(7), None).expect("layout");
        let b = generate(&words(), font, &opt(7), None).expect("layout");
        assert_eq!(a.svg(None, false), b.svg(None, false));
        assert!(a.dropped.is_empty(), "dropped: {:?}", a.dropped);
        // OR of all sprites == sum of their bits <=> no two sprites share a pixel.
        assert_eq!(a.grid.count(), a.sprite_bits);
        let c = generate(&words(), font, &opt(8), None).expect("layout");
        assert_ne!(a.svg(None, false), c.svg(None, false));
    }

    #[test]
    fn source_parses_tiers_entries_and_lints_duplicates() {
        let src: Source = toml::from_str(
            r##"
            terms = ["tsc"]
            [colors]
            accent = "#ff4f00"
            [[tier]]
            weight = 10
            color = "accent"
            words = [{ text = "TypeScript", related = ["TS"] }, "Rust"]
            [[word]]
            text = "rust"
            weight = 3
            "##,
        )
        .expect("parse");
        let w = src.words();
        assert_eq!(w[0].color, "#ff4f00");
        assert_eq!(w[0].related, vec!["TS"]);
        assert_eq!(w[2].color, "ink");
        assert_eq!(src.terms, vec!["tsc"]);
        assert_eq!(src.lint().len(), 1);
        assert_eq!(w[0].color_dark, "#ff4f00"); // no [colors_dark] -> falls back to the light colour
    }
}
