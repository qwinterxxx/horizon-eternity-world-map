//! Radar entity snapshots and layered vanilla Minecraft mob model specs for crisp icon rendering.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityCategory {
    Mob = 0,
    Player = 1,
    Vehicle = 2,
}

#[derive(Clone, Debug)]
pub struct RadarEntity {
    pub x: f64,
    pub z: f64,
    pub yaw_deg: f32,
    pub category: EntityCategory,
    /// Texture key normalized without leading `textures/` or trailing `.png`, e.g. `"entity/pig/pig_temperate"`.
    pub texture_key: String,
    /// Short species key, e.g. `"pig"`, `"zombie"`, `"boat"`.
    pub species_key: String,
    pub display_name: String,
    pub uuid: u128,
    pub is_local: bool,
}

/// Texture pixel rectangle `(px, py, pw, ph)` inside a `(tex_w, tex_h)` entity texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobFaceRect {
    pub px: u16,
    pub py: u16,
    pub pw: u16,
    pub ph: u16,
    pub tex_w: u16,
    pub tex_h: u16,
    pub hat: Option<[u16; 4]>,
    pub fallback_rgb: u32,
}

impl MobFaceRect {
    const fn std64(fallback_rgb: u32) -> Self {
        Self {
            px: 8,
            py: 8,
            pw: 8,
            ph: 8,
            tex_w: 64,
            tex_h: 64,
            hat: Some([40, 8, 8, 8]),
            fallback_rgb,
        }
    }

    const fn std32(fallback_rgb: u32) -> Self {
        Self {
            px: 8,
            py: 8,
            pw: 8,
            ph: 8,
            tex_w: 64,
            tex_h: 32,
            hat: None,
            fallback_rgb,
        }
    }

    const fn custom(
        px: u16,
        py: u16,
        pw: u16,
        ph: u16,
        tex_w: u16,
        tex_h: u16,
        hat: Option<[u16; 4]>,
        fallback_rgb: u32,
    ) -> Self {
        Self {
            px,
            py,
            pw,
            ph,
            tex_w,
            tex_h,
            hat,
            fallback_rgb,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlaySource {
    Same,
    SheepWool,
    EndermanEyes,
    SpiderEyes,
    DrownedOuter,
    BoggedOverlay,
    StrayOverlay,
    BreezeEyes,
    CreakingEyes,
    CopperGolemEyes,
    PhantomEyes,
}

/// Additional 2.5D model piece (snout, beak, wattle, nose, ears, horns, hat, eyes, wool).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobPart {
    pub px: u16,
    pub py: u16,
    pub pw: u16,
    pub ph: u16,
    /// Placement relative to head box in 1/16ths (`0..=16` spans base box).
    pub dx16: i8,
    pub dy16: i8,
    pub dw16: u8,
    pub dh16: u8,
    pub source: OverlaySource,
    pub behind_base: bool,
    pub shadow_3d: bool,
}

impl MobPart {
    const fn same(
        px: u16,
        py: u16,
        pw: u16,
        ph: u16,
        dx16: i8,
        dy16: i8,
        dw16: u8,
        dh16: u8,
        behind_base: bool,
        shadow_3d: bool,
    ) -> Self {
        Self {
            px,
            py,
            pw,
            ph,
            dx16,
            dy16,
            dw16,
            dh16,
            source: OverlaySource::Same,
            behind_base,
            shadow_3d,
        }
    }

    const fn companion(
        px: u16,
        py: u16,
        pw: u16,
        ph: u16,
        dx16: i8,
        dy16: i8,
        dw16: u8,
        dh16: u8,
        source: OverlaySource,
        behind_base: bool,
    ) -> Self {
        Self {
            px,
            py,
            pw,
            ph,
            dx16,
            dy16,
            dw16,
            dh16,
            source,
            behind_base,
            shadow_3d: false,
        }
    }
}

pub const MAX_MOB_PARTS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobModelSpec {
    pub base: MobFaceRect,
    /// Visual head aspect ratio in 1/16ths (16x16 = square head; 13x16 = villager tall head).
    pub width16: u8,
    pub height16: u8,
    pub part_count: u8,
    pub parts: [MobPart; MAX_MOB_PARTS],
}

const EMPTY_PART: MobPart = MobPart::same(0, 0, 1, 1, 0, 0, 16, 16, false, false);

impl MobModelSpec {
    const fn simple(base: MobFaceRect) -> Self {
        Self {
            base,
            width16: 16,
            height16: 16,
            part_count: 0,
            parts: [EMPTY_PART; MAX_MOB_PARTS],
        }
    }

    const fn shaped(base: MobFaceRect, width16: u8, height16: u8, parts: &[MobPart]) -> Self {
        let mut arr = [EMPTY_PART; MAX_MOB_PARTS];
        let mut i = 0;
        while i < parts.len() && i < MAX_MOB_PARTS {
            arr[i] = parts[i];
            i += 1;
        }
        Self {
            base,
            width16,
            height16,
            part_count: i as u8,
            parts: arr,
        }
    }

    pub fn active_parts(&self) -> &[MobPart] {
        &self.parts[..usize::from(self.part_count)]
    }
}

/// Normalize `"minecraft:zombie"` / `"entity/zombie/zombie.png"` to clean short species id `"zombie"`.
pub fn normalize_species(raw: &str) -> &str {
    let s = raw.rsplit(':').next().unwrap_or(raw);
    let s = s.rsplit('/').next().unwrap_or(s);
    s.strip_suffix(".png").unwrap_or(s)
}

/// Multi-part 2.5D model specification matched to `26.3` vanilla textures (including `_baby` sizes).
pub fn mob_model_spec(species_id: &str, actual_w: u16, actual_h: u16) -> MobModelSpec {
    let sp = normalize_species(species_id);

    // Handle scaled-down `_baby` variants whose atlas texture size differs cleanly from adults.
    match (sp, actual_w, actual_h) {
        ("chicken", 16, 16) => {
            return MobModelSpec::simple(MobFaceRect::custom(2, 2, 6, 5, 16, 16, None, 0x00F5_D84E));
        }
        ("pig", 32, 32) => {
            return MobModelSpec::simple(MobFaceRect::custom(4, 19, 8, 7, 32, 32, None, 0x00F1_9FA8));
        }
        ("squid" | "glow_squid", 32, 32) => {
            let col = if sp == "glow_squid" { 0x0032_A8AE } else { 0x0036_5977 };
            return MobModelSpec::simple(MobFaceRect::custom(8, 8, 8, 8, 32, 32, None, col));
        }
        ("wolf", 32, 32) => {
            return MobModelSpec::simple(MobFaceRect::custom(2, 2, 3, 3, 32, 32, None, 0x00CC_C8C0));
        }
        ("cat" | "ocelot", 32, 32) => {
            return MobModelSpec::simple(MobFaceRect::custom(2, 2, 3, 2, 32, 32, None, 0x00D8_9E55));
        }
        ("fox", 32, 32) => {
            return MobModelSpec::simple(MobFaceRect::custom(4, 7, 5, 4, 32, 32, None, 0x00E0_6F27));
        }
        ("bee", 32, 32) => {
            return MobModelSpec::simple(MobFaceRect::custom(3, 3, 4, 4, 32, 32, None, 0x00EB_BE34));
        }
        ("axolotl", 32, 32) => {
            return MobModelSpec::simple(MobFaceRect::custom(2, 2, 4, 3, 32, 32, None, 0x00F2_99CE));
        }
        ("nautilus" | "zombie_nautilus", 64, 64) => {
            return MobModelSpec::shaped(
                MobFaceRect::custom(9, 6, 9, 9, 64, 64, None, 0x00DF_8C60),
                16,
                15,
                &[MobPart::same(8, 28, 6, 5, 3, 9, 10, 7, false, true)],
            );
        }
        ("happy_ghast", 64, 64) => {
            return MobModelSpec::simple(MobFaceRect::custom(8, 8, 8, 8, 64, 64, None, 0x00F0_F0F0));
        }
        ("polar_bear", 64, 64) => {
            return MobModelSpec::simple(MobFaceRect::custom(4, 4, 4, 4, 64, 64, None, 0x00F2_F5F6));
        }
        ("turtle", 16, 16) => {
            return MobModelSpec::simple(MobFaceRect::custom(2, 2, 3, 3, 16, 16, None, 0x0049_9B42));
        }
        _ => {}
    }

    match sp {
        "pig" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 8, 64, 64, None, 0x00F1_9FA8),
            16,
            16,
            &[MobPart::same(17, 17, 4, 3, 4, 8, 8, 6, false, true)],
        ),
        "cow" | "mooshroom" => {
            let col = if sp == "mooshroom" { 0x009B_221C } else { 0x006B_4A33 };
            MobModelSpec::shaped(
                MobFaceRect::custom(6, 6, 8, 8, 64, 64, None, col),
                16,
                16,
                &[
                    MobPart::same(23, 0, 1, 3, -1, -2, 2, 4, false, false),
                    MobPart::same(23, 0, 1, 3, 15, -2, 2, 4, false, false),
                    MobPart::same(53, 1, 6, 3, 3, 10, 10, 5, false, true),
                ],
            )
        }
        "sheep" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 6, 64, 32, None, 0x00E8_D4BA),
            16,
            12,
            &[
                MobPart::companion(8, 8, 8, 8, -2, -3, 20, 17, OverlaySource::SheepWool, true),
            ],
        ),
        "chicken" => MobModelSpec::shaped(
            MobFaceRect::custom(3, 3, 4, 6, 64, 32, None, 0x00F5_F5F5),
            12,
            16,
            &[
                MobPart::same(16, 2, 4, 2, 1, 6, 14, 5, false, true),
                MobPart::same(16, 6, 2, 2, 5, 11, 6, 4, false, true),
            ],
        ),
        "creeper" => MobModelSpec::simple(MobFaceRect::std32(0x0041_B837)),
        "skeleton" | "wither_skeleton" => {
            let col = if sp == "wither_skeleton" { 0x0034_3436 } else { 0x00D8_D8D8 };
            MobModelSpec::simple(MobFaceRect::std32(col))
        }
        "stray" => MobModelSpec::shaped(
            MobFaceRect::std32(0x0078_8E93),
            16,
            16,
            &[
                MobPart::companion(8, 8, 8, 8, 0, 0, 16, 16, OverlaySource::StrayOverlay, false),
                MobPart::companion(40, 8, 8, 8, 0, 0, 16, 16, OverlaySource::StrayOverlay, false),
            ],
        ),
        "bogged" => MobModelSpec::shaped(
            MobFaceRect::std32(0x005B_6F4E),
            16,
            16,
            &[
                MobPart::companion(8, 8, 8, 8, 0, 0, 16, 16, OverlaySource::BoggedOverlay, false),
                MobPart::companion(40, 8, 8, 8, 0, 0, 16, 16, OverlaySource::BoggedOverlay, false),
            ],
        ),
        "parched" => MobModelSpec::simple(MobFaceRect::std64(0x00C6_B695)),
        "spider" | "cave_spider" => {
            let col = if sp == "cave_spider" { 0x001D_353E } else { 0x003B_3230 };
            MobModelSpec::shaped(
                MobFaceRect::custom(40, 12, 8, 8, 64, 32, None, col),
                16,
                16,
                &[MobPart::companion(40, 12, 8, 8, 0, 0, 16, 16, OverlaySource::SpiderEyes, false)],
            )
        }
        "slime" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 8, 64, 32, None, 0x006B_C35B),
            16,
            16,
            &[
                MobPart::same(6, 22, 6, 6, 2, 2, 12, 12, false, false),
                MobPart::same(34, 2, 2, 2, 3, 4, 3, 3, false, false),
                MobPart::same(34, 6, 2, 2, 10, 4, 3, 3, false, false),
                MobPart::same(33, 9, 1, 1, 7, 10, 2, 2, false, false),
            ],
        ),
        "magma_cube" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 8, 64, 64, None, 0x004A_180E),
            16,
            16,
            &[
                MobPart::same(8, 26, 8, 1, 0, 6, 16, 3, false, false),
                MobPart::same(8, 35, 8, 1, 0, 9, 16, 3, false, false),
            ],
        ),
        "squid" | "glow_squid" => {
            let col = if sp == "glow_squid" { 0x0032_A8AE } else { 0x0036_5977 };
            MobModelSpec::shaped(
                MobFaceRect::custom(12, 12, 12, 16, 64, 32, None, col),
                12,
                16,
                &[
                    MobPart::same(48, 2, 2, 12, 1, 15, 2, 7, false, false),
                    MobPart::same(48, 2, 2, 12, 5, 15, 2, 7, false, false),
                    MobPart::same(48, 2, 2, 12, 9, 15, 2, 7, false, false),
                    MobPart::same(48, 2, 2, 12, 13, 15, 2, 7, false, false),
                ],
            )
        }
        "bat" => MobModelSpec::shaped(
            MobFaceRect::custom(0, 8, 8, 4, 32, 32, None, 0x0057_4840),
            16,
            11,
            &[
                MobPart::same(12, 15, 2, 2, 1, -4, 4, 4, false, false),
                MobPart::same(12, 15, 2, 2, 11, -4, 4, 4, false, false),
            ],
        ),
        "nautilus" | "zombie_nautilus" => {
            let col = if sp == "zombie_nautilus" { 0x006A_7E56 } else { 0x00DF_8C60 };
            MobModelSpec::shaped(
                MobFaceRect::custom(18, 12, 18, 18, 128, 128, None, col),
                16,
                15,
                &[
                    MobPart::same(14, 57, 14, 9, 2, 10, 12, 6, false, true),
                    MobPart::same(6, 69, 4, 4, 1, 9, 4, 5, false, true),
                    MobPart::same(28, 69, 4, 4, 11, 9, 4, 5, false, true),
                ],
            )
        }
        "ghast" => MobModelSpec::shaped(
            MobFaceRect::custom(16, 16, 16, 16, 128, 64, None, 0x00F0_F0F0),
            16,
            16,
            &[
                MobPart::same(2, 2, 2, 10, 2, 15, 2, 4, false, false),
                MobPart::same(2, 2, 2, 12, 7, 15, 2, 5, false, false),
                MobPart::same(2, 2, 2, 10, 12, 15, 2, 4, false, false),
            ],
        ),
        "happy_ghast" => MobModelSpec::shaped(
            MobFaceRect::custom(16, 16, 16, 16, 128, 128, None, 0x00F0_F0F0),
            16,
            16,
            &[
                MobPart::same(2, 2, 2, 10, 2, 15, 2, 4, false, false),
                MobPart::same(2, 2, 2, 12, 7, 15, 2, 5, false, false),
                MobPart::same(2, 2, 2, 10, 12, 15, 2, 4, false, false),
            ],
        ),
        "wolf" => MobModelSpec::shaped(
            MobFaceRect::custom(4, 4, 6, 6, 64, 32, None, 0x00CC_C8C0),
            16,
            16,
            &[
                MobPart::same(17, 15, 2, 2, 1, -3, 4, 3, false, false),
                MobPart::same(17, 15, 2, 2, 11, -3, 4, 3, false, false),
                MobPart::same(4, 14, 3, 3, 4, 10, 8, 5, false, true),
            ],
        ),
        "ocelot" | "cat" => MobModelSpec::shaped(
            MobFaceRect::custom(5, 5, 5, 4, 64, 32, None, 0x00D8_9E55),
            16,
            13,
            &[
                MobPart::same(1, 11, 1, 2, 1, -3, 3, 3, false, false),
                MobPart::same(7, 11, 1, 2, 12, -3, 3, 3, false, false),
                MobPart::same(3, 26, 3, 2, 5, 8, 6, 5, false, true),
            ],
        ),
        "silverfish" => {
            MobModelSpec::simple(MobFaceRect::custom(2, 2, 3, 2, 64, 32, None, 0x008A_9194))
        }

        // Standard 64x64 humanoids + overlay variants
        "zombie" | "husk" | "camel_husk" => MobModelSpec::simple(MobFaceRect::std64(0x0056_8245)),
        "drowned" => MobModelSpec::shaped(
            MobFaceRect::std64(0x005C_B2A8),
            16,
            16,
            &[
                MobPart::companion(8, 8, 8, 8, 0, 0, 16, 16, OverlaySource::DrownedOuter, false),
                MobPart::companion(40, 8, 8, 8, 0, 0, 16, 16, OverlaySource::DrownedOuter, false),
            ],
        ),
        "zombie_villager" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 10, 64, 64, Some([40, 8, 8, 10]), 0x005D_8548),
            13,
            16,
            &[MobPart::same(26, 2, 2, 4, 6, 10, 4, 6, false, true)],
        ),
        "villager" | "wandering_trader" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 10, 64, 64, None, 0x00B8_8464),
            13,
            16,
            &[MobPart::same(26, 2, 2, 4, 6, 10, 4, 6, false, true)],
        ),
        "witch" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 10, 64, 128, None, 0x007E_5490),
            13,
            16,
            &[
                MobPart::same(10, 74, 10, 4, -1, -3, 18, 4, false, false),
                MobPart::same(7, 81, 7, 4, 3, -7, 10, 4, false, false),
                MobPart::same(26, 2, 2, 4, 6, 10, 4, 6, false, true),
            ],
        ),
        "pillager" | "vindicator" | "evoker" | "illusioner" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 10, 64, 64, Some([40, 8, 8, 10]), 0x008B_9694),
            13,
            16,
            &[MobPart::same(26, 2, 2, 4, 6, 10, 4, 6, false, true)],
        ),
        "enderman" => MobModelSpec::shaped(
            MobFaceRect::std32(0x001B_1B1F),
            16,
            16,
            &[MobPart::companion(8, 8, 8, 8, 0, 0, 16, 16, OverlaySource::EndermanEyes, false)],
        ),
        "blaze" => MobModelSpec::shaped(
            MobFaceRect::std32(0x00F4_A924),
            16,
            16,
            &[
                MobPart::same(2, 18, 2, 8, -4, 3, 3, 12, true, false),
                MobPart::same(2, 18, 2, 8, 17, 3, 3, 12, true, false),
            ],
        ),
        "piglin" | "piglin_brute" | "zombified_piglin" => MobModelSpec::shaped(
            MobFaceRect::std64(0x00D8_9A8A),
            16,
            16,
            &[
                MobPart::same(52, 7, 1, 4, -2, 5, 2, 8, false, true),
                MobPart::same(40, 7, 1, 4, 16, 5, 2, 8, false, true),
                MobPart::same(33, 2, 4, 4, 4, 8, 8, 6, false, true),
            ],
        ),
        "player" => MobModelSpec::simple(MobFaceRect::std64(0x00E6_C4A2)),
        "allay" => MobModelSpec::shaped(
            MobFaceRect::custom(5, 5, 5, 5, 32, 32, None, 0x0053_C9F5),
            15,
            15,
            &[
                MobPart::same(16, 15, 6, 8, -5, 2, 6, 11, true, false),
                MobPart::same(16, 15, 6, 8, 15, 2, 6, 11, true, false),
            ],
        ),
        "vex" => MobModelSpec::shaped(
            MobFaceRect::custom(5, 5, 5, 5, 32, 32, None, 0x0092_B2CE),
            15,
            15,
            &[
                MobPart::same(16, 15, 6, 8, -5, 2, 6, 11, true, false),
                MobPart::same(16, 15, 6, 8, 15, 2, 6, 11, true, false),
            ],
        ),
        "breeze" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 8, 32, 32, None, 0x008D_B0E8),
            16,
            16,
            &[MobPart::companion(8, 8, 8, 8, 0, 0, 16, 16, OverlaySource::BreezeEyes, false)],
        ),
        "copper_golem" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 8, 64, 64, None, 0x00D1_7249),
            16,
            16,
            &[MobPart::companion(8, 8, 8, 8, 0, 0, 16, 16, OverlaySource::CopperGolemEyes, false)],
        ),
        "iron_golem" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 10, 128, 128, None, 0x00D6_D1CB),
            13,
            16,
            &[MobPart::same(26, 2, 2, 4, 6, 10, 4, 6, false, true)],
        ),
        "snow_golem" => MobModelSpec::simple(MobFaceRect::custom(8, 8, 8, 8, 64, 64, None, 0x00EC_F5F5)),
        "ravager" => MobModelSpec::shaped(
            MobFaceRect::custom(16, 16, 16, 20, 128, 128, None, 0x005B_585B),
            15,
            16,
            &[
                MobPart::same(76, 57, 2, 10, -3, -2, 3, 10, false, true),
                MobPart::same(76, 57, 2, 10, 16, -2, 3, 10, false, true),
            ],
        ),
        "warden" => MobModelSpec::shaped(
            MobFaceRect::custom(16, 16, 16, 16, 128, 128, None, 0x000F_3E4C),
            16,
            16,
            &[
                MobPart::same(58, 0, 16, 16, -8, -4, 9, 12, false, false),
                MobPart::same(58, 0, 16, 16, 15, -4, 9, 12, false, false),
            ],
        ),
        "hoglin" | "zoglin" => MobModelSpec::shaped(
            MobFaceRect::custom(80, 19, 14, 19, 128, 64, None, 0x00C6_7C6E),
            16,
            15,
            &[
                MobPart::same(3, 15, 6, 3, -4, 2, 5, 5, false, false),
                MobPart::same(3, 8, 6, 3, 15, 2, 5, 5, false, false),
                MobPart::same(2, 14, 2, 6, 0, 10, 3, 6, false, true),
                MobPart::same(11, 14, 2, 6, 13, 10, 3, 6, false, true),
            ],
        ),
        "sniffer" => MobModelSpec::shaped(
            MobFaceRect::custom(26, 26, 26, 18, 192, 192, None, 0x009D_382B),
            16,
            13,
            &[
                MobPart::same(8, 27, 4, 18, -3, 1, 4, 13, false, true),
                MobPart::same(8, 27, 4, 18, 15, 1, 4, 13, false, true),
            ],
        ),
        "creaking" => MobModelSpec::shaped(
            MobFaceRect::custom(7, 7, 6, 10, 64, 64, None, 0x0066_5D52),
            12,
            16,
            &[MobPart::companion(7, 7, 6, 10, 0, 0, 16, 16, OverlaySource::CreakingEyes, false)],
        ),
        "bee" => MobModelSpec::shaped(
            MobFaceRect::custom(10, 10, 7, 7, 64, 64, None, 0x00EB_BE34),
            16,
            16,
            &[
                MobPart::same(2, 0, 1, 3, 3, -3, 2, 3, false, false),
                MobPart::same(2, 0, 1, 3, 11, -3, 2, 3, false, false),
            ],
        ),
        "fox" => MobModelSpec::shaped(
            MobFaceRect::custom(6, 11, 8, 6, 48, 32, None, 0x00E0_6F27),
            16,
            13,
            &[
                MobPart::same(9, 2, 2, 2, 1, -4, 4, 4, false, false),
                MobPart::same(16, 2, 2, 2, 11, -4, 4, 4, false, false),
                MobPart::same(9, 21, 4, 2, 4, 11, 8, 5, false, true),
            ],
        ),
        "axolotl" => MobModelSpec::shaped(
            MobFaceRect::custom(5, 5, 8, 5, 64, 64, None, 0x00F2_99CE),
            16,
            12,
            &[
                MobPart::same(0, 41, 3, 6, -4, 1, 4, 11, false, false),
                MobPart::same(12, 41, 3, 6, 16, 1, 4, 11, false, false),
            ],
        ),
        "frog" => MobModelSpec::shaped(
            MobFaceRect::custom(7, 7, 7, 4, 48, 48, None, 0x00C6_6B3A),
            16,
            11,
            &[
                MobPart::same(2, 2, 3, 3, 1, -4, 5, 5, false, false),
                MobPart::same(2, 7, 3, 3, 10, -4, 5, 5, false, false),
            ],
        ),
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" => MobModelSpec::shaped(
            MobFaceRect::custom(7, 20, 7, 5, 64, 64, None, 0x008A_5D3B),
            16,
            12,
            &[
                MobPart::same(20, 17, 2, 3, 1, -4, 3, 4, false, false),
                MobPart::same(20, 17, 2, 3, 12, -4, 3, 4, false, false),
                MobPart::same(5, 30, 5, 5, 3, 9, 10, 6, false, true),
            ],
        ),
        "llama" | "trader_llama" => MobModelSpec::shaped(
            MobFaceRect::custom(19, 19, 8, 10, 128, 64, None, 0x00DF_C59B),
            13,
            16,
            &[
                MobPart::same(19, 2, 3, 3, 1, -4, 4, 4, false, false),
                MobPart::same(19, 2, 3, 3, 11, -4, 4, 4, false, false),
                MobPart::same(4, 4, 4, 4, 4, 9, 8, 5, false, true),
            ],
        ),
        "panda" => MobModelSpec::shaped(
            MobFaceRect::custom(10, 10, 13, 10, 64, 64, None, 0x00EB_EBEC),
            16,
            13,
            &[
                MobPart::same(49, 17, 5, 4, -1, -4, 5, 5, false, false),
                MobPart::same(49, 17, 5, 4, 12, -4, 5, 5, false, false),
            ],
        ),
        "polar_bear" => MobModelSpec::shaped(
            MobFaceRect::custom(7, 7, 7, 7, 128, 64, None, 0x00F2_F5F6),
            16,
            16,
            &[
                MobPart::same(27, 1, 2, 2, 1, -2, 3, 3, false, false),
                MobPart::same(27, 1, 2, 2, 12, -2, 3, 3, false, false),
                MobPart::same(3, 47, 5, 3, 4, 9, 8, 6, false, true),
            ],
        ),
        "rabbit" => MobModelSpec::shaped(
            MobFaceRect::custom(5, 5, 5, 4, 64, 32, None, 0x00AE_8E70),
            16,
            13,
            &[
                MobPart::same(54, 1, 2, 5, 2, -7, 4, 7, false, false),
                MobPart::same(59, 1, 2, 5, 10, -7, 4, 7, false, false),
            ],
        ),
        "guardian" | "elder_guardian" => {
            let col = if sp == "elder_guardian" { 0x00B8_B69D } else { 0x0059_9486 };
            MobModelSpec::shaped(
                MobFaceRect::custom(16, 16, 16, 16, 64, 64, None, col),
                16,
                16,
                &[MobPart::same(9, 1, 4, 4, 4, 4, 8, 8, false, true)],
            )
        }
        "phantom" => MobModelSpec::shaped(
            MobFaceRect::custom(5, 5, 7, 3, 64, 64, None, 0x004D_597A),
            16,
            10,
            &[MobPart::companion(5, 5, 7, 3, 0, 0, 16, 16, OverlaySource::PhantomEyes, false)],
        ),
        "shulker" => MobModelSpec::simple(MobFaceRect::custom(6, 58, 6, 6, 64, 64, None, 0x0093_6693)),
        "strider" => MobModelSpec::shaped(
            MobFaceRect::custom(16, 16, 16, 14, 64, 128, None, 0x00A1_3636),
            16,
            14,
            &[
                MobPart::same(16, 48, 12, 4, -4, 3, 4, 6, false, false),
                MobPart::same(16, 48, 12, 4, 16, 3, 4, 6, false, false),
            ],
        ),
        "camel" => MobModelSpec::shaped(
            MobFaceRect::custom(33, 19, 7, 14, 128, 128, None, 0x00D8_AE6E),
            13,
            16,
            &[
                MobPart::same(46, 1, 3, 2, -2, 0, 4, 4, false, false),
                MobPart::same(46, 1, 3, 2, 14, 0, 4, 4, false, false),
            ],
        ),
        "armadillo" => MobModelSpec::shaped(
            MobFaceRect::custom(46, 20, 4, 5, 64, 64, None, 0x00A8_7F70),
            14,
            15,
            &[
                MobPart::same(43, 11, 2, 5, -2, -2, 4, 7, false, false),
                MobPart::same(48, 11, 2, 5, 14, -2, 4, 7, false, false),
            ],
        ),
        "goat" => MobModelSpec::shaped(
            MobFaceRect::custom(34, 6, 6, 7, 64, 64, None, 0x00D6_CEBE),
            14,
            16,
            &[
                MobPart::same(14, 57, 2, 7, 2, -6, 3, 7, false, false),
                MobPart::same(14, 57, 2, 7, 11, -6, 3, 7, false, false),
                MobPart::same(4, 62, 3, 2, -3, 2, 4, 3, false, false),
                MobPart::same(4, 62, 3, 2, 15, 2, 4, 3, false, false),
            ],
        ),
        "turtle" => MobModelSpec::shaped(
            MobFaceRect::custom(7, 7, 6, 5, 128, 64, None, 0x0049_9B42),
            16,
            13,
            &[MobPart::same(17, 45, 14, 10, -2, -3, 20, 19, true, false)],
        ),
        "dolphin" => MobModelSpec::shaped(
            MobFaceRect::custom(6, 6, 6, 5, 64, 64, None, 0x0092_A4B8),
            16,
            14,
            &[MobPart::same(3, 22, 4, 2, 5, 9, 6, 5, false, true)],
        ),
        "cod" => MobModelSpec::shaped(
            MobFaceRect::custom(11, 3, 6, 4, 32, 32, None, 0x00B8_A282),
            16,
            11,
            &[
                MobPart::same(0, 7, 7, 4, 14, 1, 8, 14, false, false),
            ],
        ),
        "salmon" => MobModelSpec::shaped(
            MobFaceRect::custom(24, 3, 4, 4, 32, 32, None, 0x00A5_4E48),
            14,
            14,
            &[
                MobPart::same(0, 8, 8, 4, 13, 1, 9, 13, false, false),
            ],
        ),
        "tropical_fish" => MobModelSpec::shaped(
            MobFaceRect::custom(2, 2, 6, 4, 32, 32, None, 0x00F0_8830),
            17,
            10,
            &[MobPart::same(22, 0, 4, 4, 15, 1, 5, 14, false, false)],
        ),
        "tadpole" => MobModelSpec::shaped(
            MobFaceRect::custom(2, 2, 4, 3, 16, 16, None, 0x004B_3A2A),
            16,
            10,
            &[MobPart::same(0, 7, 6, 3, 15, 3, 7, 9, false, false)],
        ),
        "pufferfish" => MobModelSpec::simple(MobFaceRect::custom(6, 6, 8, 8, 32, 32, None, 0x00E4_B744)),
        "parrot" => MobModelSpec::shaped(
            MobFaceRect::custom(4, 4, 3, 3, 32, 32, None, 0x00D8_3428),
            14,
            14,
            &[MobPart::same(12, 8, 2, 2, 5, 8, 6, 5, false, true)],
        ),
        "wither" => MobModelSpec::shaped(
            MobFaceRect::custom(8, 8, 8, 8, 64, 64, None, 0x002B_2B2E),
            16,
            16,
            &[
                MobPart::same(8, 8, 8, 8, -7, 3, 9, 10, true, false),
                MobPart::same(8, 8, 8, 8, 14, 3, 9, 10, true, false),
            ],
        ),
        "ender_dragon" => MobModelSpec::shaped(
            MobFaceRect::custom(128, 46, 16, 16, 256, 256, None, 0x0022_1C28),
            16,
            16,
            &[
                MobPart::same(192, 60, 12, 8, 2, 10, 12, 7, false, true),
            ],
        ),
        "endermite" => MobModelSpec::simple(MobFaceRect::custom(2, 2, 4, 3, 64, 32, None, 0x003B_2848)),
        _ => MobModelSpec::simple(MobFaceRect::std64(0x00EE_DF44)),
    }
}

/// Base face UV rect lookup for callers needing a single face box.
pub fn mob_face_spec(species_id: &str) -> MobFaceRect {
    mob_model_spec(species_id, 0, 0).base
}
