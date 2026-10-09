//! Micropolis's 16x16 tile set, and what the map's tile values draw as.
//!
//! The set is `micropolis/tiles.xpm` from the GPL release, parsed once. It
//! holds 960 tiles; the engine's map values reach 1018 only through the seven
//! extra churches, which no released tile set draws, so they show the first.

use std::sync::OnceLock;

pub const TILE: usize = 16;
pub const COUNT: usize = 960;

pub const LOMASK: u16 = 0x03ff;
pub const PWRBIT: u16 = 0x8000;
pub const CONDBIT: u16 = 0x4000;
pub const ZONEBIT: u16 = 0x0400;

pub const DIRT: u16 = 0;
pub const RIVER: u16 = 2;
pub const LASTRIVEDGE: u16 = 20;
pub const TREEBASE: u16 = 21;
pub const WOODS5: u16 = 43;
pub const RUBBLE: u16 = 44;
pub const LASTRUBBLE: u16 = 47;
pub const FLOOD: u16 = 48;
pub const LASTFLOOD: u16 = 51;
pub const RADTILE: u16 = 52;
pub const FIRE: u16 = 56;
pub const LASTFIRE: u16 = 63;
pub const HBRIDGE: u16 = 64;
pub const LASTROAD: u16 = 206;
pub const HPOWER: u16 = 208;
pub const LASTPOWER: u16 = 222;
pub const FREEZ: u16 = 244;
pub const COMCLR: u16 = 427;
pub const INDCLR: u16 = 616;
const CHURCH0BASE: u16 = 414;
const CHURCH1BASE: u16 = 956;
const CHURCH7LAST: u16 = 1018;
const LIGHTNINGBOLT: u16 = 827;

pub struct Tiles {
    /// Palette index per pixel, tile after tile, rows of [`TILE`].
    pub pix: Vec<u8>,
    pub pal: Vec<u32>,
}

pub fn tiles() -> &'static Tiles {
    static T: OnceLock<Tiles> = OnceLock::new();
    T.get_or_init(|| parse(include_str!("../../micropolis/tiles.xpm")))
}

/// An XPM's quoted strings are its header, its colours, then its rows.
fn parse(xpm: &str) -> Tiles {
    let mut strs = xpm.lines().filter_map(|l| {
        let l = l.trim();
        let a = l.find('"')?;
        let b = l.rfind('"')?;
        (b > a).then(|| &l[a + 1..b])
    });
    let head: Vec<usize> = strs
        .next()
        .expect("xpm header")
        .split_whitespace()
        .map(|n| n.parse().expect("xpm header number"))
        .collect();
    let (w, h, ncol) = (head[0], head[1], head[2]);
    assert_eq!((w, h), (TILE, TILE * COUNT), "unexpected tile sheet size");
    let mut keys = [0u8; 256];
    let mut pal = Vec::with_capacity(ncol);
    for (i, s) in strs.by_ref().take(ncol).enumerate() {
        let hex = s.rsplit('#').next().expect("xpm colour");
        // #RRRRGGGGBBBB: the high byte of each 16-bit channel.
        let ch = |n: usize| u32::from_str_radix(&hex[n * 4..n * 4 + 2], 16).unwrap_or(0);
        pal.push(ch(0) << 16 | ch(1) << 8 | ch(2));
        keys[s.as_bytes()[0] as usize] = i as u8;
    }
    let mut pix = Vec::with_capacity(w * h);
    for row in strs.take(h) {
        pix.extend(row.bytes().take(w).map(|b| keys[b as usize]));
    }
    assert_eq!(pix.len(), w * h, "short tile sheet");
    Tiles { pix, pal }
}

/// The tile a map value draws as. `blink` is the half second an unpowered
/// zone shows the lightning bolt, as the original front end does.
#[inline]
pub fn shown(v: u16, blink: bool) -> u16 {
    if blink && v & ZONEBIT != 0 && v & PWRBIT == 0 {
        return LIGHTNINGBOLT;
    }
    let t = v & LOMASK;
    match t {
        CHURCH1BASE..=CHURCH7LAST => CHURCH0BASE + (t - CHURCH1BASE) % 9,
        t if t as usize >= COUNT => DIRT,
        t => t,
    }
}

pub fn is_water(t: u16) -> bool {
    (RIVER..=LASTRIVEDGE).contains(&t)
}

/// Land the bulldozer clears for free on the way to building: dirt, trees,
/// rubble.
pub fn is_clear(t: u16) -> bool {
    t == DIRT || (TREEBASE..=WOODS5).contains(&t) || (RUBBLE..=LASTRUBBLE).contains(&t)
}

pub fn is_road(t: u16) -> bool {
    (HBRIDGE..=LASTROAD).contains(&t)
}

pub fn is_hazard(t: u16) -> bool {
    (FLOOD..=LASTFLOOD).contains(&t) || (FIRE..=LASTFIRE).contains(&t) || t == RADTILE
}

/// Every tile scaled to `tw` x `th` panel pixels, nearest neighbour, as
/// colours: one tile row is then one slice copy.
pub fn atlas(tw: usize, th: usize) -> Vec<u32> {
    let t = tiles();
    let xs: Vec<usize> = (0..tw).map(|x| x * TILE / tw).collect();
    let mut out = Vec::with_capacity(COUNT * tw * th);
    for tile in 0..COUNT {
        let src = &t.pix[tile * TILE * TILE..][..TILE * TILE];
        for y in 0..th {
            let row = &src[y * TILE / th * TILE..][..TILE];
            out.extend(xs.iter().map(|&x| t.pal[row[x] as usize]));
        }
    }
    out
}

/// Each tile's mean colour, for the mirror's one cell per tile.
pub fn means() -> Vec<u32> {
    let t = tiles();
    t.pix
        .as_chunks::<{ TILE * TILE }>()
        .0
        .iter()
        .map(|px| {
            let sum = px.iter().fold([0u32; 3], |mut s, &i| {
                let c = t.pal[i as usize];
                s[0] += c >> 16;
                s[1] += c >> 8 & 0xff;
                s[2] += c & 0xff;
                s
            });
            let n = (TILE * TILE) as u32;
            (sum[0] / n) << 16 | (sum[1] / n) << 8 | (sum[2] / n)
        })
        .collect()
}
