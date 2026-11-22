//! Waypoint data model, color palette, minimap rim clamping, 3D projection, and binary codec.

#[derive(Clone, Debug, PartialEq)]
pub struct Waypoint {
    pub id: u64,
    pub name: String,
    pub initial: String,
    pub color: u32, // 0xAARRGGBB
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub dim: String,
    pub enabled: bool,
    pub is_death: bool,
}

/// Xaero-inspired high-contrast 12-color swatch palette (0xAARRGGBB).
pub const PALETTE: [u32; 12] = [
    0xFF141414, // Charcoal / Dark (Default [D])
    0xFFD926C7, // Magenta / Pink ([Ф])
    0xFFE53935, // Red
    0xFFFB8C00, // Orange
    0xFFFDD835, // Yellow
    0xFF7CB342, // Lime
    0xFF2E7D32, // Dark Green
    0xFF00ACC1, // Cyan
    0xFF1E88E5, // Blue
    0xFF8E24AA, // Purple
    0xFFF5F5F5, // White
    0xFF6D4C41, // Brown
];

pub fn palette_index(color: u32) -> usize {
    PALETTE
        .iter()
        .position(|&c| (c & 0x00FF_FFFF) == (color & 0x00FF_FFFF))
        .unwrap_or(0)
}

/// Derive a clean 1-character uppercase glyph badge from waypoint name (Cyrillic & Latin friendly).
pub fn derive_initial(name: &str) -> String {
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() {
            return ch.to_uppercase().collect();
        }
    }
    "W".to_owned()
}

/// Choose crisp text foreground color depending on fill luminance.
pub fn contrast_fg(bg_argb: u32) -> u32 {
    let [_, r, g, b] = bg_argb.to_be_bytes();
    let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if luma > 185.0 { 0xFF121212 } else { 0xFFFFFFFF }
}

/// Clamp local screen offset `(dx, dy)` onto or inside an axis-aligned square `[-half, half]`.
/// Returns `(clamped_dx, clamped_dy, was_clamped)`.
pub fn clamp_to_square(dx: f32, dy: f32, half: f32) -> (f32, f32, bool) {
    let max_abs = dx.abs().max(dy.abs());
    if max_abs <= half || max_abs < 1e-5 {
        (dx, dy, false)
    } else {
        let s = half / max_abs;
        (dx * s, dy * s, true)
    }
}

/// Clamp local screen offset `(dx, dy)` onto or inside a circle of `radius`.
/// Returns `(clamped_dx, clamped_dy, was_clamped)`.
pub fn clamp_to_circle(dx: f32, dy: f32, radius: f32) -> (f32, f32, bool) {
    let dist_sq = dx * dx + dy * dy;
    let r_sq = radius * radius;
    if dist_sq <= r_sq || dist_sq < 1e-8 {
        (dx, dy, false)
    } else {
        let s = radius / dist_sq.sqrt();
        (dx * s, dy * s, true)
    }
}

/// Build orthonormal camera `(fwd, right, up)` vectors matching Horizon's YXZ camera convention.
/// `yaw_deg`: 0 = South (+Z), 90 = West (-X), 180 = North (-Z), -90 = East (+X).
/// `pitch_deg`: +90 = Down (-Y), -90 = Up (+Y).
pub fn camera_basis(yaw_deg: f32, pitch_deg: f32) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let yaw = f64::from(yaw_deg).to_radians();
    let pitch = f64::from(pitch_deg).to_radians();
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    // Forward direction.
    let hx = -sy;
    let hz = cy;
    let fwd = [hx * cp, -sp, hz * cp];
    // Right direction in horizontal XZ plane: cross((hx, 0, hz), (0, 1, 0)) = (-hz, 0, hx) = (-cy, 0, -sy).
    let right = [-cy, 0.0, -sy];
    // Up = cross(right, fwd) ensures right-handed camera screen coordinates (+right, +up, +fwd).
    let up = [
        right[1] * fwd[2] - right[2] * fwd[1],
        right[2] * fwd[0] - right[0] * fwd[2],
        right[0] * fwd[1] - right[1] * fwd[0],
    ];
    (fwd, right, up)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectedPoint {
    pub x: f32,
    pub y: f32,
    pub distance: f32,
}

/// Project a 3D world point onto 2D GUI screen pixels `[0..screen_w, 0..screen_h]`.
/// Returns `None` if behind the camera near plane or viewport is invalid.
pub fn project_world_point(
    target: [f64; 3],
    cam_pos: [f64; 3],
    yaw_deg: f32,
    pitch_deg: f32,
    fov_deg: f32,
    screen_w: f32,
    screen_h: f32,
) -> Option<ProjectedPoint> {
    if screen_w <= 1.0 || screen_h <= 1.0 {
        return None;
    }
    let d = [
        target[0] - cam_pos[0],
        target[1] - cam_pos[1],
        target[2] - cam_pos[2],
    ];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
    let (fwd, right, up) = camera_basis(yaw_deg, pitch_deg);
    let z_cam = d[0] * fwd[0] + d[1] * fwd[1] + d[2] * fwd[2];
    if z_cam <= 0.15 {
        return None;
    }
    let x_cam = d[0] * right[0] + d[1] * right[1] + d[2] * right[2];
    let y_cam = d[0] * up[0] + d[1] * up[1] + d[2] * up[2];
    let half_tan = (f64::from(fov_deg).clamp(20.0, 150.0).to_radians() * 0.5).tan();
    let aspect = f64::from(screen_w) / f64::from(screen_h);
    let ndc_x = x_cam / (z_cam * half_tan * aspect);
    let ndc_y = y_cam / (z_cam * half_tan);
    if !ndc_x.is_finite() || !ndc_y.is_finite() {
        return None;
    }
    let sx = ((ndc_x * 0.5 + 0.5) * f64::from(screen_w)) as f32;
    let sy = ((0.5 - ndc_y * 0.5) * f64::from(screen_h)) as f32;
    if sx < -80.0 || sx > screen_w + 80.0 || sy < -80.0 || sy > screen_h + 80.0 {
        return None;
    }
    Some(ProjectedPoint {
        x: sx,
        y: sy,
        distance: dist,
    })
}

const WAYPOINTS_MAGIC: &[u8; 8] = b"ATLSWPT1";

pub fn encode_waypoints(list: &[Waypoint], next_id: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + list.len() * 48);
    out.extend_from_slice(WAYPOINTS_MAGIC);
    out.extend_from_slice(&next_id.to_le_bytes());
    out.extend_from_slice(&(list.len() as u32).to_le_bytes());
    for wp in list {
        out.extend_from_slice(&wp.id.to_le_bytes());
        write_str(&mut out, &wp.name);
        write_str(&mut out, &wp.initial);
        out.extend_from_slice(&wp.color.to_le_bytes());
        out.extend_from_slice(&wp.x.to_le_bytes());
        out.extend_from_slice(&wp.y.to_le_bytes());
        out.extend_from_slice(&wp.z.to_le_bytes());
        write_str(&mut out, &wp.dim);
        let flags = u8::from(wp.enabled) | (u8::from(wp.is_death) << 1);
        out.push(flags);
    }
    out
}

pub fn decode_waypoints(data: &[u8]) -> Option<(Vec<Waypoint>, u64)> {
    if data.len() < 20 || &data[0..8] != WAYPOINTS_MAGIC {
        return None;
    }
    let mut cur = &data[8..];
    let next_id = read_u64(&mut cur)?;
    let count = read_u32(&mut cur)? as usize;
    if count > 50_000 {
        return None;
    }
    let mut list = Vec::with_capacity(count);
    for _ in 0..count {
        let id = read_u64(&mut cur)?;
        let name = read_str(&mut cur)?;
        let initial = read_str(&mut cur)?;
        let color = read_u32(&mut cur)?;
        let x = read_i32(&mut cur)?;
        let y = read_i32(&mut cur)?;
        let z = read_i32(&mut cur)?;
        let dim = read_str(&mut cur)?;
        let flags = read_u8(&mut cur)?;
        list.push(Waypoint {
            id,
            name,
            initial,
            color,
            x,
            y,
            z,
            dim,
            enabled: (flags & 1) != 0,
            is_death: (flags & 2) != 0,
        });
    }
    Some((list, next_id))
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&bytes[..len as usize]);
}

fn read_u8(cur: &mut &[u8]) -> Option<u8> {
    let (&b, rest) = cur.split_first()?;
    *cur = rest;
    Some(b)
}

fn read_u16(cur: &mut &[u8]) -> Option<u16> {
    if cur.len() < 2 {
        return None;
    }
    let (head, rest) = cur.split_at(2);
    *cur = rest;
    Some(u16::from_le_bytes(head.try_into().ok()?))
}

fn read_u32(cur: &mut &[u8]) -> Option<u32> {
    if cur.len() < 4 {
        return None;
    }
    let (head, rest) = cur.split_at(4);
    *cur = rest;
    Some(u32::from_le_bytes(head.try_into().ok()?))
}

fn read_i32(cur: &mut &[u8]) -> Option<i32> {
    if cur.len() < 4 {
        return None;
    }
    let (head, rest) = cur.split_at(4);
    *cur = rest;
    Some(i32::from_le_bytes(head.try_into().ok()?))
}

fn read_u64(cur: &mut &[u8]) -> Option<u64> {
    if cur.len() < 8 {
        return None;
    }
    let (head, rest) = cur.split_at(8);
    *cur = rest;
    Some(u64::from_le_bytes(head.try_into().ok()?))
}

fn read_str(cur: &mut &[u8]) -> Option<String> {
    let len = read_u16(cur)? as usize;
    if cur.len() < len {
        return None;
    }
    let (head, rest) = cur.split_at(len);
    *cur = rest;
    String::from_utf8(head.to_vec()).ok()
}
