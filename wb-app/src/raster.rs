//! Renderer-free images of a world (Part 9a): colormaps, flat projections (sphere equirectangular, cube unfolded cross, plane direct)
//! and a legend strip. No UI and no GPU: pixels are computed on the CPU and encoded by `png.rs`.
//! Each pixel is mapped to a 3D point, and the owning cell is found by a greedy walk over the mesh adjacency (topology-agnostic).
use crate::api::{ApiError, ApiResult};
use crate::schema::meta_of;
use crate::session::Session;
use crate::world::shape_name;
use enbuilder::Domain;

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32, bg: [u8; 3]) -> Image {
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for _ in 0..width * height {
            rgb.extend_from_slice(&bg);
        }
        Image { width, height, rgb }
    }
    pub fn put(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height {
            let i = ((y as u32 * self.width + x as u32) * 3) as usize;
            self.rgb[i..i + 3].copy_from_slice(&c);
        }
    }
    pub fn get(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
    pub fn to_png(&self) -> Vec<u8> { crate::png::encode_rgb(self.width, self.height, &self.rgb) }
}

// ---------------------------------------------------------------- colormaps

const BG: [u8; 3] = [24, 24, 28];
const LEGEND_BG: [u8; 3] = [16, 16, 20];
const TEXT: [u8; 3] = [230, 230, 230];

fn lerp3(stops: &[(f64, [u8; 3])], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            return [0, 1, 2].map(|k| (c0[k] as f64 + (c1[k] as f64 - c0[k] as f64) * f).round() as u8);
        }
    }
    stops.last().unwrap().1
}

/// Map t in [0,1] through a named colormap (`terrain`, `temperature`, `rain`, `speed`, `viridis`, `mask`). Unknown names use viridis.
pub fn colormap(name: &str, t: f64) -> [u8; 3] {
    match name {
        "terrain" => lerp3(&[(0.0, [8, 24, 90]), (0.3, [30, 90, 170]), (0.499, [120, 190, 230]), (0.5, [70, 140, 70]), (0.62, [150, 180, 80]), (0.75, [150, 110, 70]), (0.9, [120, 100, 95]), (1.0, [250, 250, 250])], t),
        "temperature" => lerp3(&[(0.0, [40, 20, 120]), (0.2, [40, 100, 220]), (0.4, [150, 215, 240]), (0.5, [245, 245, 235]), (0.65, [250, 215, 110]), (0.82, [225, 90, 40]), (1.0, [110, 10, 20])], t),
        "rain" => lerp3(&[(0.0, [245, 240, 225]), (0.2, [200, 225, 190]), (0.45, [90, 175, 190]), (0.7, [30, 90, 175]), (1.0, [15, 25, 100])], t),
        "speed" => lerp3(&[(0.0, [240, 245, 250]), (0.35, [150, 200, 220]), (0.7, [240, 160, 60]), (1.0, [150, 20, 30])], t),
        "mask" => if t >= 0.5 { [235, 235, 235] } else { [30, 30, 36] },
        _ => lerp3(&[(0.0, [68, 1, 84]), (0.25, [59, 82, 139]), (0.5, [33, 145, 140]), (0.75, [94, 201, 98]), (1.0, [253, 231, 37])], t),
    }
}

/// Deterministic distinct colour for category ids that have no table.
pub fn category_color(id: i64) -> [u8; 3] {
    let h = crate::png::fnv1a(&id.to_le_bytes());
    [0, 1, 2].map(|k| 60 + ((h >> (k * 8)) & 0xff) as u8 % 190)
}

// ---------------------------------------------------------------- tiny digit font (3x5)

fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7, 5, 5, 5, 7], '1' => [2, 6, 2, 2, 7], '2' => [7, 1, 7, 4, 7], '3' => [7, 1, 7, 1, 7], '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7], '6' => [7, 4, 7, 5, 7], '7' => [7, 1, 1, 1, 1], '8' => [7, 5, 7, 5, 7], '9' => [7, 5, 7, 1, 7],
        '-' => [0, 0, 7, 0, 0], '.' => [0, 0, 0, 0, 2], 'e' => [0, 7, 7, 4, 7], '+' => [0, 2, 7, 2, 0],
        _ => [0; 5],
    }
}

fn text_width(s: &str, scale: i64) -> i64 { s.chars().count() as i64 * 4 * scale }

fn draw_text(img: &mut Image, x: i64, y: i64, s: &str, scale: i64, color: [u8; 3]) {
    for (i, ch) in s.chars().enumerate() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits >> (2 - col) & 1 == 1 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            img.put(x + (i as i64 * 4 + col) * scale + dx, y + row as i64 * scale + dy, color);
                        }
                    }
                }
            }
        }
    }
}

/// Short number text for legend labels.
pub fn fmt_num(v: f64) -> String {
    let a = v.abs();
    if v == 0.0 { "0".into() }
    else if a >= 1.0e5 || a < 1.0e-2 { format!("{:.1e}", v).replace("e0", "e").replace("e-0", "e-") }
    else if a >= 100.0 { format!("{:.0}", v) }
    else if a >= 10.0 { format!("{:.1}", v) }
    else { format!("{:.2}", v) }
}

// ---------------------------------------------------------------- locating cells

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 { (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2) }

pub fn brute(d: &Domain, p: [f64; 3]) -> usize {
    let c = &d.mesh.centroid;
    (0..d.mesh.n_cells).min_by(|&i, &j| dist2(c[i], p).partial_cmp(&dist2(c[j], p)).unwrap()).unwrap_or(0)
}

/// Nearest cell by greedy descent on the adjacency graph, starting at `start` (usually the previous pixel's cell).
pub fn locate(d: &Domain, p: [f64; 3], start: usize) -> usize {
    let m = &d.mesh;
    let mut cur = start;
    let mut best = dist2(m.centroid[cur], p);
    for _ in 0..m.n_cells {
        let mut moved = false;
        for k in m.adj_offsets[cur] as usize..m.adj_offsets[cur + 1] as usize {
            let nb = m.adj[k] as usize;
            let dd = dist2(m.centroid[nb], p);
            if dd < best {
                best = dd;
                cur = nb;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    cur
}

/// Geometry of one flat projection: image size and a pixel -> 3D point function (None = outside the map).
pub struct Projection {
    pub kind: &'static str,
    pub width: u32,
    pub height: u32,
    point: Box<dyn Fn(u32, u32) -> Option<[f64; 3]> + Send + Sync>,
}

impl Projection {
    pub fn point(&self, x: u32, y: u32) -> Option<[f64; 3]> { (self.point)(x, y) }
}

/// Cube cross layout (faces left to right in the middle row): -x, -y, +x, +y; +z above the -y face, -z below it.
/// Side faces use right = z x outward-normal (as seen from outside, moving right goes counter-clockwise seen from +z, so the strip wraps); the top and bottom faces are laid out so that their edges touch the -y face.
pub const CUBE_CROSS: &str = "row1: -x -y +x +y; row0: +z above -y; row2: -z below -y";

pub fn projection(d: &Domain, kind: &str, width: u32) -> ApiResult<Projection> {
    let c = &d.mesh.centroid;
    match kind {
        "sphere" => {
            let r = c.iter().map(|p| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()).sum::<f64>() / c.len() as f64;
            let (w, h) = (width, (width / 2).max(1));
            Ok(Projection { kind: "equirectangular", width: w, height: h, point: Box::new(move |x, y| {
                let lon = ((x as f64 + 0.5) / w as f64 * 2.0 - 1.0) * std::f64::consts::PI;
                let lat = (0.5 - (y as f64 + 0.5) / h as f64) * std::f64::consts::PI;
                Some([r * lat.cos() * lon.cos(), r * lat.cos() * lon.sin(), r * lat.sin()])
            }) })
        }
        "cube" => {
            let half = c.iter().flat_map(|p| p.iter().map(|v| v.abs())).fold(0.0, f64::max);
            let s = (width / 4).max(2);
            let (w, h) = (s * 4, s * 3);
            Ok(Projection { kind: "cube_cross", width: w, height: h, point: Box::new(move |x, y| {
                let (fc, fr) = (x / s, y / s);
                let a = ((x % s) as f64 + 0.5) / s as f64 * 2.0 - 1.0; // right
                let b = 1.0 - ((y % s) as f64 + 0.5) / s as f64 * 2.0; // up
                let p = match (fr, fc) {
                    (1, 0) => [-1.0, -a, b], // -x
                    (1, 1) => [a, -1.0, b],  // -y
                    (1, 2) => [1.0, a, b],   // +x
                    (1, 3) => [-a, 1.0, b],  // +y
                    (0, 1) => [a, b, 1.0],
                    (2, 1) => [a, -b, -1.0],
                    _ => return None,
                };
                Some([p[0] * half, p[1] * half, p[2] * half])
            }) })
        }
        "plane" => {
            let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for p in c {
                x0 = x0.min(p[0]); x1 = x1.max(p[0]); y0 = y0.min(p[1]); y1 = y1.max(p[1]);
            }
            let (ex, ey) = ((x1 - x0).max(1e-9), (y1 - y0).max(1e-9));
            let z = c[0][2];
            let h = ((width as f64 * ey / ex).round() as u32).max(1);
            let w = width;
            Ok(Projection { kind: "plane", width: w, height: h, point: Box::new(move |px, py| {
                Some([x0 + (px as f64 + 0.5) / w as f64 * ex, y1 - (py as f64 + 0.5) / h as f64 * ey, z])
            }) })
        }
        other => Err(ApiError::new("bad_shape", format!("no projection for '{other}'"))),
    }
}

/// Cell under every pixel (None outside the map). Row-major, `width * height` entries.
pub fn pixel_cells(d: &Domain, proj: &Projection) -> Vec<Option<usize>> {
    let mut out = Vec::with_capacity((proj.width * proj.height) as usize);
    let mut last: Option<usize> = None;
    // Greedy descent is exact when it starts near the answer; after a jump (a new cube face, the first pixel) start from a brute-force search.
    let spacing = (d.mesh.area.iter().sum::<f64>() / d.mesh.n_cells as f64).sqrt();
    for y in 0..proj.height {
        for x in 0..proj.width {
            match proj.point(x, y) {
                None => out.push(None),
                Some(p) => {
                    let start = match last {
                        Some(l) if dist2(d.mesh.centroid[l], p) < (3.0 * spacing).powi(2) => l,
                        _ => brute(d, p),
                    };
                    let c = locate(d, p, start);
                    last = Some(c);
                    out.push(Some(c));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- rendering

#[derive(Debug, Clone)]
pub struct RenderOpts {
    pub width: u32,
    pub layer: usize,
    pub legend: bool,
}

impl Default for RenderOpts {
    fn default() -> Self { RenderOpts { width: 720, layer: 0, legend: true } }
}

/// Value range used for the colour scale: the schema's fixed range, else the min/max of the finite data.
pub fn value_range(name: &str, values: &[f64]) -> (f64, f64) {
    if let Some(Some((lo, hi))) = meta_of(name).map(|m| m.7) {
        return (lo, hi);
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &v in values.iter().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() { (0.0, 1.0) } else if hi <= lo { (lo, lo + 1.0) } else { (lo, hi) }
}

pub fn render_field(s: &Session, name: &str, opts: &RenderOpts) -> ApiResult<Image> {
    let f = s.field(name, opts.layer)?;
    let d = s.domain();
    let proj = projection(d, shape_name(s.params()), opts.width)?;
    let cells = pixel_cells(d, &proj);
    let meta = meta_of(name);
    let kind = meta.map(|m| m.1).unwrap_or("scalar");
    let cmap = meta.map(|m| m.6).unwrap_or("viridis");
    let (lo, hi) = value_range(name, &f.values);
    let table = s.biome_table();
    let n_categories = table.biomes.len() as i64;

    let legend_h = if opts.legend { 44 } else { 0 };
    let mut img = Image::new(proj.width, proj.height + legend_h, BG);
    for y in 0..proj.height {
        for x in 0..proj.width {
            if let Some(c) = cells[(y * proj.width + x) as usize] {
                let v = f.values[c];
                let col = if kind == "category" {
                    let id = v.round() as i64;
                    if name == "biome" && id >= 0 && id < n_categories { table.biomes[id as usize].color } else { category_color(id) }
                } else if !v.is_finite() {
                    BG
                } else {
                    colormap(cmap, (v - lo) / (hi - lo))
                };
                img.put(x as i64, y as i64, col);
            }
        }
    }
    if opts.legend {
        draw_legend(&mut img, proj.height, name, kind, cmap, lo, hi, &f.values, &table);
    }
    Ok(img)
}

#[allow(clippy::too_many_arguments)]
fn draw_legend(img: &mut Image, top: u32, name: &str, kind: &str, cmap: &str, lo: f64, hi: f64, values: &[f64], table: &enbuilder::BiomeTable) {
    let (w, top) = (img.width as i64, top as i64);
    for y in top..top + 44 {
        for x in 0..w {
            img.put(x, y, LEGEND_BG);
        }
    }
    if kind == "category" {
        let mut present: Vec<i64> = values.iter().filter(|v| v.is_finite()).map(|v| v.round() as i64).collect();
        present.sort_unstable();
        present.dedup();
        let sw = ((w - 8) / present.len().max(1) as i64).clamp(8, 40);
        for (i, &id) in present.iter().enumerate() {
            let x0 = 4 + i as i64 * sw;
            let col = if name == "biome" && id >= 0 && (id as usize) < table.biomes.len() { table.biomes[id as usize].color } else { category_color(id) };
            for y in top + 6..top + 26 {
                for x in x0..x0 + sw - 2 {
                    img.put(x, y, col);
                }
            }
            draw_text(img, x0, top + 30, &id.to_string(), 2, TEXT);
        }
        return;
    }
    let (x0, x1) = (8, w - 8);
    for x in x0..x1 {
        let t = (x - x0) as f64 / (x1 - x0).max(1) as f64;
        let c = colormap(cmap, t);
        for y in top + 6..top + 24 {
            img.put(x, y, c);
        }
    }
    // labels are in display units (the schema's scale and offset)
    let (sc, off) = meta_of(name).map(|m| (m.3, m.4)).unwrap_or((1.0, 0.0));
    let (a, b) = (fmt_num(lo * sc + off), fmt_num(hi * sc + off));
    draw_text(img, x0, top + 28, &a, 2, TEXT);
    draw_text(img, x1 - text_width(&b, 2) + 2, top + 28, &b, 2, TEXT);
    let unit = meta_of(name).map(|m| m.5).unwrap_or("");
    let _ = unit; // units are letters; the tiny font has digits only, so the unit is reported on stdout by wb-snap
}
