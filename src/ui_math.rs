//! Zero-allocation polygon clipping, minimap corners, circular triangulation, and World Map UI geometry.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
}

/// Stack-allocated polygon clip result (never allocates on the heap).
#[derive(Clone, Debug)]
pub struct ClippedPoly {
    pub verts: [ClipVertex; 40],
    pub len: usize,
}

impl ClippedPoly {
    pub fn slice(&self) -> &[ClipVertex] {
        &self.verts[..self.len]
    }
}

/// Sutherland-Hodgman clip against a single half-plane defined by point `p0` and inward normal `(nx, ny)`.
#[inline]
fn clip_against_half_plane(
    input: &[ClipVertex],
    out: &mut [ClipVertex; 40],
    nx: f32,
    ny: f32,
    d: f32, // edge distance: inside iff nx*x + ny*y + d >= 0
) -> usize {
    let n = input.len();
    if n < 3 {
        return 0;
    }
    let mut out_len = 0usize;
    let mut prev = input[n - 1];
    let mut prev_dist = nx * prev.pos[0] + ny * prev.pos[1] + d;

    for &curr in input {
        let curr_dist = nx * curr.pos[0] + ny * curr.pos[1] + d;
        let curr_in = curr_dist >= -1e-4;
        let prev_in = prev_dist >= -1e-4;

        if curr_in != prev_in {
            let denom = prev_dist - curr_dist;
            let t = if denom.abs() > 1e-6 {
                (prev_dist / denom).clamp(0.0, 1.0)
            } else {
                0.5
            };
            if out_len < 40 {
                out[out_len] = ClipVertex {
                    pos: [
                        prev.pos[0] + (curr.pos[0] - prev.pos[0]) * t,
                        prev.pos[1] + (curr.pos[1] - prev.pos[1]) * t,
                    ],
                    uv: [
                        prev.uv[0] + (curr.uv[0] - prev.uv[0]) * t,
                        prev.uv[1] + (curr.uv[1] - prev.uv[1]) * t,
                    ],
                };
                out_len += 1;
            }
        }
        if curr_in && out_len < 40 {
            out[out_len] = curr;
            out_len += 1;
        }
        prev = curr;
        prev_dist = curr_dist;
    }
    out_len
}

/// Clip a quad `[TL, TR, BR, BL]` against an axis-aligned rectangle `[min_x, min_y, max_x, max_y]`.
pub fn clip_quad_rect(
    quad: [ClipVertex; 4],
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
) -> ClippedPoly {
    let mut buf_a = [ClipVertex { pos: [0.0; 2], uv: [0.0; 2] }; 40];
    let mut buf_b = [ClipVertex { pos: [0.0; 2], uv: [0.0; 2] }; 40];
    buf_a[..4].copy_from_slice(&quad);
    let mut len = 4;

    len = clip_against_half_plane(&buf_a[..len], &mut buf_b, 1.0, 0.0, -min_x);
    len = clip_against_half_plane(&buf_b[..len], &mut buf_a, -1.0, 0.0, max_x);
    len = clip_against_half_plane(&buf_a[..len], &mut buf_b, 0.0, 1.0, -min_y);
    len = clip_against_half_plane(&buf_b[..len], &mut buf_a, 0.0, -1.0, max_y);

    ClippedPoly { verts: buf_a, len }
}

/// Clip a quad against a regular 32-gon circle centered at `(cx, cy)` with radius `r`.
/// Zero heap allocations; smooth silhouette matching circular metallic frame cleanly.
pub fn clip_quad_circle(
    quad: [ClipVertex; 4],
    cx: f32,
    cy: f32,
    r: f32,
) -> ClippedPoly {
    const N: usize = 32;
    let mut buf_a = [ClipVertex { pos: [0.0; 2], uv: [0.0; 2] }; 40];
    let mut buf_b = [ClipVertex { pos: [0.0; 2], uv: [0.0; 2] }; 40];
    buf_a[..4].copy_from_slice(&quad);
    let mut len = 4;

    // Cosine offset keeps vertices inside `r` while 32-gon leaves zero visible chords.
    let apothem = r * (std::f32::consts::PI / (N as f32)).cos();
    for k in 0..N {
        if len < 3 {
            return ClippedPoly { verts: buf_a, len: 0 };
        }
        let theta = (k as f32) * std::f32::consts::TAU / (N as f32);
        let nx = -theta.cos();
        let ny = -theta.sin();
        let d = apothem - nx * cx - ny * cy;
        len = clip_against_half_plane(&buf_a[..len], &mut buf_b, nx, ny, d);
        buf_a[..len].copy_from_slice(&buf_b[..len]);
    }
    ClippedPoly { verts: buf_a, len }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MinimapShape {
    Square = 0,
    Round = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MinimapCorner {
    TopLeft = 0,
    TopRight = 1,
    BottomLeft = 2,
    BottomRight = 3,
}

impl MinimapCorner {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::TopRight,
            2 => Self::BottomLeft,
            3 => Self::BottomRight,
            _ => Self::TopLeft,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MinimapRect {
    pub outer_x: f32,
    pub outer_y: f32,
    pub outer_size: f32,
    pub inner_x: f32,
    pub inner_y: f32,
    pub inner_size: f32,
    pub center_x: f32,
    pub center_y: f32,
    pub coord_y: f32,
}

pub const FRAME_BORDER: f32 = 4.0;
pub const HUD_MARGIN: f32 = 5.0;

pub fn minimap_rect(
    corner: MinimapCorner,
    outer_size: f32,
    screen_w: f32,
    screen_h: f32,
) -> MinimapRect {
    let clamped = outer_size.clamp(64.0, (screen_h - 32.0).max(64.0));
    let x = match corner {
        MinimapCorner::TopLeft | MinimapCorner::BottomLeft => HUD_MARGIN,
        MinimapCorner::TopRight | MinimapCorner::BottomRight => {
            (screen_w - clamped - HUD_MARGIN).max(HUD_MARGIN)
        }
    };
    let y = match corner {
        MinimapCorner::TopLeft | MinimapCorner::TopRight => HUD_MARGIN,
        MinimapCorner::BottomLeft | MinimapCorner::BottomRight => {
            (screen_h - clamped - HUD_MARGIN - 12.0).max(HUD_MARGIN)
        }
    };
    let inner_size = (clamped - FRAME_BORDER * 2.0).max(16.0);
    MinimapRect {
        outer_x: x,
        outer_y: y,
        outer_size: clamped,
        inner_x: x + FRAME_BORDER,
        inner_y: y + FRAME_BORDER,
        inner_size,
        center_x: x + clamped * 0.5,
        center_y: y + clamped * 0.5,
        coord_y: y + clamped + 2.0,
    }
}

/// Project world `(wx, wz)` into `(sx, sy)` relative to `(player_x, player_z)` on minimap.
/// Follows Horizon/Minecraft yaw (`0 = South (+Z), 90 = West (-X), 180 = North (-Z), -90 = East (+X)`).
#[inline]
pub fn world_to_minimap(
    wx: f64,
    wz: f64,
    player_x: f64,
    player_z: f64,
    yaw_deg: f32,
    rotate_with_player: bool,
    pixels_per_block: f32,
    center_x: f32,
    center_y: f32,
) -> [f32; 2] {
    let dx = (wx - player_x) as f32;
    let dz = (wz - player_z) as f32;
    let (fx, fz) = if rotate_with_player {
        let r = yaw_deg.to_radians();
        (-r.sin(), r.cos())
    } else {
        (0.0, -1.0) // North-Up
    };
    // right = (-fz, fx), up = (fx, fz), so screen right = dot(d, right), screen down = -dot(d, fwd)
    let sx = (dx * (-fz) + dz * fx) * pixels_per_block;
    let sy = -(dx * fx + dz * fz) * pixels_per_block;
    [center_x + sx, center_y + sy]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl UiRect {
    #[inline]
    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapButtonId {
    Settings,
    QuickWaypoint,
    PreloadRegion,
    WaypointsList,
    CaveMode,
    HelpMenu,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    ExitMap,
    ChunkGrid,
    CenterPlayer,
    ToggleWaypoints,
    ToggleEntities,
}

#[derive(Clone, Debug)]
pub struct WorldMapLayout {
    pub screen_w: f32,
    pub screen_h: f32,
    pub buttons: Vec<(MapButtonId, UiRect)>,
    pub compass_rect: UiRect,
    pub cave_slider_track: UiRect,
    pub modal_box: UiRect,
}

impl WorldMapLayout {
    pub fn compute(screen_w: f32, screen_h: f32) -> Self {
        let mut buttons = Vec::with_capacity(14);
        // Top-left gear
        buttons.push((
            MapButtonId::Settings,
            UiRect { x: 8.0, y: 8.0, w: 22.0, h: 22.0 },
        ));
        // Top-right compass indicator
        let compass_rect = UiRect {
            x: (screen_w - 42.0).max(40.0),
            y: 6.0,
            w: 36.0,
            h: 36.0,
        };

        // Left bottom toolbar matching Screenshot 2 proportions.
        let left_ids = [
            MapButtonId::QuickWaypoint,
            MapButtonId::PreloadRegion,
            MapButtonId::WaypointsList,
            MapButtonId::CaveMode,
            MapButtonId::HelpMenu,
        ];
        let btn_size = 20.0;
        let gap = 4.0;
        let left_total_h = (left_ids.len() as f32) * btn_size + ((left_ids.len() - 1) as f32) * gap;
        let left_y0 = (screen_h - left_total_h - 8.0).max(54.0);
        for (i, &id) in left_ids.iter().enumerate() {
            buttons.push((
                id,
                UiRect {
                    x: 8.0,
                    y: left_y0 + (i as f32) * (btn_size + gap),
                    w: btn_size,
                    h: btn_size,
                },
            ));
        }

        // Cave control popup bar (mode tabs Auto / Manual / Off + Y slider) next to CaveMode button.
        let cave_y = left_y0 + 3.0 * (btn_size + gap);
        let cave_slider_track = UiRect {
            x: 34.0,
            y: (cave_y - 16.0).max(40.0),
            w: 152.0,
            h: 38.0,
        };

        // Right vertical toolbar matching Screenshot 2 right side.
        let right_ids = [
            MapButtonId::ZoomIn,
            MapButtonId::ZoomOut,
            MapButtonId::ResetZoom,
            MapButtonId::ExitMap,
            MapButtonId::ChunkGrid,
            MapButtonId::CenterPlayer,
            MapButtonId::ToggleWaypoints,
            MapButtonId::ToggleEntities,
        ];
        let right_x = (screen_w - 28.0).max(40.0);
        let right_total_h = (right_ids.len() as f32) * btn_size + ((right_ids.len() - 1) as f32) * gap;
        let right_y0 = ((screen_h - right_total_h) * 0.56).max(60.0);
        for (i, &id) in right_ids.iter().enumerate() {
            buttons.push((
                id,
                UiRect {
                    x: right_x,
                    y: right_y0 + (i as f32) * (btn_size + gap),
                    w: btn_size,
                    h: btn_size,
                },
            ));
        }

        let mw = 232.0f32.min(screen_w - 24.0);
        let mh = 186.0f32.min(screen_h - 24.0);
        let modal_box = UiRect {
            x: (screen_w - mw) * 0.5,
            y: (screen_h - mh) * 0.5,
            w: mw,
            h: mh,
        };

        Self {
            screen_w,
            screen_h,
            buttons,
            compass_rect,
            cave_slider_track,
            modal_box,
        }
    }

    pub fn button_rect(&self, target: MapButtonId) -> Option<UiRect> {
        self.buttons
            .iter()
            .find(|(id, _)| *id == target)
            .map(|(_, r)| *r)
    }

    pub fn hit_button(&self, px: f32, py: f32) -> Option<MapButtonId> {
        self.buttons
            .iter()
            .find(|(_, r)| r.contains(px, py))
            .map(|(id, _)| *id)
    }
}
