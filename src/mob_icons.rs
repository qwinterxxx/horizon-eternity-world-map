//! Hand-crafted pixel-art Minecraft mob icons for razor-sharp minimap/world-map rendering.
//!
//! `squid`, `glow_squid`, `skeleton`, `wither_skeleton`, `stray`, `bogged`, `parched`,
//! `chicken`, `pig`, and `player` return `None` so they keep their texture-composited icons.

use crate::entities::normalize_species;

#[derive(Clone, Copy, Debug)]
pub struct PixelMobIcon {
    pub w: u8,
    pub h: u8,
    pub rows: &'static [&'static str],
    pub palette: &'static [(u8, u32)],
}

impl PixelMobIcon {
    #[inline]
    pub fn color_of(&self, ch: u8) -> Option<u32> {
        if ch == b'.' {
            return None;
        }
        for &(k, col) in self.palette {
            if k == ch {
                return Some(col);
            }
        }
        None
    }
}

macro_rules! icon {
    ($w:expr, $h:expr, [ $($row:expr),+ $(,)? ], [ $( ($k:expr, $c:expr) ),+ $(,)? ]) => {
        PixelMobIcon {
            w: $w,
            h: $h,
            rows: &[ $($row),+ ],
            palette: &[ $( ($k, $c) ),+ ],
        }
    };
}

// ============================================================================
// HOSTILE & NEUTRAL OVERWORLD MOBS
// ============================================================================

const CREEPER: PixelMobIcon = icon!(
    8, 8,
    [
        "gGlGgLgG",
        "GgGglGGg",
        "g##gG##g",
        "G##lg##G",
        "gGg##gGg",
        "Gl####gG",
        "gG####Gg",
        "Gg#gG#gL",
    ],
    [
        (b'g', 0xFF3E_A838),
        (b'G', 0xFF52_C44A),
        (b'l', 0xFF72_DC68),
        (b'L', 0xFF2E_8229),
        (b'#', 0xFF10_1010),
    ]
);

const ZOMBIE: PixelMobIcon = icon!(
    8, 8,
    [
        "dddddddd",
        "ddggggdd",
        "dggggggd",
        "ggGggGgg",
        "g##gg##g",
        "ggGmmGgg",
        "ggmmmmgg",
        "dggggggd",
    ],
    [
        (b'd', 0xFF36_5828),
        (b'g', 0xFF53_7D3C),
        (b'G', 0xFF66_944C),
        (b'm', 0xFF3E_612C),
        (b'#', 0xFF10_1410),
    ]
);

const HUSK: PixelMobIcon = icon!(
    8, 8,
    [
        "dddddddd",
        "ddssssdd",
        "dssssssd",
        "ssLssLss",
        "s##ss##s",
        "ssLmmLss",
        "ssmmmmss",
        "dssssssd",
    ],
    [
        (b'd', 0xFF5E_523E),
        (b's', 0xFF9E_8C68),
        (b'L', 0xFFB8_A67E),
        (b'm', 0xFF75_674E),
        (b'#', 0xFF18_1410),
    ]
);

const DROWNED: PixelMobIcon = icon!(
    8, 8,
    [
        "dddddddd",
        "ddttttdd",
        "dttttttd",
        "ttLttLtt",
        "tccTTcct",
        "ttLmmLtt",
        "ttmmmmtt",
        "dttttttd",
    ],
    [
        (b'd', 0xFF28_5E54),
        (b't', 0xFF4A_8E82),
        (b'L', 0xFF68_B2A4),
        (b'T', 0xFF3B_786E),
        (b'm', 0xFF2B_574E),
        (b'c', 0xFF78_FFF0),
    ]
);

const ZOMBIE_VILLAGER: PixelMobIcon = icon!(
    8, 10,
    [
        "gGGgGGgG",
        "dggggggd",
        "dggGGggd",
        "dggggggd",
        "dbbbbbbd",
        "gWRddRWg",
        "dggnnggd",
        "dmmnnmmd",
        "dggnnggd",
        "...nn...",
    ],
    [
        (b'g', 0xFF66_8A3F),
        (b'G', 0xFF77_A047),
        (b'd', 0xFF41_5928),
        (b'b', 0xFF33_2411),
        (b'W', 0xFFFF_FFFF),
        (b'R', 0xFF99_2B2B),
        (b'n', 0xFF55_7834),
        (b'm', 0xFF2C_3D1C),
    ]
);

const SPIDER: PixelMobIcon = icon!(
    8, 8,
    [
        "ddbbbbdd",
        "dbbbbbbd",
        "bbRbbRbb",
        "bRRbbRRb",
        "bbRbbRbb",
        "bbrbbrbb",
        "bb#bb#bb",
        "ddbbbbdd",
    ],
    [
        (b'd', 0xFF22_1C1E),
        (b'b', 0xFF3A_3032),
        (b'R', 0xFFE6_1E24),
        (b'r', 0xFF9C_1418),
        (b'#', 0xFF14_1012),
    ]
);

const CAVE_SPIDER: PixelMobIcon = icon!(
    8, 8,
    [
        "ddbbbbdd",
        "dbbbbbbd",
        "bbRbbRbb",
        "bRRbbRRb",
        "bbRbbRbb",
        "bbrbbrbb",
        "bb#bb#bb",
        "ddbbbbdd",
    ],
    [
        (b'd', 0xFF0E_242B),
        (b'b', 0xFF1B_424E),
        (b'R', 0xFFFF_2228),
        (b'r', 0xFFA8_1418),
        (b'#', 0xFF0A_161A),
    ]
);

const ENDERMAN: PixelMobIcon = icon!(
    8, 8,
    [
        "bbbbbbbb",
        "bddddddB",
        "bbbbbbbb",
        "bbbbbbbb",
        "pPpbBpPp",
        "bbbbbbbb",
        "bddddddB",
        "bbbbbbbb",
    ],
    [
        (b'b', 0xFF16_161A),
        (b'd', 0xFF1E_1E24),
        (b'B', 0xFF10_1014),
        (b'p', 0xFFCC_44FF),
        (b'P', 0xFFF4_B8FF),
    ]
);

const SLIME: PixelMobIcon = icon!(
    8, 8,
    [
        "oooooooo",
        "oGGGGGGo",
        "oG##G##o",
        "oG##G##o",
        "oGGGGGGo",
        "oGGG#GGo",
        "oGGGGGGo",
        "oooooooo",
    ],
    [
        (b'o', 0xFF88_DC72),
        (b'G', 0xFF56_B244),
        (b'#', 0xFF16_3810),
    ]
);

const MAGMA_CUBE: PixelMobIcon = icon!(
    8, 8,
    [
        "dddddddd",
        "dbbbbbbd",
        "dbbbbbbd",
        "dOYbbYOd",
        "dOYbbYOd",
        "dbbbbbbd",
        "drrddrrd",
        "dddddddd",
    ],
    [
        (b'd', 0xFF28_110D),
        (b'b', 0xFF42_1C14),
        (b'r', 0xFF7A_2810),
        (b'O', 0xFFFF_6600),
        (b'Y', 0xFFFF_CC22),
    ]
);

const WITCH: PixelMobIcon = icon!(
    8, 10,
    [
        "...hh...",
        "..hhhh..",
        ".hpppph.",
        "hhhhhhhh",
        "dddddddd",
        "dwwddwwd",
        "dp#dd#pd",
        "dddnnndd",
        "dddnnwdd",
        "dddddddd",
    ],
    [
        (b'h', 0xFF1C_1A24),
        (b'p', 0xFF7C_38A8),
        (b'd', 0xFFC2_A088),
        (b'n', 0xFFA6_826A),
        (b'w', 0xFF52_8C44),
        (b'#', 0xFF1A_141E),
    ]
);

const PILLAGER: PixelMobIcon = icon!(
    8, 9,
    [
        "gggggggg",
        "gggggggg",
        "gddddddg",
        "g##gg##g",
        "gc#gg#cg",
        "gggnnggg",
        "gggnnggg",
        "gggnnggg",
        "gggggggg",
    ],
    [
        (b'g', 0xFF8C_9492),
        (b'd', 0xFF4E_5452),
        (b'n', 0xFF74_7C7A),
        (b'c', 0xFF4C_A8A0),
        (b'#', 0xFF16_1A1A),
    ]
);

const EVOKER: PixelMobIcon = icon!(
    8, 9,
    [
        "gggggggg",
        "gggggggg",
        "gddddddg",
        "g##gg##g",
        "ge#gg#eg",
        "gggnnggg",
        "gggnnggg",
        "gggnnggg",
        "gggggggg",
    ],
    [
        (b'g', 0xFF8F_9795),
        (b'd', 0xFF3E_4242),
        (b'n', 0xFF74_7C7A),
        (b'e', 0xFF4E_C258),
        (b'#', 0xFF16_1A1A),
    ]
);

const PHANTOM: PixelMobIcon = icon!(
    8, 6,
    [
        "dddddddd",
        "dbbbbbbd",
        "bGGbbGGb",
        "bGGbbGGb",
        "dbbbbbbd",
        ".dddddd.",
    ],
    [
        (b'd', 0xFF35_405C),
        (b'b', 0xFF4B_5A80),
        (b'G', 0xFF55_FF66),
    ]
);

const SILVERFISH: PixelMobIcon = icon!(
    8, 6,
    [
        ".dddddd.",
        "dggggggd",
        "g#gggg#g",
        "gggggggg",
        "dggggggd",
        ".dddddd.",
    ],
    [
        (b'd', 0xFF5E_6468),
        (b'g', 0xFF98_A0A4),
        (b'#', 0xFF16_1618),
    ]
);

const ENDERMITE: PixelMobIcon = icon!(
    8, 6,
    [
        ".dddddd.",
        "dppppppd",
        "ppRppRpp",
        "pppppppp",
        "dppppppd",
        ".dddddd.",
    ],
    [
        (b'd', 0xFF28_1836),
        (b'p', 0xFF4C_2E64),
        (b'R', 0xFFFF_3855),
    ]
);

const GUARDIAN: PixelMobIcon = icon!(
    8, 8,
    [
        "t.tttt.t",
        ".tTTTTt.",
        "tTTwwTTt",
        "tTwRRwTt",
        "tTTwwTTt",
        "tTTTTTTt",
        ".tTTTTt.",
        "t.tttt.t",
    ],
    [
        (b't', 0xFF3D_6E62),
        (b'T', 0xFF5C_9A8C),
        (b'w', 0xFFF2_F2EE),
        (b'R', 0xFFCC_3828),
    ]
);

const ELDER_GUARDIAN: PixelMobIcon = icon!(
    8, 8,
    [
        "t.tttt.t",
        ".tTTTTt.",
        "tTTwwTTt",
        "tTwBBwTt",
        "tTTwwTTt",
        "tTTTTTTt",
        ".tTTTTt.",
        "t.tttt.t",
    ],
    [
        (b't', 0xFF88_8474),
        (b'T', 0xFFC2_BFA8),
        (b'w', 0xFFF4_F4F0),
        (b'B', 0xFF3E_5288),
    ]
);

const SHULKER: PixelMobIcon = icon!(
    8, 8,
    [
        "pppppppp",
        "pPPPPPPp",
        "pPyyyyPp",
        "pPy#y#Pp",
        "pPyyyyPp",
        "pPyyyyPp",
        "pPPPPPPp",
        "pppppppp",
    ],
    [
        (b'p', 0xFF70_4670),
        (b'P', 0xFF9A_689A),
        (b'y', 0xFFE6_DC9E),
        (b'#', 0xFF1A_161A),
    ]
);

const RAVAGER: PixelMobIcon = icon!(
    10, 8,
    [
        "h.gggggg.h",
        "h.gggggg.h",
        "hhgddddghh",
        ".gg#gg#gg.",
        ".gggnnggg.",
        ".gggnnggg.",
        ".gddddddg.",
        "..gggggg..",
    ],
    [
        (b'h', 0xFFD4_CEC4),
        (b'g', 0xFF5A_565A),
        (b'd', 0xFF36_3236),
        (b'n', 0xFF48_4448),
        (b'#', 0xFF14_1414),
    ]
);

const WARDEN: PixelMobIcon = icon!(
    10, 8,
    [
        "cc.dddd.cc",
        "cCdTTTTdCc",
        ".cdTTTTdc.",
        "..dT##Td..",
        "..d####d..",
        "..d#cc#d..",
        "..d####d..",
        "..dddddd..",
    ],
    [
        (b'd', 0xFF0A_242E),
        (b'T', 0xFF12_404E),
        (b'c', 0xFF1E_B8C8),
        (b'C', 0xFF66_F2FF),
        (b'#', 0xFF06_1218),
    ]
);

const BREEZE: PixelMobIcon = icon!(
    8, 8,
    [
        "llbbbbll",
        "lbbllbbl",
        "bbbbbbbb",
        "bY#bb#Yb",
        "bYYbbYYb",
        "bbllllbb",
        "bbbbbbbb",
        "llbbbbll",
    ],
    [
        (b'b', 0xFF84_9ED8),
        (b'l', 0xFFB8_CCF2),
        (b'Y', 0xFFFF_E855),
        (b'#', 0xFF36_4468),
    ]
);

const CREAKING: PixelMobIcon = icon!(
    8, 9,
    [
        ".dddddd.",
        ".dbbbbd.",
        ".dbOObd.",
        ".dbbbbd.",
        ".dObbOd.",
        ".dbbbbd.",
        ".dbddbd.",
        ".dbbbbd.",
        ".dddddd.",
    ],
    [
        (b'd', 0xFF3C_3630),
        (b'b', 0xFF60_564C),
        (b'O', 0xFFFF_8818),
    ]
);

const VEX: PixelMobIcon = icon!(
    10, 7,
    [
        "..bbbbbb..",
        "wwbddddbww",
        "wwb#bb#bww",
        ".wbCbbCbw.",
        "..bbbbbb..",
        "..bbddbb..",
        "..bbbbbb..",
    ],
    [
        (b'b', 0xFF8A_A6C4),
        (b'd', 0xFF56_6C84),
        (b'w', 0xFFC8_D8E8),
        (b'C', 0xFFCC_EEFF),
        (b'#', 0xFF20_2C3C),
    ]
);

// ============================================================================
// PASSIVE & FARM MOBS
// ============================================================================

const COW: PixelMobIcon = icon!(
    8, 8,
    [
        "hbbbbbbh",
        "bbwbbbbb",
        "bbwwbbbb",
        "w#bbbb#w",
        "bbbbbbbb",
        "bmmmmmmb",
        "bm#mm#mb",
        "bmmmmmmb",
    ],
    [
        (b'h', 0xFFC8_C4BC),
        (b'b', 0xFF5C_3D28),
        (b'w', 0xFFE8_E4DC),
        (b'm', 0xFFD4_B4A8),
        (b'#', 0xFF18_1412),
    ]
);

const MOOSHROOM: PixelMobIcon = icon!(
    8, 8,
    [
        "hrrrrrrh",
        "rrwrrrrr",
        "rrwwrrrr",
        "w#rrrr#w",
        "rrrrrrrr",
        "rmmmmmmr",
        "rm#mm#mr",
        "rmmmmmmr",
    ],
    [
        (b'h', 0xFFC8_C4BC),
        (b'r', 0xFF9C_2018),
        (b'w', 0xFFEA_E6E0),
        (b'm', 0xFFD4_A8A0),
        (b'#', 0xFF18_1212),
    ]
);

const SHEEP: PixelMobIcon = icon!(
    8, 8,
    [
        "WWwWWwWW",
        "wWWWWWWw",
        "wffFFffw",
        "w#WFFW#w",
        "wffFFffw",
        "wwfPPfww",
        "wwfppfww",
        ".wwwwww.",
    ],
    [
        (b'W', 0xFFF8_F8F8),
        (b'w', 0xFFDE_DEDE),
        (b'F', 0xFFC0_9E86),
        (b'f', 0xFFB7_947B),
        (b'P', 0xFFFF_B8B8),
        (b'p', 0xFFE6_9494),
        (b'#', 0xFF08_0808),
    ]
);

const WOLF: PixelMobIcon = icon!(
    8, 8,
    [
        "gg....gg",
        "gGGGGGGg",
        "GGGGGGGG",
        "G#wGGw#G",
        "GGwwwwGG",
        "GGw##wGG",
        "GGwwwwGG",
        ".GGGGGG.",
    ],
    [
        (b'g', 0xFF96_928C),
        (b'G', 0xFFD2_CEC6),
        (b'w', 0xFFE8_E6E0),
        (b'#', 0xFF1A_1818),
    ]
);

const CAT: PixelMobIcon = icon!(
    8, 8,
    [
        "oo....oo",
        "oOOOOOOo",
        "OOOOOOOO",
        "O#gOOg#O",
        "OOllllOO",
        "OOlppLOO",
        "OOllllOO",
        ".OOOOOO.",
    ],
    [
        (b'o', 0xFFB8_7830),
        (b'O', 0xFFE0_9C48),
        (b'l', 0xFFF4_E0BC),
        (b'L', 0xFFF4_E0BC),
        (b'g', 0xFF55_CC44),
        (b'p', 0xFFF0_90A0),
        (b'#', 0xFF1A_1612),
    ]
);

const FOX: PixelMobIcon = icon!(
    8, 8,
    [
        "ww....ww",
        "oOOOOOOo",
        "OOOOOOOO",
        "O#OOOO#O",
        "wwOOOOww",
        "www##www",
        ".wwwwww.",
        "..wwww..",
    ],
    [
        (b'o', 0xFFC0_5218),
        (b'O', 0xFFE8_7224),
        (b'w', 0xFFF2_ECE4),
        (b'#', 0xFF1A_1614),
    ]
);

const BEE: PixelMobIcon = icon!(
    8, 8,
    [
        ".#....#.",
        "yyyyyyyy",
        "yYYYYYYy",
        "c#YYYY#c",
        "c#YYYY#c",
        "c#YYYY#c",
        "yYYYYYYy",
        "yyyyyyyy",
    ],
    [
        (b'y', 0xFFD8_A41C),
        (b'Y', 0xFFF6_CA38),
        (b'c', 0xFF5C_D2E8),
        (b'#', 0xFF18_1614),
    ]
);

const HORSE: PixelMobIcon = icon!(
    8, 9,
    [
        "bb....bb",
        "bBBBBBBb",
        "BBBwBBBB",
        "B#BwBB#B",
        "BBBwBBBB",
        "BmmmmmmB",
        "Bm#mm#mB",
        "BmmmmmmB",
        ".mmmmmm.",
    ],
    [
        (b'b', 0xFF66_4024),
        (b'B', 0xFF94_6038),
        (b'w', 0xFFEC_E8E0),
        (b'm', 0xFF7A_4E2C),
        (b'#', 0xFF18_1412),
    ]
);

const DONKEY: PixelMobIcon = icon!(
    8, 9,
    [
        "gg....gg",
        "gg....gg",
        "gGGGGGGg",
        "G#GGGG#G",
        "GGGGGGGG",
        "GmmmmmmG",
        "Gm#mm#mG",
        "GmmmmmmG",
        ".mmmmmm.",
    ],
    [
        (b'g', 0xFF60_5A54),
        (b'G', 0xFF8A_827A),
        (b'm', 0xFFB8_B0A6),
        (b'#', 0xFF18_1614),
    ]
);

const SKELETON_HORSE: PixelMobIcon = icon!(
    8, 9,
    [
        "gg....gg",
        "gWWWWWWg",
        "WWWWWWWW",
        "W#WWWW#W",
        "WWWWWWWW",
        "WggggggW",
        "Wg#gg#gW",
        "WggggggW",
        ".gggggg.",
    ],
    [
        (b'g', 0xFFA4_A49E),
        (b'W', 0xFFE2_E2DC),
        (b'#', 0xFF20_2020),
    ]
);

const ZOMBIE_HORSE: PixelMobIcon = icon!(
    8, 9,
    [
        "dd....dd",
        "dGGGGGGd",
        "GGGGGGGG",
        "G#GGGG#G",
        "GGGGGGGG",
        "GddddddG",
        "Gd#dd#dG",
        "GddddddG",
        ".dddddd.",
    ],
    [
        (b'd', 0xFF3A_5E2C),
        (b'G', 0xFF5E_8E46),
        (b'#', 0xFF14_1A12),
    ]
);

const LLAMA: PixelMobIcon = icon!(
    8, 9,
    [
        "cc....cc",
        "cc....cc",
        "cCCCCCCc",
        "C#CCCC#C",
        "CCCCCCCC",
        "CCwwwwCC",
        "CCw##wCC",
        "CCwwwwCC",
        ".CCCCCC.",
    ],
    [
        (b'c', 0xFFC2_A67A),
        (b'C', 0xFFE4_CA9E),
        (b'w', 0xFFF2_E6D2),
        (b'#', 0xFF1C_1814),
    ]
);

const CAMEL: PixelMobIcon = icon!(
    8, 9,
    [
        "dd....dd",
        "dCCCCCCd",
        "CCCCCCCC",
        "C#CCCC#C",
        "CCCCCCCC",
        "CddddddC",
        "Cd#dd#dC",
        "CddddddC",
        ".CCCCCC.",
    ],
    [
        (b'd', 0xFFB8_8C4E),
        (b'C', 0xFFDC_B070),
        (b'#', 0xFF1C_1814),
    ]
);

const GOAT: PixelMobIcon = icon!(
    8, 9,
    [
        ".h....h.",
        ".h....h.",
        "wWWWWWWw",
        "W#WWWW#W",
        "WWWWWWWW",
        "WWwwwwWW",
        "WWw##wWW",
        ".WWwwWW.",
        "...gg...",
    ],
    [
        (b'h', 0xFF68_5C50),
        (b'w', 0xFFC0_B8AC),
        (b'W', 0xFFE8_E2D8),
        (b'g', 0xFF9C_9488),
        (b'#', 0xFF20_1A16),
    ]
);

const RABBIT: PixelMobIcon = icon!(
    8, 9,
    [
        ".bb..bb.",
        ".bb..bb.",
        ".bb..bb.",
        "bBBBBBBb",
        "B#BBBB#B",
        "BBBppBBB",
        "BBppppBB",
        ".BBBBBB.",
        "........",
    ],
    [
        (b'b', 0xFF90_7258),
        (b'B', 0xFFBA_987A),
        (b'p', 0xFFF0_A6B0),
        (b'#', 0xFF1C_1614),
    ]
);

const PANDA: PixelMobIcon = icon!(
    8, 8,
    [
        "##....##",
        "#WWWWWW#",
        "WWWWWWWW",
        "W##WW##W",
        "W#wWWw#W",
        "WWW##WWW",
        "WW####WW",
        ".WWWWWW.",
    ],
    [
        (b'W', 0xFFF4_F4F6),
        (b'w', 0xFFC8_C8CE),
        (b'#', 0xFF18_181C),
    ]
);

const POLAR_BEAR: PixelMobIcon = icon!(
    8, 8,
    [
        "gg....gg",
        "gWWWWWWg",
        "WWWWWWWW",
        "W#WWWW#W",
        "WWwwwwWW",
        "WWw##wWW",
        "WWwwwwWW",
        ".WWWWWW.",
    ],
    [
        (b'g', 0xFFD2_D8DC),
        (b'W', 0xFFF4_F8FA),
        (b'w', 0xFFE2_E6EA),
        (b'#', 0xFF1A_1C20),
    ]
);

const ARMADILLO: PixelMobIcon = icon!(
    8, 8,
    [
        "aa....aa",
        "aAAAAAAa",
        "AAAAAAAA",
        "A#AAAA#A",
        "AAssssAA",
        "AAssssAA",
        ".AAAAAA.",
        "........",
    ],
    [
        (b'a', 0xFF86_6054),
        (b'A', 0xFFB0_8474),
        (b's', 0xFFC8_A090),
        (b'#', 0xFF1C_1614),
    ]
);

const SNIFFER: PixelMobIcon = icon!(
    10, 8,
    [
        ".gggggggg.",
        "rrggggggrr",
        "rrrrrrrrrr",
        "rr#rrrr#rr",
        "rrryyyyrrr",
        "rrry##yrrr",
        ".rryyyyrr.",
        "..rrrrrr..",
    ],
    [
        (b'g', 0xFF4C_9A42),
        (b'r', 0xFFA4_382A),
        (b'y', 0xFFE8_C248),
        (b'#', 0xFF1C_1614),
    ]
);

const BAT: PixelMobIcon = icon!(
    10, 7,
    [
        "..bb..bb..",
        "ddbbbbbbdd",
        "ddb#bb#bdd",
        "ddbbbbbbdd",
        ".dbbbbbbd.",
        "..bbbbbb..",
        "..........",
    ],
    [
        (b'd', 0xFF36_2A24),
        (b'b', 0xFF5A_483E),
        (b'#', 0xFF14_100E),
    ]
);

const PARROT: PixelMobIcon = icon!(
    8, 8,
    [
        "...rr...",
        "..rrrr..",
        ".rRRRRr.",
        ".R#RR#R.",
        ".RRyyRR.",
        ".RRyyRR.",
        "..bbbb..",
        "........",
    ],
    [
        (b'r', 0xFFB0_2218),
        (b'R', 0xFFE8_3428),
        (b'y', 0xFF42_4248),
        (b'b', 0xFF2C_5ECC),
        (b'#', 0xFF14_1414),
    ]
);

const ALLAY: PixelMobIcon = icon!(
    10, 7,
    [
        "..cccccc..",
        "wwcCCCCcww",
        "wwc#CC#cww",
        ".wcCCCCcw.",
        "..cCCCCc..",
        "..cccccc..",
        "..........",
    ],
    [
        (b'c', 0xFF3E_B4E8),
        (b'C', 0xFF6E_E2FF),
        (b'w', 0xFFB8_F4FF),
        (b'#', 0xFF16_3048),
    ]
);

// ============================================================================
// WATER MOBS (FISH, TURTLE, DOLPHIN, AXOLOTL, FROG, NAUTILUS)
// ============================================================================

const SALMON: PixelMobIcon = icon!(
    10, 6,
    [
        "....rr....",
        ".ggRRRR.t.",
        "g#gRRRRttt",
        "gggRRRRttt",
        ".gllllll.t",
        "..........",
    ],
    [
        (b'g', 0xFF56_7454),
        (b'R', 0xFFC2_4E46),
        (b'r', 0xFF8C_3630),
        (b'l', 0xFFD8_8A7E),
        (b't', 0xFF9C_3E38),
        (b'#', 0xFF14_1814),
    ]
);

const COD: PixelMobIcon = icon!(
    10, 6,
    [
        "....dd....",
        ".ddBBBB.t.",
        "d#dBBBBttt",
        "dddBBBBttt",
        ".dllllll.t",
        "..........",
    ],
    [
        (b'd', 0xFF8E_7858),
        (b'B', 0xFFC2_AA84),
        (b'l', 0xFFE2_D4BA),
        (b't', 0xFFA4_8E6C),
        (b'#', 0xFF18_1614),
    ]
);

const TROPICAL_FISH: PixelMobIcon = icon!(
    10, 6,
    [
        "...oooo...",
        ".ooWooo.t.",
        "o#oWoooWtt",
        "oooWoooWtt",
        ".ooWooo.t.",
        "..........",
    ],
    [
        (b'o', 0xFFF2_7A22),
        (b'W', 0xFFF4_F4F4),
        (b't', 0xFFD8_6218),
        (b'#', 0xFF16_1412),
    ]
);

const PUFFERFISH: PixelMobIcon = icon!(
    8, 8,
    [
        ".y.yy.y.",
        "yYYYYYYy",
        "Y#YYYY#Y",
        "YYYYYYYY",
        "YyyyyyyY",
        "YwwwwwwY",
        ".wwwwww.",
        "........",
    ],
    [
        (b'y', 0xFFC8_9828),
        (b'Y', 0xFFF0_C442),
        (b'w', 0xFFF4_ECDC),
        (b'#', 0xFF18_1612),
    ]
);

const TADPOLE: PixelMobIcon = icon!(
    10, 5,
    [
        ".bbbb.....",
        "b#bb#b....",
        "bbbbbbtttt",
        ".bbbb.....",
        "..........",
    ],
    [
        (b'b', 0xFF5A_4432),
        (b't', 0xFF7A_6048),
        (b'#', 0xFF14_100E),
    ]
);

const TURTLE: PixelMobIcon = icon!(
    8, 7,
    [
        ".ssssss.",
        "sGGGGGGs",
        "G#GGGG#G",
        "GGGGGGGG",
        "GGggggGG",
        ".GGGGGG.",
        "........",
    ],
    [
        (b's', 0xFF34_722E),
        (b'G', 0xFF52_A848),
        (b'g', 0xFF3C_8436),
        (b'#', 0xFF14_1E14),
    ]
);

const DOLPHIN: PixelMobIcon = icon!(
    8, 7,
    [
        ".dddddd.",
        "dBBBBBBd",
        "B#BBBB#B",
        "BBBBBBBB",
        "BBwwwwBB",
        "..wwww..",
        "........",
    ],
    [
        (b'd', 0xFF6E_8298),
        (b'B', 0xFF98_AEBE),
        (b'w', 0xFFD4_E0EA),
        (b'#', 0xFF14_1820),
    ]
);

const AXOLOTL: PixelMobIcon = icon!(
    10, 7,
    [
        "g.pppppp.g",
        "ggPPPPPPgg",
        ".gP#PP#Pg.",
        "ggPPPPPPgg",
        "g.PPppPP.g",
        "..pppppp..",
        "..........",
    ],
    [
        (b'p', 0xFFDC_7EB8),
        (b'P', 0xFFF6_A8D8),
        (b'g', 0xFFE6_4E94),
        (b'#', 0xFF1C_1420),
    ]
);

const FROG: PixelMobIcon = icon!(
    8, 7,
    [
        ".O#..#O.",
        ".OO..OO.",
        "OOOOOOOO",
        "OooooooO",
        "OyyyyyyO",
        ".yyyyyy.",
        "........",
    ],
    [
        (b'O', 0xFFD4_7438),
        (b'o', 0xFFB0_5A28),
        (b'y', 0xFFE8_BE82),
        (b'#', 0xFF1C_1410),
    ]
);

const NAUTILUS: PixelMobIcon = icon!(
    8, 8,
    [
        ".ssssss.",
        "sSwSSwSs",
        "sSSSSSSs",
        "sSwSSwSs",
        ".sSSSSs.",
        "..b#b#..",
        "..bbbb..",
        "........",
    ],
    [
        (b's', 0xFFC2_6E46),
        (b'S', 0xFFE8_9A70),
        (b'w', 0xFFF4_ECE4),
        (b'b', 0xFFD8_B49C),
        (b'#', 0xFF1C_1412),
    ]
);

// ============================================================================
// VILLAGERS & GOLEMS
// ============================================================================

const VILLAGER: PixelMobIcon = icon!(
    8, 10,
    [
        "ssssssss",
        "ssssssss",
        "sssssssd",
        "dssssssd",
        "dbbbbbbd",
        "sWEddEWs",
        "dsSnnSsd",
        "dmmnnmmd",
        "dSsnnsSd",
        "...nn...",
    ],
    [
        (b's', 0xFFBE_886C),
        (b'S', 0xFFB3_7B62),
        (b'd', 0xFF90_5E43),
        (b'b', 0xFF33_2411),
        (b'W', 0xFFFF_FFFF),
        (b'E', 0xFF00_9611),
        (b'n', 0xFFAD_745A),
        (b'm', 0xFF77_4235),
    ]
);

const WANDERING_TRADER: PixelMobIcon = icon!(
    8, 10,
    [
        "bbbbbbbb",
        "byyyyyyb",
        "bssssssb",
        "bssssssb",
        "bccccccb",
        "bWEddEWb",
        "bsSnnSsb",
        "bmmnnmmb",
        "bsSnnSsb",
        "...nn...",
    ],
    [
        (b'b', 0xFF28_589C),
        (b'y', 0xFFE8_C240),
        (b's', 0xFFBE_886C),
        (b'S', 0xFFB3_7B62),
        (b'd', 0xFF90_5E43),
        (b'c', 0xFF33_2411),
        (b'W', 0xFFFF_FFFF),
        (b'E', 0xFF00_9611),
        (b'n', 0xFFAD_745A),
        (b'm', 0xFF77_4235),
    ]
);

const IRON_GOLEM: PixelMobIcon = icon!(
    8, 9,
    [
        "iigiiiii",
        "iiiggiii",
        "iddddddi",
        "iR#ii#Ri",
        "iiinniii",
        "iiinniii",
        "iiinniii",
        "iiiiiiii",
        "dddddddd",
    ],
    [
        (b'i', 0xFFD6_D0C8),
        (b'd', 0xFFA4_9E96),
        (b'n', 0xFFBC_B6AE),
        (b'g', 0xFF56_8E44),
        (b'R', 0xFFD8_3424),
        (b'#', 0xFF24_1816),
    ]
);

const SNOW_GOLEM: PixelMobIcon = icon!(
    8, 8,
    [
        "ooOOOOoo",
        "oOOOOOOo",
        "O##OO##O",
        "O##OO##O",
        "OOOOOOOO",
        "O#O##O#O",
        "OO####OO",
        "ooOOOOoo",
    ],
    [
        (b'o', 0xFFB8_5E14),
        (b'O', 0xFFE4_821E),
        (b'#', 0xFF2A_1408),
    ]
);

const COPPER_GOLEM: PixelMobIcon = icon!(
    8, 9,
    [
        "...rr...",
        "cccccccc",
        "cCCCCCCc",
        "cE#CC#Ec",
        "cCCnnCCc",
        "cCCnnCCc",
        "cCCCCCCc",
        "cccccccc",
        "........",
    ],
    [
        (b'r', 0xFF56_B89C),
        (b'c', 0xFFA8_5838),
        (b'C', 0xFFD8_7C50),
        (b'n', 0xFFBC_6640),
        (b'E', 0xFF66_FFEE),
        (b'#', 0xFF20_2422),
    ]
);

// ============================================================================
// NETHER & END MOBS
// ============================================================================

const GHAST: PixelMobIcon = icon!(
    8, 9,
    [
        "WWWWWWWW",
        "WwwwwwwW",
        "W##WW##W",
        "WddWWddW",
        "WWW##WWW",
        "WwwwwwwW",
        "WWWWWWWW",
        ".W.WW.W.",
        ".W.WW.W.",
    ],
    [
        (b'W', 0xFFF2_F2F4),
        (b'w', 0xFFE0_E0E6),
        (b'd', 0xFF9E_A2AC),
        (b'#', 0xFF28_282E),
    ]
);

const BLAZE: PixelMobIcon = icon!(
    10, 8,
    [
        "..oooooo..",
        "y.oYYYYo.y",
        "y.YWWWWY.y",
        "y.Y#YY#Y.y",
        "y.YYYYYY.y",
        "y.oYooYo.y",
        "..oooooo..",
        "..........",
    ],
    [
        (b'o', 0xFFD8_7014),
        (b'Y', 0xFFF8_C628),
        (b'y', 0xFFFF_D844),
        (b'W', 0xFFFF_F6C8),
        (b'#', 0xFF3A_1A08),
    ]
);

const PIGLIN: PixelMobIcon = icon!(
    10, 8,
    [
        ".pppppppp.",
        "epPPPPPPpe",
        "epP#PP#Ppe",
        "epPppppPpe",
        ".pP#pp#Pp.",
        ".pwppppwp.",
        ".pPPPPPPp.",
        "..pppppp..",
    ],
    [
        (b'p', 0xFFC2_8474),
        (b'P', 0xFFE4_A896),
        (b'e', 0xFFB0_7262),
        (b'w', 0xFFF4_F2EC),
        (b'#', 0xFF22_1816),
    ]
);

const ZOMBIFIED_PIGLIN: PixelMobIcon = icon!(
    10, 8,
    [
        ".ppppgggg.",
        "epPPPggGge",
        "epP#Pgg#ge",
        "epPppggGge",
        ".pP#pp#Gg.",
        ".pwppppwg.",
        ".pPPPgggg.",
        "..pppggg..",
    ],
    [
        (b'p', 0xFFC2_8474),
        (b'P', 0xFFE4_A896),
        (b'e', 0xFFB0_7262),
        (b'g', 0xFF4C_7838),
        (b'G', 0xFF66_984C),
        (b'w', 0xFFF2_EFE6),
        (b'#', 0xFF20_1816),
    ]
);

const HOGLIN: PixelMobIcon = icon!(
    10, 8,
    [
        ".bbbbbbbb.",
        "ebBBBBBBbe",
        "ebB#BB#Bbe",
        "ebBBBBBBbe",
        ".wB#BB#Bw.",
        ".wwBBBBww.",
        ".wBBBBBBw.",
        "..bbbbbb..",
    ],
    [
        (b'b', 0xFFA2_5C4E),
        (b'B', 0xFFCC_7E6E),
        (b'e', 0xFF8C_4C40),
        (b'w', 0xFFF4_EFE2),
        (b'#', 0xFF20_1614),
    ]
);

const STRIDER: PixelMobIcon = icon!(
    10, 7,
    [
        ".rrrrrrrr.",
        "grRRRRRRrg",
        "grR#RR#Rrg",
        ".rRRRRRRr.",
        "grRrrrrRrg",
        ".rrrrrrrr.",
        "..........",
    ],
    [
        (b'r', 0xFF86_2C2C),
        (b'R', 0xFFB8_4040),
        (b'g', 0xFF9C_9896),
        (b'#', 0xFF1C_1414),
    ]
);

const WITHER: PixelMobIcon = icon!(
    8, 8,
    [
        "dddddddd",
        "dbbbbbbd",
        "dbbbbbbd",
        "dWWddWWd",
        "dbbbbbbd",
        "d#W##W#d",
        "dbbbbbbd",
        "dddddddd",
    ],
    [
        (b'd', 0xFF20_2024),
        (b'b', 0xFF36_363C),
        (b'W', 0xFFE8_ECFF),
        (b'#', 0xFF10_1014),
    ]
);

const ENDER_DRAGON: PixelMobIcon = icon!(
    8, 8,
    [
        "g......g",
        "gddddddg",
        "dddddddd",
        "pPddddPp",
        "dddddddd",
        "dggggggd",
        "dg#gg#gd",
        ".dddddd.",
    ],
    [
        (b'd', 0xFF1C_1822),
        (b'g', 0xFF34_2E3C),
        (b'p', 0xFFCC_38FF),
        (b'P', 0xFFF4_B2FF),
        (b'#', 0xFF0C_0A10),
    ]
);

/// Return a crisp hand-crafted pixel-art icon for the mob species, or `None` for species
/// that should keep their texture-composited vanilla icons (`squid`, `glow_squid`,
/// `skeleton`, `wither_skeleton`, `stray`, `bogged`, `parched`, `chicken`, `pig`, `player`).
pub fn pixel_mob_icon(species_id: &str) -> Option<PixelMobIcon> {
    let sp = normalize_species(species_id);
    match sp {
        // Keep texture-composited icons for these species as requested:
        "squid" | "glow_squid" | "skeleton" | "wither_skeleton" | "stray" | "bogged"
        | "parched" | "chicken" | "pig" | "player" => None,

        "creeper" => Some(CREEPER),
        "zombie" => Some(ZOMBIE),
        "husk" => Some(HUSK),
        "drowned" => Some(DROWNED),
        "zombie_villager" => Some(ZOMBIE_VILLAGER),
        "spider" => Some(SPIDER),
        "cave_spider" => Some(CAVE_SPIDER),
        "enderman" => Some(ENDERMAN),
        "slime" => Some(SLIME),
        "magma_cube" => Some(MAGMA_CUBE),
        "witch" => Some(WITCH),
        "pillager" | "vindicator" | "illusioner" => Some(PILLAGER),
        "evoker" => Some(EVOKER),
        "phantom" => Some(PHANTOM),
        "silverfish" => Some(SILVERFISH),
        "endermite" => Some(ENDERMITE),
        "guardian" => Some(GUARDIAN),
        "elder_guardian" => Some(ELDER_GUARDIAN),
        "shulker" => Some(SHULKER),
        "ravager" => Some(RAVAGER),
        "warden" => Some(WARDEN),
        "breeze" => Some(BREEZE),
        "creaking" => Some(CREAKING),
        "vex" => Some(VEX),

        "cow" => Some(COW),
        "mooshroom" => Some(MOOSHROOM),
        "sheep" => Some(SHEEP),
        "wolf" => Some(WOLF),
        "cat" | "ocelot" => Some(CAT),
        "fox" => Some(FOX),
        "bee" => Some(BEE),
        "horse" => Some(HORSE),
        "donkey" | "mule" => Some(DONKEY),
        "skeleton_horse" => Some(SKELETON_HORSE),
        "zombie_horse" => Some(ZOMBIE_HORSE),
        "llama" | "trader_llama" => Some(LLAMA),
        "camel" | "camel_husk" => Some(CAMEL),
        "goat" => Some(GOAT),
        "rabbit" => Some(RABBIT),
        "panda" => Some(PANDA),
        "polar_bear" => Some(POLAR_BEAR),
        "armadillo" => Some(ARMADILLO),
        "sniffer" => Some(SNIFFER),
        "bat" => Some(BAT),
        "parrot" => Some(PARROT),
        "allay" => Some(ALLAY),

        "salmon" => Some(SALMON),
        "cod" => Some(COD),
        "tropical_fish" => Some(TROPICAL_FISH),
        "pufferfish" => Some(PUFFERFISH),
        "tadpole" => Some(TADPOLE),
        "turtle" => Some(TURTLE),
        "dolphin" => Some(DOLPHIN),
        "axolotl" => Some(AXOLOTL),
        "frog" => Some(FROG),
        "nautilus" | "zombie_nautilus" => Some(NAUTILUS),

        "villager" => Some(VILLAGER),
        "wandering_trader" => Some(WANDERING_TRADER),
        "iron_golem" => Some(IRON_GOLEM),
        "snow_golem" => Some(SNOW_GOLEM),
        "copper_golem" => Some(COPPER_GOLEM),

        "ghast" | "happy_ghast" => Some(GHAST),
        "blaze" => Some(BLAZE),
        "piglin" | "piglin_brute" => Some(PIGLIN),
        "zombified_piglin" => Some(ZOMBIFIED_PIGLIN),
        "hoglin" | "zoglin" => Some(HOGLIN),
        "strider" => Some(STRIDER),
        "wither" => Some(WITHER),
        "ender_dragon" => Some(ENDER_DRAGON),

        _ => None,
    }
}
