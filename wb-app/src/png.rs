//! Minimal PNG encoder (8-bit RGB, no compression: zlib "stored" blocks). Deterministic byte-for-byte.

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for n in 0..256u32 {
        let mut c = n;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        t[n as usize] = c;
    }
    t
}

pub fn crc32(parts: &[&[u8]]) -> u32 {
    let t = crc_table();
    let mut c = 0xFFFF_FFFFu32;
    for p in parts {
        for &b in *p {
            c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
    }
    c ^ 0xFFFF_FFFF
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&[kind, data]).to_be_bytes());
}

/// `rgb` is `width * height * 3` bytes, row-major, top row first.
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    assert_eq!(rgb.len(), (width as usize) * (height as usize) * 3, "png: bad buffer size");
    let mut raw = Vec::with_capacity(rgb.len() + height as usize);
    for row in rgb.chunks((width as usize) * 3) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(b) = blocks.next() {
        z.push(if blocks.peek().is_none() { 1 } else { 0 });
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Stable 64-bit FNV-1a hash (used for mesh ids and snapshot hashes).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Decode only what the tests need: dimensions and the raw RGB buffer of a PNG made by `encode_rgb`.
pub fn decode_rgb_stored(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if png.len() < 8 || png[..8] != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }
    let (mut i, mut w, mut h, mut idat) = (8usize, 0u32, 0u32, Vec::new());
    while i + 8 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().ok()?) as usize;
        let kind = &png[i + 4..i + 8];
        let data = png.get(i + 8..i + 8 + len)?;
        let crc = u32::from_be_bytes(png.get(i + 8 + len..i + 12 + len)?.try_into().ok()?);
        if crc != crc32(&[kind, data]) {
            return None;
        }
        match kind {
            b"IHDR" => {
                w = u32::from_be_bytes(data[0..4].try_into().ok()?);
                h = u32::from_be_bytes(data[4..8].try_into().ok()?);
            }
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        i += 12 + len;
    }
    let mut p = 2usize;
    let mut raw = Vec::new();
    loop {
        let fin = *idat.get(p)? & 1;
        let n = u16::from_le_bytes([*idat.get(p + 1)?, *idat.get(p + 2)?]) as usize;
        raw.extend_from_slice(idat.get(p + 5..p + 5 + n)?);
        p += 5 + n;
        if fin == 1 {
            break;
        }
    }
    let row = (w as usize) * 3;
    let mut rgb = Vec::with_capacity(row * h as usize);
    for r in raw.chunks(row + 1) {
        rgb.extend_from_slice(&r[1..]);
    }
    Some((w, h, rgb))
}
