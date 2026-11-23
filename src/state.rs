//! Shared mod configuration, World Map UI state machine, input queue, and persistence coordination.

use crate::atlas_cache::TerrainStore;
use crate::entities::RadarEntity;
use crate::terrain::CaveMode;
use crate::ui_math::{MapButtonId, MinimapCorner, MinimapShape, UiRect, WorldMapLayout};
use crate::waypoints::{PALETTE, Waypoint, derive_initial, palette_index};
use std::collections::{HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};

pub const MIN_ZOOM: f32 = 0.0625;
pub const MAX_ZOOM: f32 = 16.0;
pub const MIN_MINIMAP_ZOOM: f32 = 0.35;
pub const MAX_MINIMAP_ZOOM: f32 = 4.0;
pub const DEFAULT_MINIMAP_ZOOM: f32 = 0.58;
pub const MIN_MINIMAP_SIZE: f32 = 64.0;
pub const MAX_MINIMAP_SIZE: f32 = 256.0;

/// Compute the persistent storage key for the active world.
///
/// Integrated single-player worlds must take precedence over `remote_address` because
/// Horizon connects local single-player clients over an in-memory session (`address == "local"`).
pub fn resolve_world_key(
    integrated_save_folder: Option<&str>,
    remote_address: Option<&str>,
) -> String {
    if let Some(folder) = integrated_save_folder.filter(|s| !s.is_empty()) {
        format!("sp_{folder}")
    } else if let Some(addr) = remote_address.filter(|s| !s.is_empty()) {
        format!("srv_{addr}")
    } else {
        "local".to_owned()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModConfig {
    pub enabled: bool,
    pub minimap_size: f32,
    pub shape: MinimapShape,
    pub corner: MinimapCorner,
    pub minimap_zoom: f32,
    pub rotate_with_player: bool,
    pub cave_mode: CaveMode,
    pub cave_y: i32,
    pub show_entities: bool,
    pub show_waypoints_3d: bool,
    pub auto_death_waypoint: bool,
    pub chunk_grid: bool,
    pub realistic_colors: bool,
    pub show_waypoints_on_map: bool,
    pub show_entities_on_map: bool,
    pub russian_lang: bool,
}

impl Default for ModConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            // Screenshot 3 square proportion (~104 GUI px at standard HUD scale)
            minimap_size: 104.0,
            shape: MinimapShape::Square,
            corner: MinimapCorner::TopLeft,
            minimap_zoom: DEFAULT_MINIMAP_ZOOM,
            rotate_with_player: true,
            cave_mode: CaveMode::Auto,
            cave_y: 40,
            show_entities: true,
            show_waypoints_3d: true,
            auto_death_waypoint: true,
            chunk_grid: false,
            realistic_colors: true,
            show_waypoints_on_map: true,
            show_entities_on_map: true,
            russian_lang: false,
        }
    }
}

impl ModConfig {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        out.extend_from_slice(b"ETWMCFG1");
        let flags: u16 = u16::from(self.enabled)
            | (u16::from(self.rotate_with_player) << 1)
            | (u16::from(self.show_entities) << 2)
            | (u16::from(self.show_waypoints_3d) << 3)
            | (u16::from(self.auto_death_waypoint) << 4)
            | (u16::from(self.chunk_grid) << 5)
            | (u16::from(self.realistic_colors) << 6)
            | (u16::from(self.show_waypoints_on_map) << 7)
            | (u16::from(self.show_entities_on_map) << 8)
            | ((self.shape as u16) << 9);
        out.extend_from_slice(&flags.to_le_bytes());
        out.push(self.corner as u8);
        out.push(self.cave_mode as u8);
        out.extend_from_slice(&self.minimap_size.to_le_bytes());
        out.extend_from_slice(&self.minimap_zoom.to_le_bytes());
        out.extend_from_slice(&self.cave_y.to_le_bytes());
        out
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 24 {
            return None;
        }
        let magic = &data[0..8];
        let is_v1 = magic == b"ATLSCFG1";
        let is_v2 = magic == b"ATLSCFG2" || magic == b"ETWMCFG1";
        if !is_v1 && !is_v2 {
            return None;
        }
        let flags = u16::from_le_bytes([data[8], data[9]]);
        let corner = MinimapCorner::from_u8(data[10]);
        let cave_mode = CaveMode::from_u8(data[11]);
        let minimap_size = f32::from_le_bytes(data[12..16].try_into().ok()?)
            .clamp(MIN_MINIMAP_SIZE, MAX_MINIMAP_SIZE);
        let raw_zoom = f32::from_le_bytes(data[16..20].try_into().ok()?)
            .clamp(MIN_MINIMAP_ZOOM, MAX_MINIMAP_ZOOM);
        let minimap_zoom = if is_v1 && (raw_zoom - 1.0).abs() < 0.01 {
            DEFAULT_MINIMAP_ZOOM
        } else {
            raw_zoom
        };
        let cave_y = i32::from_le_bytes(data[20..24].try_into().ok()?).clamp(-64, 320);
        Some(Self {
            enabled: (flags & (1 << 0)) != 0,
            rotate_with_player: (flags & (1 << 1)) != 0,
            show_entities: (flags & (1 << 2)) != 0,
            show_waypoints_3d: (flags & (1 << 3)) != 0,
            auto_death_waypoint: (flags & (1 << 4)) != 0,
            chunk_grid: (flags & (1 << 5)) != 0,
            realistic_colors: (flags & (1 << 6)) != 0,
            show_waypoints_on_map: if is_v1 { true } else { (flags & (1 << 7)) != 0 },
            show_entities_on_map: (flags & (1 << 8)) != 0,
            shape: if (flags & (1 << 9)) != 0 {
                MinimapShape::Round
            } else {
                MinimapShape::Square
            },
            corner,
            cave_mode,
            minimap_size,
            minimap_zoom,
            cave_y,
            russian_lang: false,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivePanel {
    None,
    WaypointModal,
    ContextMenu,
    Settings,
    WaypointsList,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalField {
    Name,
    Initial,
    Y,
}

#[derive(Clone, Debug)]
pub struct WaypointModalState {
    pub editing_id: Option<u64>,
    pub name: String,
    pub initial: String,
    pub initial_manual: bool,
    pub x: i32,
    pub y: i32,
    pub y_text: String,
    pub z: i32,
    pub dim: String,
    pub color_idx: usize,
    pub active_field: ModalField,
}

impl Default for WaypointModalState {
    fn default() -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            initial: "W".to_owned(),
            initial_manual: false,
            x: 0,
            y: 64,
            y_text: "64".to_owned(),
            z: 0,
            dim: "minecraft:overworld".to_owned(),
            color_idx: 1,
            active_field: ModalField::Name,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContextMenuState {
    pub screen_x: f32,
    pub screen_y: f32,
    pub world_x: i32,
    pub world_y: i32,
    pub world_z: i32,
    pub near_waypoint_id: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct PreloadState {
    pub active: bool,
    pub completed: u32,
    pub total: u32,
    pub center_x: i32,
    pub center_z: i32,
}

#[derive(Clone, Debug)]
pub enum UiEvent {
    MouseDown { x: f32, y: f32, button: i32 },
    MouseUp { x: f32, y: f32, button: i32 },
    MouseDrag { x: f32, y: f32, dx: f32, dy: f32, button: i32 },
    MouseScroll { x: f32, y: f32, delta_y: f32 },
    KeyDown { key: String, shift: bool, ctrl: bool },
    CharTyped(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModAction {
    ToggleWorldMap,
    OpenNewWaypoint,
    ZoomIn,
    ZoomOut,
    ToggleMinimap,
    ToggleCaveMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PendingCommand {
    RequestPreload { center_x: i32, center_z: i32, radius_chunks: i32 },
    Teleport { x: i32, y: i32, z: i32 },
    CopyClipboard(String),
    ForceRescanLoaded,
}

#[derive(Clone, Debug)]
pub struct WorldMapUi {
    pub open: bool,
    pub want_open: bool,
    pub want_close: bool,
    pub center_x: f64,
    pub center_z: f64,
    pub zoom: f32,
    pub follow_player: bool,
    pub dragging_pan: bool,
    pub drag_distance: f32,
    pub drag_button: i32,
    pub cave_slider_open: bool,
    pub dragging_cave_slider: bool,
    pub dragging_size_slider: bool,
    pub active_panel: ActivePanel,
    pub modal: WaypointModalState,
    pub context_menu: Option<ContextMenuState>,
    pub waypoints_scroll: usize,
    pub hovered_block: [i32; 2],
    pub hovered_y: Option<i16>,
    pub hovered_biome_id: String,
    pub hovered_biome_name: String,
    pub hovered_button: Option<MapButtonId>,
    pub preload: PreloadState,
    pub toast: Option<(String, u64)>,
}

impl Default for WorldMapUi {
    fn default() -> Self {
        Self {
            open: false,
            want_open: false,
            want_close: false,
            center_x: 0.0,
            center_z: 0.0,
            zoom: 1.0,
            follow_player: true,
            dragging_pan: false,
            drag_distance: 0.0,
            drag_button: 0,
            cave_slider_open: false,
            dragging_cave_slider: false,
            dragging_size_slider: false,
            active_panel: ActivePanel::None,
            modal: WaypointModalState::default(),
            context_menu: None,
            waypoints_scroll: 0,
            hovered_block: [0, 0],
            hovered_y: None,
            hovered_biome_id: String::new(),
            hovered_biome_name: String::new(),
            hovered_button: None,
            preload: PreloadState::default(),
            toast: None,
        }
    }
}

pub struct PlayerSnapshot {
    pub valid: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub eye_height: f64,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub fov_deg: f32,
    pub screen_w: f32,
    pub screen_h: f32,
    pub under_roof: bool,
    pub tab_held: bool,
    pub dead: bool,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            valid: false,
            x: 0.0,
            y: 64.0,
            z: 0.0,
            eye_height: 1.62,
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            fov_deg: 70.0,
            screen_w: 854.0,
            screen_h: 480.0,
            under_roof: false,
            tab_held: false,
            dead: false,
        }
    }
}

pub struct EternityMapState {
    pub config: ModConfig,
    pub config_dirty: bool,
    pub config_loaded: bool,
    pub terrain: TerrainStore,
    pub waypoints: Vec<Waypoint>,
    pub next_waypoint_id: u64,
    pub waypoints_loaded_world: String,
    pub waypoints_dirty: bool,
    pub biomes_loaded_world: String,
    pub biomes_dirty: bool,
    pub loaded_regions: HashSet<(i32, i32)>,
    pub loaded_regions_key: (String, String),
    pub pending_disk_writes: Vec<(String, Vec<u8>)>,
    pub player: PlayerSnapshot,
    pub entities: Vec<RadarEntity>,
    pub map: WorldMapUi,
    pub ui_events: VecDeque<UiEvent>,
    pub actions: VecDeque<ModAction>,
    pub commands: VecDeque<PendingCommand>,
    pub world_map_key: String,
    pub was_dead: bool,
    pub auto_cave_active: bool,
    pub auto_cave_y: i32,
    pub open_sky_frames: u8,
    pub tick_counter: u64,
    pub cursor_x: f32,
    pub cursor_y: f32,
}

impl EternityMapState {
    pub fn new() -> Self {
        Self {
            config: ModConfig::default(),
            config_dirty: false,
            config_loaded: false,
            terrain: TerrainStore::new(),
            waypoints: Vec::new(),
            next_waypoint_id: 1,
            waypoints_loaded_world: String::new(),
            waypoints_dirty: false,
            biomes_loaded_world: String::new(),
            biomes_dirty: false,
            loaded_regions: HashSet::new(),
            loaded_regions_key: (String::new(), String::new()),
            pending_disk_writes: Vec::new(),
            player: PlayerSnapshot::default(),
            entities: Vec::new(),
            map: WorldMapUi::default(),
            ui_events: VecDeque::new(),
            actions: VecDeque::new(),
            commands: VecDeque::new(),
            world_map_key: "key.keyboard.m".to_owned(),
            was_dead: false,
            auto_cave_active: false,
            auto_cave_y: 40,
            open_sky_frames: 0,
            tick_counter: 0,
            cursor_x: 0.0,
            cursor_y: 0.0,
        }
    }

    pub fn open_world_map(&mut self) {
        self.map.open = true;
        self.map.want_open = true;
        self.map.want_close = false;
        self.map.active_panel = ActivePanel::None;
        self.map.context_menu = None;
        self.map.follow_player = true;
        if self.player.valid {
            self.map.center_x = self.player.x;
            self.map.center_z = self.player.z;
        }
    }

    pub fn close_world_map(&mut self) {
        self.map.open = false;
        self.map.want_close = true;
        self.map.want_open = false;
        self.map.active_panel = ActivePanel::None;
        self.map.context_menu = None;
        self.map.dragging_pan = false;
        self.map.dragging_cave_slider = false;
        self.map.dragging_size_slider = false;
    }

    pub fn update_auto_cave_state(&mut self, roof_detected: bool, player_y: f64) {
        self.player.under_roof = roof_detected;
        if roof_detected {
            self.open_sky_frames = 0;
            let target_y = (player_y.floor() as i32 + 5).clamp(-64, 320);
            if !self.auto_cave_active {
                self.auto_cave_active = true;
                self.auto_cave_y = target_y;
            } else if (target_y - self.auto_cave_y).abs() >= 3 {
                self.auto_cave_y = target_y;
            }
        } else {
            self.open_sky_frames = self.open_sky_frames.saturating_add(1);
            if self.open_sky_frames >= 8 {
                self.auto_cave_active = false;
            }
        }
    }

    pub fn effective_cave_mode(&self) -> (bool, i32) {
        match self.config.cave_mode {
            CaveMode::Off => (false, self.config.cave_y),
            CaveMode::Manual => (true, self.config.cave_y),
            CaveMode::Auto => {
                if self.auto_cave_active {
                    (true, self.auto_cave_y)
                } else {
                    (false, self.config.cave_y)
                }
            }
        }
    }

    pub fn add_or_update_waypoint(&mut self, wp: Waypoint) {
        if let Some(existing) = self.waypoints.iter_mut().find(|w| w.id == wp.id) {
            *existing = wp;
        } else {
            self.waypoints.push(wp);
        }
        self.waypoints_dirty = true;
    }

    pub fn create_waypoint_modal_at(&mut self, x: i32, y: i32, z: i32, name_hint: &str) {
        let color_idx = (self.next_waypoint_id as usize) % (PALETTE.len() - 1) + 1;
        let initial = if name_hint.is_empty() {
            "W".to_owned()
        } else {
            derive_initial(name_hint)
        };
        self.map.modal = WaypointModalState {
            editing_id: None,
            name: name_hint.to_owned(),
            initial,
            initial_manual: false,
            x,
            y,
            y_text: y.to_string(),
            z,
            dim: self.terrain.dimension.clone(),
            color_idx,
            active_field: ModalField::Name,
        };
        self.map.active_panel = ActivePanel::WaypointModal;
        self.map.context_menu = None;
    }

    pub fn open_edit_waypoint_modal(&mut self, id: u64) {
        let Some(wp) = self.waypoints.iter().find(|w| w.id == id).cloned() else {
            return;
        };
        self.map.modal = WaypointModalState {
            editing_id: Some(wp.id),
            name: wp.name,
            initial: wp.initial,
            initial_manual: true,
            x: wp.x,
            y: wp.y,
            y_text: wp.y.to_string(),
            z: wp.z,
            dim: wp.dim,
            color_idx: palette_index(wp.color),
            active_field: ModalField::Name,
        };
        self.map.active_panel = ActivePanel::WaypointModal;
        self.map.context_menu = None;
    }

    pub fn commit_waypoint_modal(&mut self) {
        let m = self.map.modal.clone();
        let name = if m.name.trim().is_empty() {
            format!("{} {}", m.x, m.z)
        } else {
            m.name.trim().to_owned()
        };
        let initial = if m.initial.trim().is_empty() {
            derive_initial(&name)
        } else {
            m.initial.trim().chars().take(2).collect::<String>().to_uppercase()
        };
        let parsed_y = m.y_text.trim().parse::<i32>().unwrap_or(m.y).clamp(-64, 320);
        let id = m.editing_id.unwrap_or_else(|| {
            let id = self.next_waypoint_id;
            self.next_waypoint_id += 1;
            id
        });
        let color = PALETTE[m.color_idx % PALETTE.len()];
        self.add_or_update_waypoint(Waypoint {
            id,
            name,
            initial,
            color,
            x: m.x,
            y: parsed_y,
            z: m.z,
            dim: m.dim,
            enabled: true,
            is_death: false,
        });
        self.map.active_panel = ActivePanel::None;
    }

    pub fn delete_waypoint(&mut self, id: u64) {
        let before = self.waypoints.len();
        self.waypoints.retain(|w| w.id != id);
        if self.waypoints.len() != before {
            self.waypoints_dirty = true;
        }
    }

    pub fn add_death_waypoint(&mut self, x: i32, y: i32, z: i32) {
        if !self.config.auto_death_waypoint {
            return;
        }
        let id = self.next_waypoint_id;
        self.next_waypoint_id += 1;
        let name = if self.config.russian_lang {
            format!("Смерть ({x}, {y}, {z})")
        } else {
            format!("Death ({x}, {y}, {z})")
        };
        self.add_or_update_waypoint(Waypoint {
            id,
            name,
            initial: "D".to_owned(),
            color: PALETTE[0],
            x,
            y,
            z,
            dim: self.terrain.dimension.clone(),
            enabled: true,
            is_death: true,
        });
    }

    pub fn screen_to_world(&self, sx: f32, sy: f32, screen_w: f32, screen_h: f32) -> (f64, f64) {
        let z = f64::from(self.map.zoom.max(MIN_ZOOM));
        let wx = self.map.center_x + f64::from(sx - screen_w * 0.5) / z;
        let wz = self.map.center_z + f64::from(sy - screen_h * 0.5) / z;
        (wx, wz)
    }

    pub fn world_to_screen(&self, wx: f64, wz: f64, screen_w: f32, screen_h: f32) -> [f32; 2] {
        let z = f64::from(self.map.zoom.max(MIN_ZOOM));
        [
            screen_w * 0.5 + ((wx - self.map.center_x) * z) as f32,
            screen_h * 0.5 + ((wz - self.map.center_z) * z) as f32,
        ]
    }

    pub fn surface_y_or_default(&self, wx: i32, wz: i32) -> i32 {
        self.terrain
            .get_column(wx, wz)
            .filter(|c| c.is_explored())
            .map(|c| i32::from(c.height))
            .unwrap_or_else(|| {
                if self.player.valid {
                    self.player.y.round() as i32
                } else {
                    64
                }
            })
    }

    pub fn find_waypoint_near_screen(
        &self,
        sx: f32,
        sy: f32,
        screen_w: f32,
        screen_h: f32,
        radius_px: f32,
    ) -> Option<u64> {
        let mut best: Option<(u64, f32)> = None;
        for wp in &self.waypoints {
            if wp.dim != self.terrain.dimension {
                continue;
            }
            let pos = self.world_to_screen(f64::from(wp.x) + 0.5, f64::from(wp.z) + 0.5, screen_w, screen_h);
            let dx = pos[0] - sx;
            let dy = pos[1] - sy;
            let d2 = dx * dx + dy * dy;
            if d2 <= radius_px * radius_px && best.is_none_or(|b| d2 < b.1) {
                best = Some((wp.id, d2));
            }
        }
        best.map(|(id, _)| id)
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.map.toast = Some((msg.into(), self.tick_counter + 60));
    }

    /// Process queued World Map UI events cleanly (pure unit-testable).
    pub fn process_world_map_events(&mut self, screen_w: f32, screen_h: f32) {
        let layout = WorldMapLayout::compute(screen_w, screen_h);
        self.map.hovered_button = layout.hit_button(self.cursor_x, self.cursor_y);

        while let Some(act) = self.actions.pop_front() {
            match act {
                ModAction::ToggleWorldMap => {
                    if self.map.open {
                        self.close_world_map();
                    } else {
                        self.open_world_map();
                    }
                }
                ModAction::OpenNewWaypoint => {
                    if !self.map.open {
                        self.open_world_map();
                    }
                    let px = self.player.x.floor() as i32;
                    let py = self.player.y.floor() as i32;
                    let pz = self.player.z.floor() as i32;
                    self.create_waypoint_modal_at(px, py, pz, "");
                }
                ModAction::ZoomIn => {
                    self.config.minimap_zoom = (self.config.minimap_zoom * 1.25).clamp(MIN_MINIMAP_ZOOM, MAX_MINIMAP_ZOOM);
                    self.config_dirty = true;
                }
                ModAction::ZoomOut => {
                    self.config.minimap_zoom = (self.config.minimap_zoom / 1.25).clamp(MIN_MINIMAP_ZOOM, MAX_MINIMAP_ZOOM);
                    self.config_dirty = true;
                }
                ModAction::ToggleMinimap => {
                    self.config.enabled = !self.config.enabled;
                    self.config_dirty = true;
                }
                ModAction::ToggleCaveMode => {
                    self.config.cave_mode = self.config.cave_mode.next();
                    self.terrain.invalidate_cave_tiles();
                    self.config_dirty = true;
                }
            }
        }

        if !self.map.open {
            self.ui_events.clear();
            return;
        }

        while let Some(ev) = self.ui_events.pop_front() {
            match ev {
                UiEvent::MouseScroll { x, y, delta_y } => {
                    if self.map.active_panel == ActivePanel::WaypointsList {
                        if delta_y < 0.0 {
                            self.map.waypoints_scroll += 1;
                        } else if delta_y > 0.0 {
                            self.map.waypoints_scroll = self.map.waypoints_scroll.saturating_sub(1);
                        }
                        continue;
                    }
                    if self.map.active_panel != ActivePanel::None {
                        continue;
                    }
                    let (wx_before, wz_before) = self.screen_to_world(x, y, screen_w, screen_h);
                    let factor = if delta_y > 0.0 { 1.18 } else { 1.0 / 1.18 };
                    self.map.zoom = (self.map.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                    self.map.follow_player = false;
                    let z = f64::from(self.map.zoom);
                    self.map.center_x = wx_before - f64::from(x - screen_w * 0.5) / z;
                    self.map.center_z = wz_before - f64::from(y - screen_h * 0.5) / z;
                }
                UiEvent::MouseDown { x, y, button } => {
                    self.cursor_x = x;
                    self.cursor_y = y;
                    if button == 0 {
                        // Modal / Context Menu / Settings / Waypoints List clicks handled first
                        if self.handle_panel_mouse_down(&layout, x, y) {
                            continue;
                        }
                        if self.map.cave_slider_open && layout.cave_slider_track.contains(x, y) {
                            let tr = &layout.cave_slider_track;
                            if y < tr.y + 18.0 {
                                let new_mode = if x < tr.x + tr.w / 3.0 {
                                    CaveMode::Auto
                                } else if x < tr.x + (tr.w * 2.0) / 3.0 {
                                    CaveMode::Manual
                                } else {
                                    CaveMode::Off
                                };
                                self.set_cave_mode_from_ui(new_mode);
                            } else {
                                self.update_cave_slider_from_x(tr, x);
                                self.map.dragging_cave_slider = true;
                            }
                            continue;
                        }
                        if let Some(btn) = layout.hit_button(x, y) {
                            self.handle_toolbar_button(btn, screen_w, screen_h);
                            continue;
                        }
                        self.map.dragging_pan = true;
                        self.map.drag_distance = 0.0;
                        self.map.drag_button = 0;
                    } else if button == 1 {
                        if self.map.active_panel == ActivePanel::ContextMenu
                            || self.map.active_panel == ActivePanel::None
                        {
                            let (wx, wz) = self.screen_to_world(x, y, screen_w, screen_h);
                            let bx = wx.floor() as i32;
                            let bz = wz.floor() as i32;
                            let by = self.surface_y_or_default(bx, bz);
                            let near = self.find_waypoint_near_screen(x, y, screen_w, screen_h, 12.0);
                            self.map.context_menu = Some(ContextMenuState {
                                screen_x: x.min((screen_w - 156.0).max(4.0)),
                                screen_y: y.min((screen_h - 130.0).max(4.0)),
                                world_x: bx,
                                world_y: by,
                                world_z: bz,
                                near_waypoint_id: near,
                            });
                            self.map.active_panel = ActivePanel::ContextMenu;
                        } else {
                            self.map.active_panel = ActivePanel::None;
                        }
                    }
                }
                UiEvent::MouseDrag { x, y, dx, dy, .. } => {
                    self.cursor_x = x;
                    self.cursor_y = y;
                    if self.map.dragging_cave_slider {
                        self.update_cave_slider_from_x(&layout.cave_slider_track, x);
                    } else if self.map.dragging_size_slider {
                        let sw = 236.0f32.min(screen_w - 20.0);
                        let bx = (screen_w - sw) * 0.5 + 18.0;
                        let bw = (sw - 36.0).max(16.0);
                        let frac = ((x - bx) / bw).clamp(0.0, 1.0);
                        self.config.minimap_size =
                            (MIN_MINIMAP_SIZE + frac * (MAX_MINIMAP_SIZE - MIN_MINIMAP_SIZE)).round();
                        self.config_dirty = true;
                    } else if self.map.dragging_pan {
                        self.map.drag_distance += dx.abs() + dy.abs();
                        if self.map.drag_distance > 3.5 {
                            self.map.follow_player = false;
                            let z = f64::from(self.map.zoom.max(MIN_ZOOM));
                            self.map.center_x -= f64::from(dx) / z;
                            self.map.center_z -= f64::from(dy) / z;
                        }
                    }
                }
                UiEvent::MouseUp { x, y, button } => {
                    self.cursor_x = x;
                    self.cursor_y = y;
                    self.map.dragging_cave_slider = false;
                    self.map.dragging_size_slider = false;
                    if button == 0 && self.map.dragging_pan {
                        self.map.dragging_pan = false;
                        if self.map.drag_distance <= 4.0 && self.map.active_panel == ActivePanel::None {
                            if let Some(near_id) = self.find_waypoint_near_screen(x, y, screen_w, screen_h, 11.0) {
                                self.open_edit_waypoint_modal(near_id);
                            } else {
                                let (wx, wz) = self.screen_to_world(x, y, screen_w, screen_h);
                                let bx = wx.floor() as i32;
                                let bz = wz.floor() as i32;
                                let by = self.surface_y_or_default(bx, bz);
                                self.create_waypoint_modal_at(bx, by, bz, "");
                            }
                        }
                    }
                }
                UiEvent::KeyDown { key, ctrl: _, .. } => {
                    self.handle_key_down(&key, screen_w, screen_h);
                }
                UiEvent::CharTyped(ch) => {
                    self.handle_char_typed(ch);
                }
            }
        }
    }

    fn set_cave_mode_from_ui(&mut self, mode: CaveMode) {
        if mode == CaveMode::Manual && self.config.cave_mode == CaveMode::Auto && self.auto_cave_active {
            self.config.cave_y = self.auto_cave_y;
        }
        self.config.cave_mode = mode;
        self.map.cave_slider_open = true;
        self.terrain.invalidate_cave_tiles();
        self.config_dirty = true;
    }

    fn update_cave_slider_from_x(&mut self, track: &UiRect, x: f32) {
        let sx = track.x + 8.0;
        let sw = (track.w - 16.0).max(16.0);
        let t = ((x - sx) / sw).clamp(0.0, 1.0);
        self.config.cave_mode = CaveMode::Manual;
        self.config.cave_y = (-64.0 + t * 384.0).round() as i32;
        self.terrain.invalidate_cave_tiles();
        self.config_dirty = true;
    }

    fn handle_toolbar_button(&mut self, btn: MapButtonId, _screen_w: f32, _screen_h: f32) {
        match btn {
            MapButtonId::Settings => {
                self.map.active_panel = if self.map.active_panel == ActivePanel::Settings {
                    ActivePanel::None
                } else {
                    ActivePanel::Settings
                };
            }
            MapButtonId::QuickWaypoint => {
                let cx = self.map.center_x.floor() as i32;
                let cz = self.map.center_z.floor() as i32;
                let cy = self.surface_y_or_default(cx, cz);
                self.create_waypoint_modal_at(cx, cy, cz, "");
            }
            MapButtonId::PreloadRegion => {
                let cx = self.map.center_x.floor() as i32;
                let cz = self.map.center_z.floor() as i32;
                self.commands.push_back(PendingCommand::ForceRescanLoaded);
                self.commands.push_back(PendingCommand::RequestPreload {
                    center_x: cx,
                    center_z: cz,
                    radius_chunks: 16, // 512x512 block area (32x32 chunks)
                });
            }
            MapButtonId::WaypointsList => {
                self.map.active_panel = if self.map.active_panel == ActivePanel::WaypointsList {
                    ActivePanel::None
                } else {
                    ActivePanel::WaypointsList
                };
            }
            MapButtonId::CaveMode => {
                self.map.cave_slider_open = !self.map.cave_slider_open;
            }
            MapButtonId::HelpMenu => {
                self.map.active_panel = if self.map.active_panel == ActivePanel::Help {
                    ActivePanel::None
                } else {
                    ActivePanel::Help
                };
            }
            MapButtonId::ZoomIn => {
                self.map.zoom = (self.map.zoom * 1.4).clamp(MIN_ZOOM, MAX_ZOOM);
            }
            MapButtonId::ZoomOut => {
                self.map.zoom = (self.map.zoom / 1.4).clamp(MIN_ZOOM, MAX_ZOOM);
            }
            MapButtonId::ResetZoom => {
                self.map.zoom = 1.0;
                self.map.follow_player = true;
                if self.player.valid {
                    self.map.center_x = self.player.x;
                    self.map.center_z = self.player.z;
                }
            }
            MapButtonId::ExitMap => {
                self.close_world_map();
            }
            MapButtonId::ChunkGrid => {
                self.config.chunk_grid = !self.config.chunk_grid;
                self.config_dirty = true;
            }
            MapButtonId::CenterPlayer => {
                self.map.follow_player = true;
                if self.player.valid {
                    self.map.center_x = self.player.x;
                    self.map.center_z = self.player.z;
                }
            }
            MapButtonId::ToggleWaypoints => {
                self.config.show_waypoints_on_map = !self.config.show_waypoints_on_map;
                self.config_dirty = true;
            }
            MapButtonId::ToggleEntities => {
                self.config.show_entities_on_map = !self.config.show_entities_on_map;
                self.config_dirty = true;
            }
        }
    }

    fn handle_panel_mouse_down(&mut self, layout: &WorldMapLayout, x: f32, y: f32) -> bool {
        match self.map.active_panel {
            ActivePanel::None => false,
            ActivePanel::ContextMenu => {
                let Some(cm) = self.map.context_menu.clone() else {
                    self.map.active_panel = ActivePanel::None;
                    return false;
                };
                let mw = 148.0;
                let row_h = 18.0;
                let rows = if cm.near_waypoint_id.is_some() { 6 } else { 5 };
                let mh = (rows as f32) * row_h + 6.0;
                let menu_rect = UiRect { x: cm.screen_x, y: cm.screen_y, w: mw, h: mh };
                if !menu_rect.contains(x, y) {
                    self.map.active_panel = ActivePanel::None;
                    self.map.context_menu = None;
                    return true;
                }
                let idx = ((y - cm.screen_y - 3.0) / row_h).floor() as i32;
                self.map.active_panel = ActivePanel::None;
                self.map.context_menu = None;
                let mut cur_idx = 0;
                if let Some(wp_id) = cm.near_waypoint_id {
                    if idx == cur_idx {
                        self.delete_waypoint(wp_id);
                        return true;
                    }
                    cur_idx += 1;
                }
                match idx - cur_idx {
                    0 => {
                        self.create_waypoint_modal_at(cm.world_x, cm.world_y, cm.world_z, "");
                    }
                    1 => {
                        let id = self.next_waypoint_id;
                        self.next_waypoint_id += 1;
                        self.add_or_update_waypoint(Waypoint {
                            id,
                            name: format!("Point #{}", id),
                            initial: "P".to_owned(),
                            color: PALETTE[(id as usize) % (PALETTE.len() - 1) + 1],
                            x: cm.world_x,
                            y: cm.world_y,
                            z: cm.world_z,
                            dim: self.terrain.dimension.clone(),
                            enabled: true,
                            is_death: false,
                        });
                    }
                    2 => {
                        self.commands.push_back(PendingCommand::RequestPreload {
                            center_x: cm.world_x,
                            center_z: cm.world_z,
                            radius_chunks: 16,
                        });
                    }
                    3 => {
                        self.commands.push_back(PendingCommand::Teleport {
                            x: cm.world_x,
                            y: cm.world_y + 1,
                            z: cm.world_z,
                        });
                        self.close_world_map();
                    }
                    4 => {
                        let txt = format!("{} {} {}", cm.world_x, cm.world_y, cm.world_z);
                        self.commands.push_back(PendingCommand::CopyClipboard(txt.clone()));
                        self.set_toast(format!("Copied: {txt}"));
                    }
                    _ => {}
                }
                true
            }
            ActivePanel::WaypointModal => {
                let bx = layout.modal_box;
                if !bx.contains(x, y) {
                    self.map.active_panel = ActivePanel::None;
                    return true;
                }
                // Name box: (bx.x + 12, bx.y + 28, bx.w - 64, 18)
                let name_r = UiRect { x: bx.x + 12.0, y: bx.y + 28.0, w: bx.w - 66.0, h: 18.0 };
                let init_r = UiRect { x: bx.x + bx.w - 48.0, y: bx.y + 28.0, w: 36.0, h: 18.0 };
                let y_r = UiRect { x: bx.x + 32.0, y: bx.y + 54.0, w: 68.0, h: 18.0 };
                let y_minus = UiRect { x: bx.x + 12.0, y: bx.y + 54.0, w: 18.0, h: 18.0 };
                let y_plus = UiRect { x: bx.x + 102.0, y: bx.y + 54.0, w: 18.0, h: 18.0 };
                if name_r.contains(x, y) {
                    self.map.modal.active_field = ModalField::Name;
                    return true;
                }
                if init_r.contains(x, y) {
                    self.map.modal.active_field = ModalField::Initial;
                    return true;
                }
                if y_r.contains(x, y) {
                    self.map.modal.active_field = ModalField::Y;
                    return true;
                }
                if y_minus.contains(x, y) {
                    let v = (self.map.modal.y - 4).clamp(-64, 320);
                    self.map.modal.y = v;
                    self.map.modal.y_text = v.to_string();
                    return true;
                }
                if y_plus.contains(x, y) {
                    let v = (self.map.modal.y + 4).clamp(-64, 320);
                    self.map.modal.y = v;
                    self.map.modal.y_text = v.to_string();
                    return true;
                }
                // 12 color swatches (2 rows x 6 cols) starting at (bx.x + 12, bx.y + 82)
                for i in 0..PALETTE.len() {
                    let col = (i % 6) as f32;
                    let row = (i / 6) as f32;
                    let sw = UiRect {
                        x: bx.x + 12.0 + col * 24.0,
                        y: bx.y + 82.0 + row * 22.0,
                        w: 20.0,
                        h: 18.0,
                    };
                    if sw.contains(x, y) {
                        self.map.modal.color_idx = i;
                        return true;
                    }
                }
                // Bottom action buttons at bx.y + bx.h - 28
                let save_r = UiRect { x: bx.x + 12.0, y: bx.y + bx.h - 28.0, w: 66.0, h: 20.0 };
                let cancel_r = UiRect { x: bx.x + 84.0, y: bx.y + bx.h - 28.0, w: 64.0, h: 20.0 };
                let delete_r = UiRect { x: bx.x + bx.w - 68.0, y: bx.y + bx.h - 28.0, w: 56.0, h: 20.0 };
                if save_r.contains(x, y) {
                    self.commit_waypoint_modal();
                    return true;
                }
                if cancel_r.contains(x, y) {
                    self.map.active_panel = ActivePanel::None;
                    return true;
                }
                if self.map.modal.editing_id.is_some() && delete_r.contains(x, y) {
                    if let Some(id) = self.map.modal.editing_id {
                        self.delete_waypoint(id);
                    }
                    self.map.active_panel = ActivePanel::None;
                    return true;
                }
                true
            }
            ActivePanel::Settings => {
                let sw = 236.0f32.min(layout.screen_w - 14.0);
                let sh = 222.0f32.min(layout.screen_h - 14.0);
                let sx = (layout.screen_w - sw) * 0.5;
                let sy = (layout.screen_h - sh) * 0.5;
                let panel = UiRect { x: sx, y: sy, w: sw, h: sh };
                if !panel.contains(x, y) {
                    self.map.active_panel = ActivePanel::None;
                    return true;
                }
                // Size slider track
                let size_slider = UiRect { x: sx + 18.0, y: sy + 32.0, w: sw - 36.0, h: 15.0 };
                if size_slider.contains(x, y) {
                    let frac = ((x - size_slider.x) / size_slider.w).clamp(0.0, 1.0);
                    self.config.minimap_size =
                        (MIN_MINIMAP_SIZE + frac * (MAX_MINIMAP_SIZE - MIN_MINIMAP_SIZE)).round();
                    self.map.dragging_size_slider = true;
                    self.config_dirty = true;
                    return true;
                }
                // Size preset chips (Small 84, Default 104, Large 140, XL 180)
                let presets = [84.0f32, 104.0, 140.0, 180.0];
                for (i, &p) in presets.iter().enumerate() {
                    let chip = UiRect {
                        x: sx + 12.0 + (i as f32) * 53.0,
                        y: sy + 50.0,
                        w: 49.0,
                        h: 15.0,
                    };
                    if chip.contains(x, y) {
                        self.config.minimap_size = p;
                        self.config_dirty = true;
                        return true;
                    }
                }
                // Minimap zoom - / + controls at sy + 69
                let zoom_minus = UiRect { x: sx + sw - 54.0, y: sy + 69.0, w: 20.0, h: 15.0 };
                let zoom_plus = UiRect { x: sx + sw - 32.0, y: sy + 69.0, w: 20.0, h: 15.0 };
                if zoom_minus.contains(x, y) {
                    self.config.minimap_zoom = (self.config.minimap_zoom / 1.2).clamp(MIN_MINIMAP_ZOOM, MAX_MINIMAP_ZOOM);
                    self.config_dirty = true;
                    return true;
                }
                if zoom_plus.contains(x, y) {
                    self.config.minimap_zoom = (self.config.minimap_zoom * 1.2).clamp(MIN_MINIMAP_ZOOM, MAX_MINIMAP_ZOOM);
                    self.config_dirty = true;
                    return true;
                }
                // Toggle rows starting at sy + 90, step 15.5px (fits cleanly on 720p too!)
                let toggles_y0 = sy + 90.0;
                for row in 0..8 {
                    let r = UiRect { x: sx + 12.0, y: toggles_y0 + (row as f32) * 15.5, w: sw - 24.0, h: 14.0 };
                    if r.contains(x, y) {
                        match row {
                            0 => {
                                self.config.shape = match self.config.shape {
                                    MinimapShape::Square => MinimapShape::Round,
                                    MinimapShape::Round => MinimapShape::Square,
                                };
                            }
                            1 => {
                                self.config.corner = MinimapCorner::from_u8((self.config.corner as u8 + 1) % 4);
                            }
                            2 => {
                                self.config.rotate_with_player = !self.config.rotate_with_player;
                            }
                            3 => {
                                self.config.cave_mode = self.config.cave_mode.next();
                                self.terrain.invalidate_cave_tiles();
                            }
                            4 => self.config.show_entities = !self.config.show_entities,
                            5 => self.config.show_waypoints_3d = !self.config.show_waypoints_3d,
                            6 => self.config.auto_death_waypoint = !self.config.auto_death_waypoint,
                            7 => self.config.show_waypoints_on_map = !self.config.show_waypoints_on_map,
                            _ => {}
                        }
                        self.config_dirty = true;
                        return true;
                    }
                }
                let close_btn = UiRect { x: sx + sw - 24.0, y: sy + 5.0, w: 18.0, h: 15.0 };
                if close_btn.contains(x, y) {
                    self.map.active_panel = ActivePanel::None;
                }
                true
            }
            ActivePanel::WaypointsList => {
                let pw = 256.0f32.min(layout.screen_w - 20.0);
                let ph = 200.0f32.min(layout.screen_h - 20.0);
                let px = (layout.screen_w - pw) * 0.5;
                let py = (layout.screen_h - ph) * 0.5;
                let panel = UiRect { x: px, y: py, w: pw, h: ph };
                if !panel.contains(x, y) {
                    self.map.active_panel = ActivePanel::None;
                    return true;
                }
                let close_btn = UiRect { x: px + pw - 24.0, y: py + 6.0, w: 18.0, h: 16.0 };
                if close_btn.contains(x, y) {
                    self.map.active_panel = ActivePanel::None;
                    return true;
                }
                let dim = self.terrain.dimension.clone();
                let ids: Vec<(u64, i32, i32, i32)> = self
                    .waypoints
                    .iter()
                    .filter(|w| w.dim == dim)
                    .map(|w| (w.id, w.x, w.y, w.z))
                    .collect();
                let skip = self.map.waypoints_scroll.min(ids.len().saturating_sub(1));
                for (row_idx, &(id, wx, _wy, wz)) in ids.iter().skip(skip).take(8).enumerate() {
                    let ry = py + 28.0 + (row_idx as f32) * 20.0;
                    let toggle_r = UiRect { x: px + 10.0, y: ry, w: 16.0, h: 16.0 };
                    let goto_r = UiRect { x: px + 30.0, y: ry, w: pw - 96.0, h: 16.0 };
                    let edit_r = UiRect { x: px + pw - 62.0, y: ry, w: 24.0, h: 16.0 };
                    let del_r = UiRect { x: px + pw - 34.0, y: ry, w: 24.0, h: 16.0 };
                    if toggle_r.contains(x, y) {
                        if let Some(w) = self.waypoints.iter_mut().find(|w| w.id == id) {
                            w.enabled = !w.enabled;
                            self.waypoints_dirty = true;
                        }
                        return true;
                    }
                    if goto_r.contains(x, y) {
                        self.map.follow_player = false;
                        self.map.center_x = f64::from(wx) + 0.5;
                        self.map.center_z = f64::from(wz) + 0.5;
                        self.map.active_panel = ActivePanel::None;
                        return true;
                    }
                    if edit_r.contains(x, y) {
                        self.open_edit_waypoint_modal(id);
                        return true;
                    }
                    if del_r.contains(x, y) {
                        self.delete_waypoint(id);
                        return true;
                    }
                }
                true
            }
            ActivePanel::Help => {
                self.map.active_panel = ActivePanel::None;
                true
            }
        }
    }

    fn handle_key_down(&mut self, key: &str, _screen_w: f32, _screen_h: f32) {
        if key == "Escape" {
            if self.map.active_panel != ActivePanel::None {
                self.map.active_panel = ActivePanel::None;
                self.map.context_menu = None;
            } else {
                self.close_world_map();
            }
            return;
        }
        if self.map.active_panel == ActivePanel::WaypointModal {
            match key {
                "Enter" => self.commit_waypoint_modal(),
                "Tab" => {
                    self.map.modal.active_field = match self.map.modal.active_field {
                        ModalField::Name => ModalField::Initial,
                        ModalField::Initial => ModalField::Y,
                        ModalField::Y => ModalField::Name,
                    };
                }
                "Backspace" => match self.map.modal.active_field {
                    ModalField::Name => {
                        self.map.modal.name.pop();
                        if !self.map.modal.initial_manual {
                            self.map.modal.initial = derive_initial(&self.map.modal.name);
                        }
                    }
                    ModalField::Initial => {
                        self.map.modal.initial.pop();
                        self.map.modal.initial_manual = true;
                    }
                    ModalField::Y => {
                        self.map.modal.y_text.pop();
                    }
                },
                _ => {}
            }
            return;
        }
        if self.map.active_panel == ActivePanel::None
            && (key == "m" || key == "M" || key == "ь" || key == "Ь" || key == self.world_map_key)
        {
            self.close_world_map();
        }
    }

    fn handle_char_typed(&mut self, ch: char) {
        if self.map.active_panel != ActivePanel::WaypointModal || ch.is_control() {
            return;
        }
        match self.map.modal.active_field {
            ModalField::Name => {
                if self.map.modal.name.chars().count() < 32 {
                    self.map.modal.name.push(ch);
                    if !self.map.modal.initial_manual {
                        self.map.modal.initial = derive_initial(&self.map.modal.name);
                    }
                }
            }
            ModalField::Initial => {
                if self.map.modal.initial.chars().count() < 2 && !ch.is_whitespace() {
                    self.map.modal.initial.push(ch);
                    self.map.modal.initial_manual = true;
                }
            }
            ModalField::Y => {
                if (ch.is_ascii_digit() || (ch == '-' && self.map.modal.y_text.is_empty()))
                    && self.map.modal.y_text.len() < 5
                {
                    self.map.modal.y_text.push(ch);
                }
            }
        }
    }
}

static STATE: OnceLock<Mutex<EternityMapState>> = OnceLock::new();

pub fn state() -> &'static Mutex<EternityMapState> {
    STATE.get_or_init(|| Mutex::new(EternityMapState::new()))
}
