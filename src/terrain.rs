//! Terrain color resolution, biome tinting, Cave Mode slicing, and NW slope shading.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TintMode {
    None = 0,
    Grass = 1,
    Foliage = 2,
    DryFoliage = 3,
    Water = 4,
    FixedSpruce = 5,
    FixedBirch = 6,
    FixedLilyPad = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CaveMode {
    Auto = 0,
    Off = 1,
    Manual = 2,
}

impl CaveMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Off,
            2 => Self::Manual,
            _ => Self::Auto,
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Manual,
            Self::Manual => Self::Off,
            Self::Off => Self::Auto,
        }
    }
}

/// Compact per-block-state visual summary built once per resource reload.
#[derive(Clone, Copy, Debug)]
pub struct BlockLook {
    pub rgb: [u8; 3],
    pub tint: TintMode,
    pub is_air: bool,
    pub is_fluid: bool,
    pub is_water: bool,
    pub is_foliage: bool,
    pub transparent: bool,
}

impl Default for BlockLook {
    fn default() -> Self {
        Self {
            rgb: [0, 0, 0],
            tint: TintMode::None,
            is_air: true,
            is_fluid: false,
            is_water: false,
            is_foliage: false,
            transparent: true,
        }
    }
}

/// Biome tint palette entry (0x00RRGGBB colors).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BiomeColors {
    pub grass: u32,
    pub foliage: u32,
    pub dry_foliage: u32,
    pub water: u32,
    /// 0 = normal, 1 = swamp grass modifier, 2 = dark forest grass modifier.
    pub grass_mod: u8,
}

impl Default for BiomeColors {
    fn default() -> Self {
        Self {
            grass: 0x0079_C05A,
            foliage: 0x0059_AE30,
            dry_foliage: 0x00A8_8D53,
            water: 0x003F_76E4,
            grass_mod: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColumnCell {
    pub rgb: [u8; 3],
    pub height: i16,
    pub biome: u16,
    pub water_depth: u8,
    pub flags: u8, // bit 0: explored, bit 1: water_surface, bit 2: cave_sample
}

impl ColumnCell {
    pub const FLAG_EXPLORED: u8 = 1 << 0;
    pub const FLAG_WATER: u8 = 1 << 1;
    pub const FLAG_CAVE: u8 = 1 << 2;

    #[inline]
    pub fn is_explored(self) -> bool {
        (self.flags & Self::FLAG_EXPLORED) != 0
    }

    #[inline]
    pub fn is_water(self) -> bool {
        (self.flags & Self::FLAG_WATER) != 0
    }
}

/// Multiply-tint a base texture color with biome grass/foliage/water tint.
#[inline]
pub fn apply_biome_tint(
    rgb: [u8; 3],
    tint: TintMode,
    biome: BiomeColors,
    world_x: i32,
    world_z: i32,
) -> [u8; 3] {
    let tint_rgb = match tint {
        TintMode::None => return rgb,
        TintMode::Grass => {
            let base = biome.grass;
            match biome.grass_mod {
                1 => {
                    // Vanilla swamp grass modifier alternates two mossy tones cleanly by coordinate hash.
                    let wave = ((world_x.wrapping_mul(734_287) ^ world_z.wrapping_mul(912_271)) & 3) == 0;
                    if wave { 0x006A_7039 } else { 0x004C_763C }
                }
                2 => ((base & 0x00FE_FEFE) + 0x0028_340A) >> 1,
                _ => base,
            }
        }
        TintMode::Foliage => biome.foliage,
        TintMode::DryFoliage => biome.dry_foliage,
        TintMode::Water => biome.water,
        TintMode::FixedSpruce => 0x0061_9961,
        TintMode::FixedBirch => 0x0080_A755,
        TintMode::FixedLilyPad => 0x0020_8030,
    };
    let [_, tr, tg, tb] = tint_rgb.to_be_bytes();
    let mut r = ((u16::from(rgb[0]) * u16::from(tr) + 127) / 255) as u16;
    let mut g = ((u16::from(rgb[1]) * u16::from(tg) + 127) / 255) as u16;
    let mut b = ((u16::from(rgb[2]) * u16::from(tb) + 127) / 255) as u16;
    if matches!(
        tint,
        TintMode::Foliage | TintMode::FixedSpruce | TintMode::FixedBirch
    ) {
        // Gentle canopy depth so tree crowns stand out cleanly over grass without looking muddy.
        r = (r * 88) / 100;
        g = (g * 92) / 100;
        b = (b * 86) / 100;
    }
    [r.min(255) as u8, g.min(255) as u8, b.min(255) as u8]
}

/// Shade a surface/cave pixel using Xaero-style NW elevation slope and water depth absorption.
#[inline]
pub fn shade_column_pixel(
    cell: ColumnCell,
    north: Option<ColumnCell>,
    west: Option<ColumnCell>,
    nw_or_se_step: i16,
    in_cave: bool,
) -> [u8; 4] {
    if !cell.is_explored() {
        return [0, 0, 0, 255];
    }

    let mut r = f32::from(cell.rgb[0]);
    let mut g = f32::from(cell.rgb[1]);
    let mut b = f32::from(cell.rgb[2]);

    // Northwest slope highlight / southeast drop shadow matching Screenshot 1/2/3 relief.
    let mut slope: i32 = 0;
    if let Some(n) = north.filter(|n| n.is_explored()) {
        slope += i32::from(cell.height - n.height).clamp(-4, 4);
    }
    if let Some(w) = west.filter(|w| w.is_explored()) {
        slope += i32::from(cell.height - w.height).clamp(-4, 4);
    }
    if nw_or_se_step != 0 {
        slope += i32::from(nw_or_se_step).clamp(-2, 2);
    }

    let factor = if cell.is_water() {
        // Deep navy absorption + bright shallow shoreline ripple.
        let depth = f32::from(cell.water_depth.min(24));
        let depth_shade = (0.98 - depth * 0.024).clamp(0.56, 0.98);
        let ripple = 1.0 + (slope as f32) * 0.025;
        (depth_shade * ripple).clamp(0.52, 1.06)
    } else if in_cave {
        // Emphasize vertical tunnel edges and walls in cave slicing.
        (0.92 + (slope as f32) * 0.075).clamp(0.58, 1.24)
    } else {
        match slope {
            s if s >= 4 => 1.22,
            3 => 1.18,
            2 => 1.13,
            1 => 1.07,
            0 => 1.0,
            -1 => 0.85,
            -2 => 0.75,
            -3 => 0.67,
            _ => 0.60,
        }
    };

    r = (r * factor).clamp(0.0, 255.0);
    g = (g * factor).clamp(0.0, 255.0);
    b = (b * factor).clamp(0.0, 255.0);

    // Subtle atmospheric height tint on high terrain peaks vs deep lowlands while preserving true block RGB.
    if !cell.is_water() && !in_cave {
        let height_bias = ((f32::from(cell.height) - 64.0) * 0.0007).clamp(-0.05, 0.06);
        let mul = 1.0 + height_bias;
        let avg = (r + g + b) * (1.0 / 3.0);
        let boost = 1.04;
        r = ((avg + (r - avg) * boost) * mul).clamp(0.0, 255.0);
        g = ((avg + (g - avg) * boost) * mul).clamp(0.0, 255.0);
        b = ((avg + (b - avg) * boost) * mul).clamp(0.0, 255.0);
    }

    [r.round() as u8, g.round() as u8, b.round() as u8, 255]
}

/// Find the visible block Y for a column in Cave Mode starting downward from `ceiling_y`.
/// First skips any initial ceiling rock until hitting an air pocket, then stops on the
/// first solid/fluid floor below that pocket. If the column is solid all the way down,
/// falls back cleanly to a dark bedrock/wall shade so cave corridors stand out sharply.
pub fn scan_cave_column<F>(
    min_y: i32,
    ceiling_y: i32,
    max_y: i32,
    mut read_block: F,
    looks: &[BlockLook],
) -> Option<(i32, BlockLook, u8)>
where
    F: FnMut(i32) -> u32,
{
    let start_y = ceiling_y.min(max_y - 1);
    if start_y < min_y {
        return None;
    }

    // Check if the starting block itself is solid roof (must carve through to open cavity) or open air.
    let top_id = read_block(start_y) as usize;
    let top_look = looks.get(top_id).copied().unwrap_or_default();
    let mut in_open_cavity = top_look.is_air || top_look.transparent;
    let mut y = start_y;

    while y >= min_y {
        let state_id = read_block(y) as usize;
        let look = looks.get(state_id).copied().unwrap_or_default();
        if !in_open_cavity {
            if look.is_air || look.transparent {
                in_open_cavity = true;
            }
        } else if !look.is_air {
            if look.is_water {
                let mut depth = 1u8;
                let mut wy = y - 1;
                while wy >= min_y && depth < 32 {
                    let below = looks.get(read_block(wy) as usize).copied().unwrap_or_default();
                    if !below.is_water {
                        break;
                    }
                    depth += 1;
                    wy -= 1;
                }
                return Some((y, look, depth));
            }
            if !look.transparent || look.is_foliage {
                return Some((y, look, 0));
            }
        }
        y -= 1;
    }

    // Column stayed solid roof down to min_y: return flagged solid-pillar look (`transparent=true, !is_foliage`).
    if !top_look.is_air {
        let mut wall = top_look;
        wall.rgb = [wall.rgb[0] / 3, wall.rgb[1] / 3, wall.rgb[2] / 3];
        wall.tint = TintMode::None;
        wall.is_foliage = false;
        wall.transparent = true;
        return Some((start_y, wall, 0));
    }
    None
}
