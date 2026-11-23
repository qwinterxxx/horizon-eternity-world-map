//! Eternity world map — zero-overhead native minimap, persistent fullscreen world map, and waypoints.

pub mod atlas_cache;
pub mod entities;
pub mod mob_icons;
pub mod state;
pub mod terrain;
pub mod ui_math;
pub mod waypoints;

use crate::atlas_cache::{BiomeTable, RegionBuffer, floor_div};
use crate::state::{ModAction, ModConfig, UiEvent, state};
use crate::terrain::CaveMode;
use crate::ui_math::{MinimapCorner, MinimapShape};
use crate::waypoints::{decode_waypoints, encode_waypoints};
use mod_api::{
    self, ClientHost, ClientTickPostEvent, ConfigFiles, ConfigPath, Flow, Hooks,
    KeyConflictContext, KeyRegistration, Registrar, RegistrationContext, ScreenCharTypedPreEvent,
    ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent, SettingChangedEvent, SettingSpec,
};

pub const MOD_ID: &str = "eternity_world_map";

#[derive(Default)]
pub struct Mod;

fn fnv1a_32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for &b in bytes {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

pub fn sanitize_path_token(s: &str) -> String {
    let mut prefix = String::with_capacity(s.len().min(48));
    let mut needs_hash = false;
    for (i, c) in s.chars().enumerate() {
        let safe = c.is_ascii_alphanumeric() || c == '_' || c == '-';
        if !safe || i >= 48 {
            needs_hash = true;
        }
        if i < 48 {
            prefix.push(if safe { c } else { '_' });
        }
    }
    if prefix.is_empty() {
        return "default".to_owned();
    }
    if needs_hash {
        let trimmed: String = prefix.chars().take(39).collect();
        format!("{trimmed}_{:08x}", fnv1a_32(s.as_bytes()))
    } else {
        prefix
    }
}

/// Legacy v0.1.0 path sanitizer (no hash suffix) used as read-only fallback so existing
/// v0.1.0 server/world maps migrate seamlessly on load.
fn sanitize_path_token_legacy(s: &str) -> String {
    let out: String = s
        .chars()
        .take(48)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "default".to_owned()
    } else {
        out
    }
}

fn read_data_file(files: &dyn ConfigFiles, path: &str) -> Option<Vec<u8>> {
    let cp = ConfigPath::new(path).ok()?;
    files.read(&cp).ok()
}

fn write_data_file(files: &dyn ConfigFiles, path: &str, bytes: &[u8]) {
    if let Ok(cp) = ConfigPath::new(path) {
        let _ = files.write(&cp, bytes);
    }
}

fn normalize_hook_key(key: &str) -> (&'static str, String) {
    match key {
        "key.keyboard.escape" => ("Escape", String::new()),
        "key.keyboard.enter" | "key.keyboard.keypad.enter" => ("Enter", String::new()),
        "key.keyboard.backspace" => ("Backspace", String::new()),
        "key.keyboard.tab" => ("Tab", String::new()),
        "key.keyboard.m" => ("m", "m".to_owned()),
        _ => ("", key.to_owned()),
    }
}

fn sync_settings_from_host(host: &dyn ClientHost, cfg: &mut ModConfig) {
    if let Some(v) = host.setting_bool("enabled") {
        cfg.enabled = v;
    }
    if let Some(v) = host.setting_f64("minimap_size") {
        cfg.minimap_size = (v as f32).clamp(state::MIN_MINIMAP_SIZE, state::MAX_MINIMAP_SIZE);
    }
    if let Some(v) = host.setting_choice("shape") {
        cfg.shape = if v == 1 {
            MinimapShape::Round
        } else {
            MinimapShape::Square
        };
    }
    if let Some(v) = host.setting_choice("corner") {
        cfg.corner = MinimapCorner::from_u8(v as u8);
    }
    if let Some(v) = host.setting_f64("minimap_zoom") {
        cfg.minimap_zoom = (v as f32).clamp(state::MIN_MINIMAP_ZOOM, state::MAX_MINIMAP_ZOOM);
    }
    if let Some(v) = host.setting_bool("rotate") {
        cfg.rotate_with_player = v;
    }
    if let Some(v) = host.setting_choice("cave_mode") {
        cfg.cave_mode = CaveMode::from_u8(v as u8);
    }
    if let Some(v) = host.setting_f64("cave_y") {
        cfg.cave_y = (v.round() as i32).clamp(-64, 320);
    }
    if let Some(v) = host.setting_bool("entities") {
        cfg.show_entities = v;
    }
    if let Some(v) = host.setting_bool("waypoints_3d") {
        cfg.show_waypoints_3d = v;
    }
    if let Some(v) = host.setting_bool("death_waypoint") {
        cfg.auto_death_waypoint = v;
    }
    if let Some(v) = host.setting_bool("chunk_grid") {
        cfg.chunk_grid = v;
    }
    if let Some(v) = host.setting_bool("realistic_colors") {
        cfg.realistic_colors = v;
    }
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn Registrar, _cx: &RegistrationContext) {
        let _ = registrar.register_setting(SettingSpec::toggle(
            "enabled",
            "config.eternity_world_map.enabled",
            true,
        ));
        let _ = registrar.register_setting(SettingSpec::range(
            "minimap_size",
            "config.eternity_world_map.size",
            64.0,
            256.0,
            4.0,
            104.0,
        ));
        let _ = registrar.register_setting(SettingSpec::choice(
            "shape",
            "config.eternity_world_map.shape",
            [
                "config.eternity_world_map.shape.square",
                "config.eternity_world_map.shape.round",
            ],
            0,
        ));
        let _ = registrar.register_setting(SettingSpec::choice(
            "corner",
            "config.eternity_world_map.corner",
            [
                "config.eternity_world_map.corner.top_left",
                "config.eternity_world_map.corner.top_right",
                "config.eternity_world_map.corner.bottom_left",
                "config.eternity_world_map.corner.bottom_right",
            ],
            0,
        ));
        let _ = registrar.register_setting(SettingSpec::range(
            "minimap_zoom",
            "config.eternity_world_map.zoom",
            0.35,
            4.0,
            0.05,
            0.58,
        ));
        let _ = registrar.register_setting(SettingSpec::toggle(
            "rotate",
            "config.eternity_world_map.rotate",
            true,
        ));
        let _ = registrar.register_setting(SettingSpec::choice(
            "cave_mode",
            "config.eternity_world_map.cave_mode",
            [
                "config.eternity_world_map.cave_mode.auto",
                "config.eternity_world_map.cave_mode.off",
                "config.eternity_world_map.cave_mode.manual",
            ],
            0,
        ));
        let _ = registrar.register_setting(SettingSpec::range(
            "cave_y",
            "config.eternity_world_map.cave_y",
            -64.0,
            320.0,
            1.0,
            40.0,
        ));
        let _ = registrar.register_setting(SettingSpec::toggle(
            "entities",
            "config.eternity_world_map.entities",
            true,
        ));
        let _ = registrar.register_setting(SettingSpec::toggle(
            "waypoints_3d",
            "config.eternity_world_map.waypoints_3d",
            true,
        ));
        let _ = registrar.register_setting(SettingSpec::toggle(
            "death_waypoint",
            "config.eternity_world_map.death_waypoint",
            true,
        ));
        let _ = registrar.register_setting(SettingSpec::toggle(
            "chunk_grid",
            "config.eternity_world_map.chunk_grid",
            false,
        ));
        let _ = registrar.register_setting(SettingSpec::toggle(
            "realistic_colors",
            "config.eternity_world_map.realistic_colors",
            true,
        ));

        registrar.register_key_mapping(
            KeyRegistration::new("world_map", "key.keyboard.m")
                .label("key.eternity_world_map.world_map")
                .category("key.category.eternity_world_map")
                .context(KeyConflictContext::InGame),
        );
        registrar.register_key_mapping(
            KeyRegistration::new("new_waypoint", "key.keyboard.b")
                .label("key.eternity_world_map.new_waypoint")
                .category("key.category.eternity_world_map")
                .context(KeyConflictContext::InGame),
        );
        registrar.register_key_mapping(
            KeyRegistration::new("zoom_in", "key.keyboard.equal")
                .label("key.eternity_world_map.zoom_in")
                .category("key.category.eternity_world_map")
                .context(KeyConflictContext::InGame),
        );
        registrar.register_key_mapping(
            KeyRegistration::new("zoom_out", "key.keyboard.minus")
                .label("key.eternity_world_map.zoom_out")
                .category("key.category.eternity_world_map")
                .context(KeyConflictContext::InGame),
        );
        registrar.register_key_mapping(
            KeyRegistration::new("toggle_minimap", "key.keyboard.h")
                .label("key.eternity_world_map.toggle_minimap")
                .category("key.category.eternity_world_map")
                .context(KeyConflictContext::InGame),
        );
        registrar.register_key_mapping(
            KeyRegistration::new("cave_mode", "key.keyboard.c")
                .label("key.eternity_world_map.cave_mode")
                .category("key.category.eternity_world_map")
                .context(KeyConflictContext::InGame),
        );
    }

    fn setting_changed(&self, host: &mut dyn ClientHost, event: &mut SettingChangedEvent<'_>) {
        if !host.on(MOD_ID) {
            return;
        }
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        let prev_cave = (st.config.cave_mode, st.config.cave_y);
        sync_settings_from_host(host, &mut st.config);
        let _ = event.key;
        if (st.config.cave_mode, st.config.cave_y) != prev_cave {
            st.terrain.invalidate_cave_tiles();
        }
        st.config_dirty = true;
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _event: &mut ClientTickPostEvent) {
        if !host.on(MOD_ID) {
            return;
        }
        {
            let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
            while host.consume_click(MOD_ID, "world_map") {
                st.actions.push_back(ModAction::ToggleWorldMap);
            }
            while host.consume_click(MOD_ID, "new_waypoint") {
                st.actions.push_back(ModAction::OpenNewWaypoint);
            }
            while host.consume_click(MOD_ID, "zoom_in") {
                st.actions.push_back(ModAction::ZoomIn);
            }
            while host.consume_click(MOD_ID, "zoom_out") {
                st.actions.push_back(ModAction::ZoomOut);
            }
            while host.consume_click(MOD_ID, "toggle_minimap") {
                st.actions.push_back(ModAction::ToggleMinimap);
            }
            while host.consume_click(MOD_ID, "cave_mode") {
                st.actions.push_back(ModAction::ToggleCaveMode);
            }
        }
        let files = host.data_files();

        let (want_open, want_close, current_screen) = {
            let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
            st.tick_counter += 1;

            if !st.config_loaded {
                st.config_loaded = true;
                sync_settings_from_host(host, &mut st.config);
                if let Some(bytes) = read_data_file(files, "config.bin")
                    && let Some(decoded) = ModConfig::decode(&bytes)
                {
                    st.config = decoded;
                    if bytes.len() >= 8 && &bytes[0..8] != b"ETWMCFG1" {
                        st.config_dirty = true;
                    }
                }
            }

            if st.config_dirty {
                st.config_dirty = false;
                let encoded = st.config.encode();
                write_data_file(files, "config.bin", &encoded);
            }

            // Flush any queued disk writes from a recent world/dimension switch immediately.
            if !st.pending_disk_writes.is_empty() {
                let queued = std::mem::take(&mut st.pending_disk_writes);
                for (path, bytes) in queued {
                    write_data_file(files, &path, &bytes);
                }
            }

            let wkey = sanitize_path_token(&st.terrain.world_key);
            let dkey = sanitize_path_token(&st.terrain.dimension);
            let wkey_legacy = sanitize_path_token_legacy(&st.terrain.world_key);
            let dkey_legacy = sanitize_path_token_legacy(&st.terrain.dimension);
            let valid_world = !st.terrain.world_key.is_empty();

            if valid_world
                && (st.loaded_regions_key.0 != st.terrain.world_key
                    || st.loaded_regions_key.1 != st.terrain.dimension)
            {
                st.loaded_regions_key = (
                    st.terrain.world_key.clone(),
                    st.terrain.dimension.clone(),
                );
                st.loaded_regions.clear();
            }

            if valid_world && st.biomes_loaded_world != st.terrain.world_key {
                st.biomes_loaded_world = st.terrain.world_key.clone();
                let biome_bytes = read_data_file(files, &format!("biomes/{wkey}.bin"))
                    .or_else(|| {
                        if wkey_legacy != wkey {
                            read_data_file(files, &format!("biomes/{wkey_legacy}.bin"))
                        } else {
                            None
                        }
                    });
                if let Some(bytes) = biome_bytes
                    && let Some(tbl) = BiomeTable::decode(&bytes)
                {
                    st.terrain.biomes = tbl;
                }
                st.loaded_regions.clear();
            }
            if valid_world && st.waypoints_loaded_world != st.terrain.world_key {
                st.waypoints_loaded_world = st.terrain.world_key.clone();
                let wp_bytes = read_data_file(files, &format!("waypoints/{wkey}.bin"))
                    .or_else(|| {
                        if wkey_legacy != wkey {
                            read_data_file(files, &format!("waypoints/{wkey_legacy}.bin"))
                        } else {
                            None
                        }
                    });
                if let Some(bytes) = wp_bytes
                    && let Some((list, next_id)) = decode_waypoints(&bytes)
                {
                    st.waypoints = list;
                    st.next_waypoint_id = next_id.max(1);
                } else {
                    st.waypoints.clear();
                    st.next_waypoint_id = 1;
                }
            }

            // Lazily load visible/nearby persisted regions on disk once per region coordinate,
            // merging into any in-memory region that already received freshly scanned chunks.
            if valid_world && st.player.valid {
                let prx = floor_div(st.player.x.floor() as i32, atlas_cache::REGION_SIZE);
                let prz = floor_div(st.player.z.floor() as i32, atlas_cache::REGION_SIZE);
                let vr = if st.map.open {
                    let crx = floor_div(st.map.center_x.floor() as i32, atlas_cache::REGION_SIZE);
                    let crz = floor_div(st.map.center_z.floor() as i32, atlas_cache::REGION_SIZE);
                    (crx, crz, 3)
                } else {
                    (prx, prz, 1)
                };
                for rz in (vr.1 - vr.2)..=(vr.1 + vr.2) {
                    for rx in (vr.0 - vr.2)..=(vr.0 + vr.2) {
                        if !st.loaded_regions.insert((rx, rz)) {
                            continue;
                        }
                        let path = format!("regions/{wkey}/{dkey}/r_{rx}_{rz}.bin");
                        let reg_bytes = read_data_file(files, &path).or_else(|| {
                            if wkey_legacy != wkey || dkey_legacy != dkey {
                                read_data_file(
                                    files,
                                    &format!("regions/{wkey_legacy}/{dkey_legacy}/r_{rx}_{rz}.bin"),
                                )
                            } else {
                                None
                            }
                        });
                        if let Some(bytes) = reg_bytes
                            && let Some(reg) = RegionBuffer::decode(&bytes)
                        {
                            st.terrain.merge_disk_region(rx, rz, reg);
                        }
                    }
                }
            }

            if valid_world && st.waypoints_dirty {
                st.waypoints_dirty = false;
                let bytes = encode_waypoints(&st.waypoints, st.next_waypoint_id);
                write_data_file(files, &format!("waypoints/{wkey}.bin"), &bytes);
            }

            if valid_world && st.biomes_dirty && (st.tick_counter % 20 == 0 || !st.player.valid) {
                st.biomes_dirty = false;
                let bytes = st.terrain.biomes.encode();
                write_data_file(files, &format!("biomes/{wkey}.bin"), &bytes);
            }

            // Flush dirty regions in background (1 per 10 ticks while playing, or all on leave/disconnect).
            if valid_world && (!st.player.valid || st.tick_counter % 10 == 0) {
                let max_flush = if !st.player.valid { 64 } else { 1 };
                for _ in 0..max_flush {
                    let dirty_coord = st
                        .terrain
                        .regions
                        .iter()
                        .find(|(_, r)| r.dirty_to_disk)
                        .map(|(&c, _)| c);
                    let Some((rx, rz)) = dirty_coord else {
                        break;
                    };
                    if let Some(reg) = st.terrain.regions.get_mut(&(rx, rz)) {
                        reg.dirty_to_disk = false;
                        let encoded = reg.encode();
                        write_data_file(
                            files,
                            &format!("regions/{wkey}/{dkey}/r_{rx}_{rz}.bin"),
                            &encoded,
                        );
                    }
                }
            }

            let want_open = std::mem::take(&mut st.map.want_open);
            let want_close = std::mem::take(&mut st.map.want_close);
            (want_open, want_close, host.mod_screen() == Some(MOD_ID))
        };

        if want_open && !current_screen {
            let _ = host.mod_screen_open(MOD_ID);
        } else if want_close && current_screen {
            host.mod_screen_close(MOD_ID);
        } else if !current_screen {
            let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
            if st.map.open && !st.map.want_open {
                // Player closed Screen::Mod externally (e.g. pause / disconnect)
                st.close_world_map();
                st.map.want_close = false;
            }
        }
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(MOD_ID) {
            return Flow::Continue;
        }
        event.cancelled = true;
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        let [mx, my] = [event.screen.mouse[0] as f32, event.screen.mouse[1] as f32];
        st.cursor_x = mx;
        st.cursor_y = my;
        st.ui_events.push_back(UiEvent::MouseDown {
            x: mx,
            y: my,
            button: event.button,
        });
        Flow::Cancel
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(MOD_ID) {
            return Flow::Continue;
        }
        event.cancelled = true;
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        let [mx, my] = [event.screen.mouse[0] as f32, event.screen.mouse[1] as f32];
        st.cursor_x = mx;
        st.cursor_y = my;
        st.ui_events.push_back(UiEvent::MouseUp {
            x: mx,
            y: my,
            button: event.button,
        });
        Flow::Cancel
    }

    fn screen_mouse_dragged_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseDraggedPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(MOD_ID) {
            return Flow::Continue;
        }
        event.cancelled = true;
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        let [mx, my] = [event.screen.mouse[0] as f32, event.screen.mouse[1] as f32];
        st.cursor_x = mx;
        st.cursor_y = my;
        st.ui_events.push_back(UiEvent::MouseDrag {
            x: mx,
            y: my,
            dx: event.delta[0],
            dy: event.delta[1],
            button: event.button,
        });
        Flow::Cancel
    }

    fn screen_mouse_scrolled_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseScrolledPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(MOD_ID) {
            return Flow::Continue;
        }
        event.cancelled = true;
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        let [mx, my] = [event.screen.mouse[0] as f32, event.screen.mouse[1] as f32];
        st.cursor_x = mx;
        st.cursor_y = my;
        st.ui_events.push_back(UiEvent::MouseScroll {
            x: mx,
            y: my,
            delta_y: event.scroll[1] as f32,
        });
        Flow::Cancel
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(MOD_ID) {
            return Flow::Continue;
        }
        event.cancelled = true;
        let (norm, raw) = normalize_hook_key(event.key);
        let key = if norm.is_empty() { raw } else { norm.to_owned() };
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        st.ui_events.push_back(UiEvent::KeyDown {
            key,
            shift: (event.modifiers & 1) != 0,
            ctrl: (event.modifiers & 2) != 0,
        });
        Flow::Cancel
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(MOD_ID) {
            return Flow::Continue;
        }
        event.cancelled = true;
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        st.ui_events.push_back(UiEvent::CharTyped(event.c));
        Flow::Cancel
    }
}

#[cfg(test)]
mod tests {
    use crate::sanitize_path_token;
    use crate::atlas_cache::{BiomeTable, RegionBuffer, SLOT_INNER, SLOT_STRIDE, TerrainStore, TileKey};
    use crate::entities::{OverlaySource, mob_face_spec, mob_model_spec, normalize_species};
    use crate::state::{EternityMapState, ModConfig};
    use crate::terrain::{
        BiomeColors, BlockLook, ColumnCell, TintMode, apply_biome_tint, scan_cave_column,
        shade_column_pixel,
    };
    use crate::ui_math::{
        ClipVertex, MinimapCorner, clip_quad_circle, clip_quad_rect, minimap_rect, world_to_minimap,
    };
    use crate::waypoints::{
        PALETTE, Waypoint, clamp_to_circle, clamp_to_square, decode_waypoints, derive_initial,
        encode_waypoints, project_world_point,
    };

    #[test]
    fn waypoint_initial_and_codec_roundtrip() {
        assert_eq!(derive_initial("дом"), "Д");
        assert_eq!(derive_initial("Farm"), "F");
        assert_eq!(derive_initial("   "), "W");

        let list = vec![
            Waypoint {
                id: 1,
                name: "База".to_owned(),
                initial: "Б".to_owned(),
                color: PALETTE[1],
                x: -2353,
                y: 70,
                z: -2098,
                dim: "minecraft:overworld".to_owned(),
                enabled: true,
                is_death: false,
            },
            Waypoint {
                id: 2,
                name: "Death".to_owned(),
                initial: "D".to_owned(),
                color: PALETTE[0],
                x: 120,
                y: -15,
                z: 450,
                dim: "minecraft:the_nether".to_owned(),
                enabled: true,
                is_death: true,
            },
        ];
        let encoded = encode_waypoints(&list, 17);
        let (decoded, next_id) = decode_waypoints(&encoded).expect("decode");
        assert_eq!(next_id, 17);
        assert_eq!(decoded, list);
    }

    #[test]
    fn minimap_rim_clamping_and_rotation_match_screenshot1() {
        // Facing SE (~ -45 deg? wait yaw_deg: South=0(+Z), East=-90(+X) => SE = -45 deg)
        // North (0, -1) relative to player facing SE should project Bottom-Left (-x, +y).
        let north = world_to_minimap(0.0, -50.0, 0.0, 0.0, -45.0, true, 1.0, 50.0, 50.0);
        assert!(north[0] < 50.0 && north[1] > 50.0, "North when facing SE should be bottom-left: {north:?}");
        // East (+1, 0) should project Top-Left (-x, -y)
        let east = world_to_minimap(50.0, 0.0, 0.0, 0.0, -45.0, true, 1.0, 50.0, 50.0);
        assert!(east[0] < 50.0 && east[1] < 50.0, "East when facing SE should be top-left: {east:?}");
        // South (0, +1) should project Top-Right (+x, -y)
        let south = world_to_minimap(0.0, 50.0, 0.0, 0.0, -45.0, true, 1.0, 50.0, 50.0);
        assert!(south[0] > 50.0 && south[1] < 50.0, "South when facing SE should be top-right: {south:?}");
        // West (-1, 0) should project Bottom-Right (+x, +y)
        let west = world_to_minimap(-50.0, 0.0, 0.0, 0.0, -45.0, true, 1.0, 50.0, 50.0);
        assert!(west[0] > 50.0 && west[1] > 50.0, "West when facing SE should be bottom-right: {west:?}");

        let (sx, sy, clamped) = clamp_to_square(100.0, 50.0, 40.0);
        assert!(clamped);
        assert!((sx - 40.0).abs() < 1e-4 && (sy - 20.0).abs() < 1e-4);

        let (cx, cy, c_clamped) = clamp_to_circle(60.0, 80.0, 50.0);
        assert!(c_clamped);
        assert!(((cx * cx + cy * cy).sqrt() - 50.0).abs() < 1e-3);
    }

    #[test]
    fn polygon_clipping_and_3d_projection() {
        let quad = [
            ClipVertex { pos: [-10.0, -10.0], uv: [0.0, 0.0] },
            ClipVertex { pos: [110.0, -10.0], uv: [1.0, 0.0] },
            ClipVertex { pos: [110.0, 110.0], uv: [1.0, 1.0] },
            ClipVertex { pos: [-10.0, 110.0], uv: [0.0, 1.0] },
        ];
        let clipped_sq = clip_quad_rect(quad, 0.0, 0.0, 50.0, 50.0);
        assert_eq!(clipped_sq.len, 4);
        for v in clipped_sq.slice() {
            assert!(v.pos[0] >= -0.01 && v.pos[0] <= 50.01);
            assert!(v.pos[1] >= -0.01 && v.pos[1] <= 50.01);
        }

        let clipped_circ = clip_quad_circle(quad, 50.0, 50.0, 30.0);
        assert!(clipped_circ.len >= 8);
        for v in clipped_circ.slice() {
            let d = ((v.pos[0] - 50.0).powi(2) + (v.pos[1] - 50.0).powi(2)).sqrt();
            assert!(d <= 30.1, "vertex escaped circle: {d}");
        }

        // Facing South (+Z) at pitch 0 -> point ahead at z=+20 projects near center
        let p = project_world_point([0.0, 64.0, 20.0], [0.0, 64.0, 0.0], 0.0, 0.0, 70.0, 800.0, 600.0)
            .expect("ahead");
        assert!((p.x - 400.0).abs() < 1.0 && (p.y - 300.0).abs() < 1.0);
        assert!(project_world_point([0.0, 64.0, -20.0], [0.0, 64.0, 0.0], 0.0, 0.0, 70.0, 800.0, 600.0).is_none());
    }

    #[test]
    fn terrain_tints_slope_and_cave_slicing() {
        let biome = BiomeColors::default();
        let tinted = apply_biome_tint([200, 200, 200], TintMode::Grass, biome, 10, 20);
        assert!(tinted[1] > tinted[0] && tinted[1] > tinted[2]);

        let c = ColumnCell {
            rgb: [120, 120, 120],
            height: 70,
            biome: 0,
            water_depth: 0,
            flags: ColumnCell::FLAG_EXPLORED,
        };
        let lower = ColumnCell { height: 68, ..c };
        let higher = ColumnCell { height: 72, ..c };
        let lit = shade_column_pixel(c, Some(lower), Some(lower), 0, false);
        let shadowed = shade_column_pixel(c, Some(higher), Some(higher), 0, false);
        assert!(lit[0] > shadowed[0]);

        // Cave column: roof 80..65 (stone), open air 64..51, floor 50 (deepslate)
        let mut looks = [BlockLook::default(); 3];
        looks[0].is_air = true;
        looks[1] = BlockLook {
            rgb: [128, 128, 128],
            tint: TintMode::None,
            is_air: false,
            is_fluid: false,
            is_water: false,
            is_foliage: false,
            transparent: false,
        };
        looks[2] = BlockLook { rgb: [80, 80, 90], ..looks[1] };

        let hit = scan_cave_column(0, 80, 120, |y| {
            if y >= 65 { 1 } else if y > 50 { 0 } else { 2 }
        }, &looks)
        .expect("cave floor");
        assert_eq!(hit.0, 50);
        assert_eq!(hit.1.rgb, [80, 80, 90]);
    }

    #[test]
    fn region_store_and_gpu_lru_pool() {
        let mut store = TerrainStore::new();
        let mut biomes = BiomeTable::default();
        let b_id = biomes.intern("minecraft:sunflower_plains");
        assert_eq!(b_id, 0);
        let b_bytes = biomes.encode();
        assert_eq!(BiomeTable::decode(&b_bytes).unwrap().name(0), Some("minecraft:sunflower_plains"));

        let chunk = [ColumnCell {
            rgb: [80, 140, 60],
            height: 72,
            biome: b_id,
            water_depth: 0,
            flags: ColumnCell::FLAG_EXPLORED,
        }; 256];
        store.put_surface_chunk(0, 0, &chunk);
        store.put_surface_chunk(100, 100, &chunk);

        let mut budget = 10usize;
        let uv1 = store
            .touch_and_get_uv(TileKey::SurfaceDetail { tx: 0, tz: 0 }, &mut budget)
            .expect("slot");
        assert!(uv1.u1 > uv1.u0);
        assert_eq!(store.pending_patches.len(), 1);
        let patch = &store.pending_patches[0];
        assert_eq!(patch.width, SLOT_STRIDE);
        assert_eq!(patch.rgba.len(), (SLOT_STRIDE * SLOT_STRIDE * 4) as usize);
        // Check 1px gutter extrusion matches first pixel of inner 64x64
        let inner_tl = (1 * SLOT_STRIDE as usize + 1) * 4;
        assert_eq!(&patch.rgba[0..4], &patch.rgba[inner_tl..inner_tl + 4]);
        let _ = SLOT_INNER;

        // Region encode/decode sparse round-trip
        let reg = store.regions.get(&(0, 0)).unwrap();
        let encoded = reg.encode();
        assert!(encoded.len() < 8192, "sparse region compressed cleanly: {}", encoded.len());
        let decoded = RegionBuffer::decode(&encoded).unwrap();
        assert_eq!(decoded.explored_columns, 256);
        assert_eq!(decoded.get(5, 5).height, 72);
        assert!(decoded.has_detail_tile(0, 0));
        assert!(decoded.has_mid2_tile(0, 0));
        assert!(decoded.has_mid4_tile(0, 0));

        assert!(store.touch_and_get_uv(TileKey::Mid2 { tx: 0, tz: 0 }, &mut budget).is_some());
        assert!(store.touch_and_get_uv(TileKey::Mid4 { rx: 0, rz: 0, qx: 0, qz: 0 }, &mut budget).is_some());

        // Cave mode keeps chunks in place on Y change (zero black wipe) + builds CaveMid2 cleanly
        let cave_chunk = [ColumnCell {
            rgb: [90, 90, 95],
            height: 20,
            biome: b_id,
            water_depth: 0,
            flags: ColumnCell::FLAG_EXPLORED | ColumnCell::FLAG_CAVE,
        }; 256];
        store.put_cave_chunk(0, 0, 40, cave_chunk);
        store.put_cave_chunk(1, 0, 12, cave_chunk);
        store.put_cave_chunk(2, 0, 12, cave_chunk);
        store.invalidate_cave_tiles();
        assert_eq!(store.cave_chunks.len(), 3);
        // Unloaded chunk (1,0) from stale Y=12 is pruned when active_y moves to 40, loaded (2,0) stays until rescanned
        store.prune_stale_cave_chunks(40, |cx, cz| (cx == 0 || cx == 2) && cz == 0);
        assert_eq!(store.cave_chunks.len(), 2);
        assert!(!store.cave_chunks.contains_key(&(1, 0)));
        assert!(store.touch_and_get_uv(TileKey::CaveDetail { tx: 0, tz: 0 }, &mut budget).is_some());
        assert!(store.touch_and_get_uv(TileKey::CaveMid2 { tx: 0, tz: 0 }, &mut budget).is_some());
    }

    #[test]
    fn world_map_interaction_waypoints_zoom_and_settings() {
        let cfg = ModConfig::default();
        assert_eq!(ModConfig::decode(&cfg.encode()).unwrap(), cfg);

        // Legacy ATLSCFG1 migration restores hidden World Map waypoints and old 1.0 default zoom.
        let mut legacy = cfg.encode();
        legacy[0..8].copy_from_slice(b"ATLSCFG1");
        legacy[8] &= 0xFF ^ 0x80; // clear bit 7 (show_waypoints_on_map = false)
        legacy[16..20].copy_from_slice(&1.0f32.to_le_bytes());
        let migrated = ModConfig::decode(&legacy).unwrap();
        assert!(migrated.show_waypoints_on_map);
        assert!((migrated.minimap_zoom - 0.58).abs() < 0.01);

        let rect = minimap_rect(MinimapCorner::TopLeft, cfg.minimap_size, 854.0, 480.0);
        assert_eq!(rect.outer_size, 104.0);

        EternityMapState::state_test_cycle();
    }

    #[test]
    fn entity_species_lookup_works() {
        assert_eq!(normalize_species("minecraft:zombified_piglin"), "zombified_piglin");
        assert_eq!(mob_face_spec("entity/pig/pig.png").tex_h, 64);
        assert_eq!(mob_face_spec("minecraft:zombie").pw, 8);
        let pig = mob_model_spec("minecraft:pig", 64, 64);
        assert_eq!(pig.active_parts().len(), 1);
        assert_eq!(mob_model_spec("minecraft:pig", 32, 32).base.tex_h, 32);
        assert_eq!(mob_model_spec("minecraft:chicken", 16, 16).base.tex_w, 16);
        let em = mob_model_spec("minecraft:enderman", 64, 32);
        assert_eq!(em.base.tex_h, 32);
        assert_eq!(em.active_parts()[0].source, OverlaySource::EndermanEyes);
        assert_eq!(mob_model_spec("minecraft:bat", 32, 32).base.py, 8);
        assert_eq!(mob_model_spec("minecraft:nautilus", 128, 128).active_parts().len(), 3);
    }

    #[test]
    fn world_key_and_path_sanitization_isolate_singleplayer_worlds() {
        use crate::state::resolve_world_key;

        // Horizon sets remote.session.address = "local" alongside game.integrated in singleplayer.
        let sp_a = resolve_world_key(Some("world_alpha"), Some("local"));
        let sp_b = resolve_world_key(Some("world_beta"), Some("local"));
        assert_eq!(sp_a, "sp_world_alpha");
        assert_eq!(sp_b, "sp_world_beta");
        assert_ne!(sp_a, sp_b);

        assert_eq!(
            resolve_world_key(None, Some("play.example.com:25565")),
            "srv_play.example.com:25565"
        );
        assert_eq!(resolve_world_key(Some(""), Some("local")), "srv_local");
        assert_eq!(resolve_world_key(None, None), "local");

        assert_eq!(sanitize_path_token("sp_ascii_world-1"), "sp_ascii_world-1");
        let uni_a = sanitize_path_token("sp_Новый мир 1");
        let uni_b = sanitize_path_token("sp_Второй м 2");
        let uni_c = sanitize_path_token("sp_Новый мир 2");
        assert_ne!(uni_a, uni_b);
        assert_ne!(uni_a, uni_c);
        assert!(uni_a.len() <= 48);
    }

    #[test]
    fn disk_region_merge_preserves_live_chunks_and_restores_saved_map() {
        use crate::atlas_cache::TerrainStore;
        use crate::terrain::ColumnCell;

        let mut disk_reg = RegionBuffer::new_empty();
        let mut old_chunk = [ColumnCell::default(); 256];
        old_chunk[0] = ColumnCell {
            rgb: [10, 20, 30],
            height: 64,
            biome: 1,
            water_depth: 0,
            flags: ColumnCell::FLAG_EXPLORED,
        };
        disk_reg.set_chunk_surface(0, 0, &old_chunk);
        let encoded = disk_reg.encode();
        let decoded = RegionBuffer::decode(&encoded).expect("decode saved region");

        let mut store = TerrainStore::new();
        // Simulate a freshly scanned live chunk arriving BEFORE disk load (rejoin race!).
        let mut live_chunk = [ColumnCell::default(); 256];
        live_chunk[0] = ColumnCell {
            rgb: [200, 100, 50],
            height: 70,
            biome: 2,
            water_depth: 0,
            flags: ColumnCell::FLAG_EXPLORED,
        };
        store.put_surface_chunk(1, 0, &live_chunk);

        // Now merge the disk region (0, 0): both chunk (0,0) from disk AND chunk (1,0) live must exist!
        store.merge_disk_region(0, 0, decoded);
        assert_eq!(store.get_column(0, 0).map(|c| c.rgb), Some([10, 20, 30]));
        assert_eq!(store.get_column(16, 0).map(|c| c.rgb), Some([200, 100, 50]));
    }

    #[test]
    fn pixel_mob_icons_are_well_formed_and_preserve_kept_species() {
        use crate::mob_icons::pixel_mob_icon;

        for kept in ["squid", "glow_squid", "skeleton", "wither_skeleton", "chicken", "pig", "player"] {
            assert!(pixel_mob_icon(kept).is_none(), "{kept} should keep texture icon");
        }
        let species = [
            "creeper", "zombie", "husk", "drowned", "zombie_villager", "spider", "cave_spider",
            "enderman", "slime", "magma_cube", "witch", "pillager", "evoker", "phantom",
            "silverfish", "endermite", "guardian", "elder_guardian", "shulker", "ravager",
            "warden", "breeze", "creaking", "vex", "cow", "mooshroom", "sheep", "wolf", "cat",
            "fox", "bee", "horse", "donkey", "skeleton_horse", "zombie_horse", "llama", "camel",
            "goat", "rabbit", "panda", "polar_bear", "armadillo", "sniffer", "bat", "parrot",
            "allay", "salmon", "cod", "tropical_fish", "pufferfish", "tadpole", "turtle",
            "dolphin", "axolotl", "frog", "nautilus", "villager", "wandering_trader",
            "iron_golem", "snow_golem", "copper_golem", "ghast", "blaze", "piglin",
            "zombified_piglin", "hoglin", "strider", "wither", "ender_dragon",
        ];
        for sp in species {
            let ic = pixel_mob_icon(sp).unwrap_or_else(|| panic!("missing icon for {sp}"));
            assert_eq!(ic.rows.len(), usize::from(ic.h), "{sp} height mismatch");
            for (ry, row) in ic.rows.iter().enumerate() {
                assert_eq!(row.len(), usize::from(ic.w), "{sp} row {ry} width mismatch");
                for &b in row.as_bytes() {
                    if b != b'.' {
                        assert!(ic.color_of(b).is_some(), "{sp} unknown char '{}'", b as char);
                    }
                }
            }
        }
    }
}

impl state::EternityMapState {
    #[cfg(test)]
    pub fn state_test_cycle() {
        use crate::state::{ActivePanel, ModAction, PendingCommand, UiEvent};
        use crate::ui_math::{MapButtonId, MinimapShape, WorldMapLayout};
        let mut st = Self::new();
        st.player.valid = true;
        st.player.x = 100.0;
        st.player.y = 70.0;
        st.player.z = 200.0;

        // Open world map via action
        st.actions.push_back(ModAction::ToggleWorldMap);
        st.process_world_map_events(854.0, 480.0);
        assert!(st.map.open);

        // Left click in center -> opens WaypointModal
        st.ui_events.push_back(UiEvent::MouseDown { x: 427.0, y: 240.0, button: 0 });
        st.ui_events.push_back(UiEvent::MouseUp { x: 427.0, y: 240.0, button: 0 });
        // Type Cyrillic name "Дом" and press Enter
        st.ui_events.push_back(UiEvent::CharTyped('Д'));
        st.ui_events.push_back(UiEvent::CharTyped('о'));
        st.ui_events.push_back(UiEvent::CharTyped('м'));
        st.ui_events.push_back(UiEvent::KeyDown { key: "Enter".into(), shift: false, ctrl: false });
        st.process_world_map_events(854.0, 480.0);

        assert_eq!(st.waypoints.len(), 1);
        assert_eq!(st.waypoints[0].name, "Дом");
        assert_eq!(st.waypoints[0].initial, "Д");
        assert_eq!(st.map.active_panel, ActivePanel::None);

        // Preload button click -> queues ForceRescanLoaded + RequestPreload
        let layout = WorldMapLayout::compute(854.0, 480.0);
        let pre = layout.button_rect(MapButtonId::PreloadRegion).unwrap();
        st.ui_events.push_back(UiEvent::MouseDown { x: pre.x + 4.0, y: pre.y + 4.0, button: 0 });
        st.ui_events.push_back(UiEvent::MouseUp { x: pre.x + 4.0, y: pre.y + 4.0, button: 0 });
        st.process_world_map_events(854.0, 480.0);
        assert!(st.commands.iter().any(|c| matches!(c, PendingCommand::RequestPreload { .. })));

        // Right-click opens context menu -> escape closes it cleanly before testing Cave switcher
        st.ui_events.push_back(UiEvent::MouseDown { x: 500.0, y: 300.0, button: 1 });
        st.process_world_map_events(854.0, 480.0);
        assert_eq!(st.map.active_panel, ActivePanel::ContextMenu);
        st.map.active_panel = ActivePanel::None;

        // Cave mode toolbar button toggles open/closed ONLY (never switches mode!)
        let cb = layout.button_rect(MapButtonId::CaveMode).unwrap();
        st.ui_events.push_back(UiEvent::MouseDown { x: cb.x + 5.0, y: cb.y + 5.0, button: 0 });
        st.ui_events.push_back(UiEvent::MouseUp { x: cb.x + 5.0, y: cb.y + 5.0, button: 0 });
        st.process_world_map_events(854.0, 480.0);
        assert!(st.map.cave_slider_open);
        assert_eq!(st.config.cave_mode, crate::terrain::CaveMode::Auto);

        let tr = layout.cave_slider_track;
        // Click [Manual] pill -> Manual, [Auto] -> Auto, [Off] -> Off (tab remains open until CaveMode clicked)
        st.ui_events.push_back(UiEvent::MouseDown { x: tr.x + tr.w * 0.5, y: tr.y + 8.0, button: 0 });
        st.ui_events.push_back(UiEvent::MouseUp { x: tr.x + tr.w * 0.5, y: tr.y + 8.0, button: 0 });
        st.process_world_map_events(854.0, 480.0);
        assert_eq!(st.config.cave_mode, crate::terrain::CaveMode::Manual);

        st.ui_events.push_back(UiEvent::MouseDown { x: tr.x + 15.0, y: tr.y + 8.0, button: 0 });
        st.ui_events.push_back(UiEvent::MouseUp { x: tr.x + 15.0, y: tr.y + 8.0, button: 0 });
        st.process_world_map_events(854.0, 480.0);
        assert_eq!(st.config.cave_mode, crate::terrain::CaveMode::Auto);
        assert!(st.map.cave_slider_open);

        st.ui_events.push_back(UiEvent::MouseDown { x: tr.x + tr.w - 15.0, y: tr.y + 8.0, button: 0 });
        st.ui_events.push_back(UiEvent::MouseUp { x: tr.x + tr.w - 15.0, y: tr.y + 8.0, button: 0 });
        st.process_world_map_events(854.0, 480.0);
        assert_eq!(st.config.cave_mode, crate::terrain::CaveMode::Off);
        assert!(st.map.cave_slider_open);

        // Clicking CaveMode toolbar icon closes the tab WITHOUT switching CaveMode back on
        st.ui_events.push_back(UiEvent::MouseDown { x: cb.x + 5.0, y: cb.y + 5.0, button: 0 });
        st.ui_events.push_back(UiEvent::MouseUp { x: cb.x + 5.0, y: cb.y + 5.0, button: 0 });
        st.process_world_map_events(854.0, 480.0);
        assert!(!st.map.cave_slider_open);
        assert_eq!(st.config.cave_mode, crate::terrain::CaveMode::Off);

        let _ = MinimapShape::Round;
    }
}
