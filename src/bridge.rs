//! Engine bridge inside `horizon_client::game` (`mod eternity_world_map_bridge`):
//! zero-allocation GPU tile rendering, budgeted chunk/cave scanning, background region
//! preloading, entity + Tab-head radar, 3D waypoints, and the fullscreen World Map (`M`).

use super::entity::Entity as _;
use super::{Game, Screen};
use horizon_client::blocks::Fluid;
use horizon_client::font::Patch;
use horizon_client::hud::{HudAtlas, HudBatch, HudVertex, MOD_IMAGE_SPRITE, Rect, rgba};
use horizon_client::streaming::ChunkGenerator as _;
use horizon_client::vanillagen::VanillaTerrain;
use horizon_items::blocks::MAP_COLORS;
use horizon_world::registry::BlockStateId;
use mod_eternity_world_map::atlas_cache::{
    BiomeTable, REGION_SIZE, TILE_BLOCKS, TILE_CHUNKS, TileKey, floor_div,
};
use mod_eternity_world_map::entities::{
    EntityCategory, MobPart, OverlaySource, RadarEntity, mob_model_spec, normalize_species,
};
use mod_eternity_world_map::state::{
    ActivePanel, ModalField, PendingCommand, PreloadState, resolve_world_key, state,
};
use mod_eternity_world_map::terrain::{
    BiomeColors, BlockLook, CaveMode, ColumnCell, TintMode, apply_biome_tint, scan_cave_column,
};
use mod_eternity_world_map::ui_math::{
    ClipVertex, MapButtonId, MinimapShape, UiRect, WorldMapLayout, clip_quad_circle,
    clip_quad_rect, minimap_rect, world_to_minimap,
};
use mod_eternity_world_map::waypoints::{
    PALETTE, clamp_to_circle, clamp_to_square, contrast_fg, project_world_point,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

struct PreloadJob {
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU32>,
    total: u32,
}

struct BridgeRuntime {
    assets_generation: u64,
    block_looks: Vec<BlockLook>,
    scanned_chunk_ptrs: HashMap<(i32, i32), usize>,
    cave_scanned_ptrs: HashMap<(i32, i32), (usize, i32)>,
    chunk_queue: Vec<(i32, i32)>,
    chunk_queue_player: (i32, i32),
    chunk_queue_len: usize,
    biome_cache: BiomeTable,
    biome_world_key: String,
    frame_count: u64,
    last_world_key: String,
    last_dim_id: String,
    preload_job: Option<PreloadJob>,
    russian_ui: bool,
}

impl BridgeRuntime {
    fn new() -> Self {
        Self {
            assets_generation: u64::MAX,
            block_looks: Vec::new(),
            scanned_chunk_ptrs: HashMap::new(),
            cave_scanned_ptrs: HashMap::new(),
            chunk_queue: Vec::new(),
            chunk_queue_player: (i32::MAX, i32::MAX),
            chunk_queue_len: usize::MAX,
            biome_cache: BiomeTable::default(),
            biome_world_key: String::new(),
            frame_count: 0,
            last_world_key: String::new(),
            last_dim_id: String::new(),
            preload_job: None,
            russian_ui: false,
        }
    }
}

static RUNTIME: OnceLock<Mutex<BridgeRuntime>> = OnceLock::new();

fn runtime() -> &'static Mutex<BridgeRuntime> {
    RUNTIME.get_or_init(|| Mutex::new(BridgeRuntime::new()))
}

#[inline]
fn hud_color(argb: u32) -> u32 {
    let [a, r, g, b] = argb.to_be_bytes();
    rgba(r, g, b, a)
}

#[inline]
fn white_uv(atlas: &HudAtlas) -> [f32; 2] {
    let w = atlas.white;
    [(w.u0 + w.u1) * 0.5, (w.v0 + w.v1) * 0.5]
}

fn sub_rect(r: Rect, px: f32, py: f32, pw: f32, ph: f32, tw: f32, th: f32) -> Rect {
    let du = r.u1 - r.u0;
    let dv = r.v1 - r.v0;
    Rect {
        u0: r.u0 + du * (px / tw),
        v0: r.v0 + dv * (py / th),
        u1: r.u0 + du * ((px + pw) / tw),
        v1: r.v0 + dv * ((py + ph) / th),
        w: pw.round() as u32,
        h: ph.round() as u32,
    }
}

fn build_block_palette(game: &Game<'_>) -> Vec<BlockLook> {
    let facts = game.res.facts();
    let render = &game.pack.render;
    let raw_pixels = render.pixels.get();
    let side = render.sprite_size.max(1) as usize;
    let layers = render.texture_layers as usize;
    let layer_bytes = side * side * 4;

    let mut layer_avg: Vec<[u8; 3]> = Vec::with_capacity(layers);
    for l in 0..layers {
        let start = l * layer_bytes;
        let end = start + layer_bytes;
        if end > raw_pixels.len() {
            layer_avg.push([128, 128, 128]);
            continue;
        }
        let mut sr = 0u64;
        let mut sg = 0u64;
        let mut sb = 0u64;
        let mut count = 0u64;
        for px in raw_pixels[start..end].chunks_exact(4) {
            let a = u64::from(px[3]);
            if a >= 32 {
                sr += u64::from(px[0]) * a;
                sg += u64::from(px[1]) * a;
                sb += u64::from(px[2]) * a;
                count += a;
            }
        }
        if count == 0 {
            layer_avg.push([128, 128, 128]);
        } else {
            layer_avg.push([
                (sr / count).min(255) as u8,
                (sg / count).min(255) as u8,
                (sb / count).min(255) as u8,
            ]);
        }
    }

    let mut out = Vec::with_capacity(facts.states.len());
    for i in 0..facts.states.len() {
        let id = BlockStateId(i as u32);
        let sf = facts.get(id);
        let is_air = facts.is_air(id);
        if is_air {
            out.push(BlockLook::default());
            continue;
        }

        let name: &str = &sf.name;
        let is_water = sf.fluid == Fluid::Water
            || name == "minecraft:water"
            || name == "minecraft:kelp"
            || name == "minecraft:kelp_plant"
            || name == "minecraft:seagrass"
            || name == "minecraft:tall_seagrass"
            || name == "minecraft:bubble_column";
        let is_fluid = sf.fluid != Fluid::None || is_water;
        let is_foliage = name.ends_with("_leaves")
            || name.contains("vine")
            || name == "minecraft:sugar_cane";

        let map_idx = usize::from(sf.map_color).min(MAP_COLORS.len() - 1);
        let map_col = MAP_COLORS[map_idx];
        let map_rgb = [
            ((map_col >> 16) & 0xFF) as u8,
            ((map_col >> 8) & 0xFF) as u8,
            (map_col & 0xFF) as u8,
        ];

        let mut picked_rgb = map_rgb;
        let mut tint = TintMode::None;

        if is_water {
            picked_rgb = [152, 168, 204];
            tint = TintMode::Water;
        } else if let Ok(models) = render.model_catalog.collect_seeded(id, 0) {
            // Prefer upward-facing face first so grass tops show grass green rather than dirt side!
            let mut best_face = None;
            'outer: for &(_, model) in &models {
                for face in &model.faces {
                    if matches!(face.quad.facing, horizon_assets::model::Direction::Up) {
                        best_face = Some(face);
                        break 'outer;
                    }
                    if best_face.is_none() {
                        best_face = Some(face);
                    }
                }
            }
            if let Some(face) = best_face {
                if let Some(&avg) = layer_avg.get(face.layer as usize) {
                    picked_rgb = avg;
                }
                if face.quad.tint_index >= 0 {
                    tint = if name.contains("leaf_litter") || name.contains("dry_") || name.contains("wildflowers") {
                        TintMode::DryFoliage
                    } else if is_foliage {
                        TintMode::Foliage
                    } else {
                        TintMode::Grass
                    };
                }
            }
        }

        let transparent = !sf.solid_render
            && !is_fluid
            && !sf.full_collision
            && !is_foliage
            && !name.ends_with("_slab")
            && !name.ends_with("_stairs")
            && !name.contains("path")
            && !name.contains("farmland")
            && !name.contains("carpet")
            && !name.contains("snow");

        out.push(BlockLook {
            rgb: picked_rgb,
            tint,
            is_air: false,
            is_fluid,
            is_water,
            is_foliage,
            transparent,
        });
    }
    out
}

fn biome_colors_of(biomes: &horizon_client::biome::Biomes, id: u16) -> BiomeColors {
    let Some(b) = biomes.list.get(usize::from(id)) else {
        return BiomeColors::default();
    };
    let grass_mod = match b.modifier {
        horizon_client::biome::GrassModifier::Swamp => 1,
        horizon_client::biome::GrassModifier::DarkForest => 2,
        horizon_client::biome::GrassModifier::None => 0,
    };
    BiomeColors {
        grass: b.grass,
        foliage: b.foliage,
        dry_foliage: b.dry_foliage,
        water: b.water,
        grass_mod,
    }
}

fn intern_biome_id(
    engine_biomes: &horizon_client::biome::Biomes,
    raw_id: u16,
    table: &mut BiomeTable,
    dirty_flag: &mut bool,
) -> u16 {
    let name: &str = engine_biomes
        .list
        .get(usize::from(raw_id))
        .map_or("minecraft:plains", |b| &b.name);
    let existed = table.lookup_id(name);
    let id = table.intern(name);
    if existed.is_none() {
        *dirty_flag = true;
    }
    id
}

fn scan_chunk_surface(
    chunk: &horizon_client::chunk::Chunk,
    looks: &[BlockLook],
    engine_biomes: &horizon_client::biome::Biomes,
    biome_table: &mut mod_eternity_world_map::atlas_cache::BiomeTable,
    biomes_dirty: &mut bool,
    air_id: BlockStateId,
    min_y: i32,
    max_y: i32,
    realistic_colors: bool,
    has_ceiling: bool,
) -> [ColumnCell; 256] {
    let mut cells = [ColumnCell::default(); 256];
    let sections_top_y = min_y + (chunk.sections.len() as i32) * 16 - 1;
    let start_y = if has_ceiling {
        (min_y + 120).min(max_y - 1).min(sections_top_y)
    } else {
        (max_y - 1).min(sections_top_y)
    };

    // Fast section bitmap skips completely empty air sections above terrain!
    let mut col_top_sec = chunk.sections.len() as i32 - 1;
    while col_top_sec > 0 {
        let s = &chunk.sections[col_top_sec as usize];
        if !s.is_uniform(air_id.0) {
            break;
        }
        col_top_sec -= 1;
    }
    let effective_top = start_y.min(min_y + (col_top_sec + 1) * 16 - 1);

    for lz in 0..16i32 {
        for lx in 0..16i32 {
            let wx = chunk.x * 16 + lx;
            let wz = chunk.z * 16 + lz;
            let idx = (lz * 16 + lx) as usize;

            if has_ceiling {
                // Nether/ceiling world: slice down past initial bedrock/netherrack roof.
                let hit = scan_cave_column(
                    min_y,
                    effective_top,
                    max_y,
                    |y| chunk.get(lx, y, lz),
                    looks,
                );
                if let Some((hy, look, depth)) = hit {
                    let raw_b = chunk.biome(lx, hy, lz);
                    let bc = biome_colors_of(engine_biomes, raw_b);
                    let rgb = if realistic_colors {
                        apply_biome_tint(look.rgb, look.tint, bc, wx, wz)
                    } else {
                        look.rgb
                    };
                    let b_intern = intern_biome_id(engine_biomes, raw_b, biome_table, biomes_dirty);
                    cells[idx] = ColumnCell {
                        rgb,
                        height: hy as i16,
                        biome: b_intern,
                        water_depth: depth,
                        flags: ColumnCell::FLAG_EXPLORED
                            | if look.is_water { ColumnCell::FLAG_WATER } else { 0 },
                    };
                }
                continue;
            }

            let mut y = effective_top;
            let mut water_y: Option<i32> = None;
            while y >= min_y {
                let sec_idx = ((y - min_y) >> 4) as usize;
                if let Some(sec) = chunk.sections.get(sec_idx)
                    && sec.is_uniform(air_id.0)
                {
                    y = min_y + ((sec_idx as i32) << 4) - 1;
                    continue;
                }
                let st_id = chunk.get(lx, y, lz) as usize;
                let look = looks.get(st_id).copied().unwrap_or_default();
                if look.is_air {
                    y -= 1;
                    continue;
                }
                if look.is_water {
                    if water_y.is_none() {
                        water_y = Some(y);
                    }
                    y -= 1;
                    continue;
                }
                if look.transparent && water_y.is_none() {
                    // Thin grass/flower/torch overlays tint slightly over solid floor unless alone.
                    y -= 1;
                    continue;
                }

                let raw_b = chunk.biome(lx, water_y.unwrap_or(y), lz);
                let bc = biome_colors_of(engine_biomes, raw_b);
                let b_intern = intern_biome_id(engine_biomes, raw_b, biome_table, biomes_dirty);
                if let Some(wy) = water_y {
                    let depth = (wy - y).clamp(1, 255) as u8;
                    let w_rgb = apply_biome_tint([152, 168, 204], TintMode::Water, bc, wx, wz);
                    // Blend shallow shore riverbed tint slightly with water tint for crisp coastlines.
                    let bed = apply_biome_tint(look.rgb, look.tint, bc, wx, wz);
                    let w_mix = (f32::from(depth) * 0.14).clamp(0.50, 0.94);
                    let rgb = [
                        (f32::from(w_rgb[0]) * w_mix + f32::from(bed[0]) * (1.0 - w_mix)) as u8,
                        (f32::from(w_rgb[1]) * w_mix + f32::from(bed[1]) * (1.0 - w_mix)) as u8,
                        (f32::from(w_rgb[2]) * w_mix + f32::from(bed[2]) * (1.0 - w_mix)) as u8,
                    ];
                    cells[idx] = ColumnCell {
                        rgb,
                        height: wy as i16,
                        biome: b_intern,
                        water_depth: depth,
                        flags: ColumnCell::FLAG_EXPLORED | ColumnCell::FLAG_WATER,
                    };
                } else {
                    let rgb = if realistic_colors {
                        apply_biome_tint(look.rgb, look.tint, bc, wx, wz)
                    } else {
                        look.rgb
                    };
                    cells[idx] = ColumnCell {
                        rgb,
                        height: y as i16,
                        biome: b_intern,
                        water_depth: 0,
                        flags: ColumnCell::FLAG_EXPLORED,
                    };
                }
                break;
            }
            if !cells[idx].is_explored()
                && let Some(wy) = water_y
            {
                let raw_b = chunk.biome(lx, wy, lz);
                let bc = biome_colors_of(engine_biomes, raw_b);
                let b_intern = intern_biome_id(engine_biomes, raw_b, biome_table, biomes_dirty);
                let rgb = apply_biome_tint([152, 168, 204], TintMode::Water, bc, wx, wz);
                cells[idx] = ColumnCell {
                    rgb,
                    height: wy as i16,
                    biome: b_intern,
                    water_depth: 16,
                    flags: ColumnCell::FLAG_EXPLORED | ColumnCell::FLAG_WATER,
                };
            }
        }
    }
    cells
}

fn scan_chunk_cave(
    chunk: &horizon_client::chunk::Chunk,
    looks: &[BlockLook],
    engine_biomes: &horizon_client::biome::Biomes,
    biome_table: &mut mod_eternity_world_map::atlas_cache::BiomeTable,
    biomes_dirty: &mut bool,
    air_id: BlockStateId,
    min_y: i32,
    max_y: i32,
    cave_y: i32,
) -> [ColumnCell; 256] {
    let mut cells = [ColumnCell::default(); 256];
    let ceiling_y = cave_y.clamp(min_y, max_y - 1);
    let sections_top_y = min_y + (chunk.sections.len() as i32) * 16 - 1;
    let mut col_top_sec = chunk.sections.len() as i32 - 1;
    while col_top_sec > 0 {
        if !chunk.sections[col_top_sec as usize].is_uniform(air_id.0) {
            break;
        }
        col_top_sec -= 1;
    }
    let effective_top = (max_y - 1)
        .min(sections_top_y)
        .min(min_y + (col_top_sec + 1) * 16 - 1);
    // If every non-air block is strictly below ceiling_y, open sky has no cave roof here.
    if effective_top < ceiling_y {
        return cells;
    }
    for lz in 0..16i32 {
        for lx in 0..16i32 {
            let mut has_roof = false;
            let mut ry = effective_top;
            while ry >= ceiling_y {
                let sec_idx = ((ry - min_y) >> 4) as usize;
                if let Some(sec) = chunk.sections.get(sec_idx)
                    && sec.is_uniform(air_id.0)
                {
                    ry = min_y + ((sec_idx as i32) << 4) - 1;
                    continue;
                }
                let st_id = chunk.get(lx, ry, lz) as usize;
                let look = looks.get(st_id).copied().unwrap_or_default();
                if !look.is_air && !look.is_water && !look.is_foliage && !look.transparent {
                    has_roof = true;
                    break;
                }
                ry -= 1;
            }
            if !has_roof {
                continue;
            }
            let wx = chunk.x * 16 + lx;
            let wz = chunk.z * 16 + lz;
            let idx = (lz * 16 + lx) as usize;
            if let Some((hy, look, depth)) =
                scan_cave_column(min_y, ceiling_y, max_y, |y| chunk.get(lx, y, lz), looks)
            {
                let raw_b = chunk.biome(lx, hy, lz);
                let bc = biome_colors_of(engine_biomes, raw_b);
                let b_intern = intern_biome_id(engine_biomes, raw_b, biome_table, biomes_dirty);
                let rgb = apply_biome_tint(look.rgb, look.tint, bc, wx, wz);
                let is_real_cavity = !look.transparent || look.is_foliage || look.is_water;
                cells[idx] = ColumnCell {
                    rgb,
                    height: hy as i16,
                    biome: b_intern,
                    water_depth: depth,
                    flags: ColumnCell::FLAG_EXPLORED
                        | if is_real_cavity { ColumnCell::FLAG_CAVE } else { 0 }
                        | if look.is_water { ColumnCell::FLAG_WATER } else { 0 },
                };
            }
        }
    }
    cells
}

fn spawn_background_preload(
    terrain: Arc<VanillaTerrain>,
    looks: Vec<BlockLook>,
    engine_biomes: Arc<horizon_client::biome::Biomes>,
    center_x: i32,
    center_z: i32,
    radius_chunks: i32,
    dim_id: String,
    realistic_colors: bool,
) -> PreloadJob {
    let c_cx = floor_div(center_x, 16);
    let c_cz = floor_div(center_z, 16);
    let side = (radius_chunks * 2 + 1) as u32;
    let total = side * side;
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicU32::new(0));
    let cancel_t = Arc::clone(&cancel);
    let done_t = Arc::clone(&done);

    std::thread::Builder::new()
        .name("eternity_world_map_preload".into())
        .spawn(move || {
            let mut sampler = terrain.lod_sampler();
            let mut count = 0u32;
            // Spiral outward from map center so immediate surrounding chunks paint in milliseconds.
            let mut coords: Vec<(i32, i32)> = Vec::with_capacity(total as usize);
            for dz in -radius_chunks..=radius_chunks {
                for dx in -radius_chunks..=radius_chunks {
                    coords.push((c_cx + dx, c_cz + dz));
                }
            }
            coords.sort_by_key(|&(cx, cz)| {
                let dx = cx - c_cx;
                let dz = cz - c_cz;
                dx * dx + dz * dz
            });

            for (cx, cz) in coords {
                if cancel_t.load(Ordering::Relaxed) {
                    break;
                }
                // Check quickly if chunk is already mostly explored so preloading skips existing ground.
                let already_explored = {
                    let st = state().lock().unwrap_or_else(|e| e.into_inner());
                    st.terrain
                        .get_column(cx * 16 + 8, cz * 16 + 8)
                        .is_some_and(|c| c.is_explored())
                        && st
                            .terrain
                            .get_column(cx * 16 + 1, cz * 16 + 1)
                            .is_some_and(|c| c.is_explored())
                };
                if already_explored {
                    count += 1;
                    done_t.store(count, Ordering::Relaxed);
                    continue;
                }

                // Sample 8x8 grid per chunk (2-block resolution) -> ~0.35s for a full 512x512 region!
                let mut cells = [ColumnCell::default(); 256];
                let mut temp_biomes = Vec::new();
                for gz in 0..8i32 {
                    for gx in 0..8i32 {
                        let wx = cx * 16 + gx * 2;
                        let wz = cz * 16 + gz * 2;
                        if let Some(surf) = sampler.sample(wx, wz) {
                            let (h, st_id, is_water, water_depth) = match surf.water {
                                Some((wy, wst)) => (
                                    wy,
                                    wst,
                                    true,
                                    (wy - surf.height).clamp(1, 255) as u8,
                                ),
                                None => (surf.height, surf.state, false, 0),
                            };
                            let look = looks.get(st_id.0 as usize).copied().unwrap_or_default();
                            let bc = biome_colors_of(&engine_biomes, surf.biome);
                            let rgb = if realistic_colors {
                                let base = if is_water { [152, 168, 204] } else { look.rgb };
                                let mode = if is_water { TintMode::Water } else { look.tint };
                                apply_biome_tint(base, mode, bc, wx, wz)
                            } else {
                                look.rgb
                            };
                            let b_name: &str = engine_biomes
                                .list
                                .get(usize::from(surf.biome))
                                .map_or("minecraft:plains", |b| &b.name);
                            let b_local_idx = match temp_biomes.iter().position(|&n| n == b_name) {
                                Some(i) => i as u16,
                                None => {
                                    temp_biomes.push(b_name);
                                    (temp_biomes.len() - 1) as u16
                                }
                            };
                            let c = ColumnCell {
                                rgb,
                                height: h as i16,
                                biome: b_local_idx,
                                water_depth,
                                flags: ColumnCell::FLAG_EXPLORED
                                    | if is_water { ColumnCell::FLAG_WATER } else { 0 },
                            };
                            for dz in 0..2 {
                                for dx in 0..2 {
                                    let idx = ((gz * 2 + dz) * 16 + (gx * 2 + dx)) as usize;
                                    cells[idx] = c;
                                }
                            }
                        }
                    }
                }

                {
                    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
                    if st.terrain.dimension != dim_id {
                        break;
                    }
                    let remapped: Vec<u16> = temp_biomes
                        .iter()
                        .map(|n| st.terrain.biomes.intern(n))
                        .collect();
                    st.biomes_dirty = true;
                    for c in &mut cells {
                        if let Some(&mapped) = remapped.get(usize::from(c.biome)) {
                            c.biome = mapped;
                        }
                    }
                    st.terrain.put_surface_chunk(cx, cz, &cells);
                }
                count += 1;
                done_t.store(count, Ordering::Relaxed);
            }
            let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
            st.map.preload.active = false;
        })
        .ok();

    PreloadJob {
        cancel,
        done,
        total,
    }
}

fn localize_biome_name(biome_id: &str) -> String {
    if let Some((ns, path)) = biome_id.split_once(':') {
        let key = format!("biome.{ns}.{path}");
        let fallback = path.replace('_', " ");
        horizon_client::lang::get_or(&key, &fallback)
    } else {
        biome_id.to_owned()
    }
}

/// Execute once per frame in `extras_frame`: sync player/entities/world, budget-scan chunks, handle preload.
pub fn on_extras_frame(game: &mut Game<'_>) {
    let mut rt = runtime().lock().unwrap_or_else(|e| e.into_inner());

    // Language check (cached via translation lookup)
    rt.russian_ui = horizon_client::lang::get_or("eternity_world_map.death", "Death") == "Смерть";

    // Rebuild block texture palette ONLY when resource packs reload.
    if game.pack.render.generation != rt.assets_generation || rt.block_looks.is_empty() {
        rt.assets_generation = game.pack.render.generation;
        rt.block_looks = build_block_palette(game);
        rt.scanned_chunk_ptrs.clear();
        rt.cave_scanned_ptrs.clear();
    }

    let world_key = resolve_world_key(
        game.integrated.as_ref().map(|integ| integ.save_folder.as_str()),
        game.remote.as_ref().map(|remote| remote.session.address.as_str()),
    );
    let dim_id = game.dim.id().to_owned();

    rt.frame_count = rt.frame_count.wrapping_add(1);
    if rt.last_world_key != world_key || rt.last_dim_id != dim_id {
        if let Some(job) = rt.preload_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        rt.last_world_key = world_key.clone();
        rt.last_dim_id = dim_id.clone();
        rt.scanned_chunk_ptrs.clear();
        rt.cave_scanned_ptrs.clear();
        rt.chunk_queue.clear();
        rt.chunk_queue_len = usize::MAX;
        rt.chunk_queue_player = (i32::MAX, i32::MAX);
    }

    let (gui_w, gui_h) = game.gui_size();
    let tab_held = game
        .keys
        .held(horizon_client::actions::Action::Playerlist)
        || game
            .physical_keys
            .contains(&winit::keyboard::KeyCode::Tab);

    let remote_tab = if tab_held && game.remote.is_some() {
        game.remote_tab_players().map(|(list, _, _)| list)
    } else {
        None
    };

    let (px, py, pz, eye_h, yaw_deg, pitch_deg, valid_player) = match &game.player {
        Some(p) => (
            p.position.x,
            p.position.y,
            p.position.z,
            p.dimensions.eye_height as f64,
            game.yaw,
            game.pitch,
            true,
        ),
        None => (0.0, 64.0, 0.0, 1.62, 0.0, 0.0, false),
    };
    let fov_deg = if valid_player {
        game.fov_degrees(0.0, [px, py + eye_h, pz])
    } else {
        70.0
    };

    // Determine whether player stands under a ceiling (for CaveMode::Auto).
    let mut under_roof = game.world.shape.has_ceiling;
    if !under_roof && valid_player {
        let bx = px.floor() as i32;
        let bz = pz.floor() as i32;
        let by = (py + eye_h + 1.0).floor() as i32;
        let max_y = game.world.shape.max_y();
        if let Some(ch) = game
            .world
            .chunks
            .get(&(floor_div(bx, 16), floor_div(bz, 16)))
        {
            let lx = bx & 15;
            let lz = bz & 15;
            let mut solid_above = 0;
            for y in by..max_y {
                let st_id = ch.get(lx, y, lz) as usize;
                let look = rt.block_looks.get(st_id).copied().unwrap_or_default();
                if !look.is_air && !look.transparent && !look.is_foliage {
                    solid_above += 1;
                    if solid_above >= 2 {
                        under_roof = true;
                        break;
                    }
                }
            }
        }
    }

    // Snapshot nearby radar entities cleanly without holding long locks.
    let mut entities_snap: Vec<RadarEntity> = Vec::with_capacity(64);
    let max_rad_sq = 128.0 * 128.0;
    if valid_player {
        let remote_names: HashMap<u32, (u128, String, String)> = remote_tab
            .as_ref()
            .zip(game.remote.as_ref())
            .map(|(tabs, rem)| {
                let by_uuid: HashMap<u128, (&str, &str)> = tabs
                    .iter()
                    .map(|t| (t.uuid, (t.name.as_str(), t.skin.as_str())))
                    .collect();
                rem.level_players
                    .iter()
                    .filter_map(|(&eid, &uuid)| {
                        by_uuid.get(&uuid).map(|&(name, skin)| {
                            (eid as u32, (uuid, name.to_owned(), skin.to_owned()))
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        for mob in &game.mobs {
            if mob.removed || !mob.alive() {
                continue;
            }
            let pos = mob.position();
            let dx = pos.x - px;
            let dz = pos.z - pz;
            if dx * dx + dz * dz > max_rad_sq {
                continue;
            }
            let is_player = mob.kind.id == "minecraft:player";
            let (uuid, display_name, tex, species_key) = if tab_held {
                let (u, n, mut t) = if let Some((u, n, sk)) = remote_names.get(&mob.id) {
                    (*u, n.clone(), sk.clone())
                } else {
                    let fallback_name = mob
                        .custom_name
                        .clone()
                        .unwrap_or_else(|| mob.kind.id.clone());
                    let raw_tex = mob
                        .skin
                        .clone()
                        .unwrap_or_else(|| mob.kind.texture.clone());
                    (0u128, fallback_name, raw_tex)
                };
                if !is_player {
                    t = t
                        .strip_prefix("textures/")
                        .unwrap_or(&t)
                        .strip_suffix(".png")
                        .unwrap_or(&t)
                        .to_owned();
                }
                (u, n, t, normalize_species(&mob.kind.id).to_owned())
            } else {
                (0u128, String::new(), String::new(), String::new())
            };
            entities_snap.push(RadarEntity {
                x: pos.x,
                z: pos.z,
                yaw_deg: mob.yaw,
                category: if is_player {
                    EntityCategory::Player
                } else {
                    EntityCategory::Mob
                },
                texture_key: tex,
                species_key,
                display_name,
                uuid,
                is_local: false,
            });
        }
        for boat in &game.boats {
            if boat.is_removed() {
                continue;
            }
            let pos = boat.position();
            let dx = pos.x - px;
            let dz = pos.z - pz;
            if dx * dx + dz * dz <= max_rad_sq {
                entities_snap.push(RadarEntity {
                    x: pos.x,
                    z: pos.z,
                    yaw_deg: 0.0,
                    category: EntityCategory::Vehicle,
                    texture_key: String::new(),
                    species_key: String::new(),
                    display_name: String::new(),
                    uuid: 0,
                    is_local: false,
                });
            }
        }
        for cart in &game.carts {
            if cart.is_removed() {
                continue;
            }
            let pos = cart.position();
            let dx = pos.x - px;
            let dz = pos.z - pz;
            if dx * dx + dz * dz <= max_rad_sq {
                entities_snap.push(RadarEntity {
                    x: pos.x,
                    z: pos.z,
                    yaw_deg: 0.0,
                    category: EntityCategory::Vehicle,
                    texture_key: String::new(),
                    species_key: String::new(),
                    display_name: String::new(),
                    uuid: 0,
                    is_local: false,
                });
            }
        }
    }

    let world_map_key = game
        .mod_key("eternity_world_map", "world_map")
        .map(|(k, _)| k);

    // Update shared EternityMapState + process World Map UI inputs.
    let mut pending_cmds = Vec::new();
    let (cave_active, cave_y, realistic_colors, force_rescan, map_open) = {
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = world_map_key {
            st.world_map_key = k;
        }
        st.terrain.frame_seq = st.terrain.frame_seq.wrapping_add(1);
        st.terrain.switch_world_dim(world_key.clone(), dim_id.clone());
        st.config.russian_lang = rt.russian_ui;

        let is_dead = valid_player && game.screen == Screen::Dead;
        if is_dead && !st.was_dead {
            st.add_death_waypoint(px.floor() as i32, py.floor() as i32, pz.floor() as i32);
        }
        st.was_dead = is_dead;

        st.player.valid = valid_player;
        st.player.x = px;
        st.player.y = py;
        st.player.z = pz;
        st.player.eye_height = eye_h;
        st.player.yaw_deg = yaw_deg;
        st.player.pitch_deg = pitch_deg;
        st.player.fov_deg = fov_deg;
        st.player.screen_w = gui_w;
        st.player.screen_h = gui_h;
        st.update_auto_cave_state(under_roof, py);
        st.player.tab_held = tab_held;
        st.player.dead = is_dead;
        st.entities = entities_snap;

        if st.map.open && st.map.follow_player && valid_player {
            st.map.center_x = px;
            st.map.center_z = pz;
        }
        st.cursor_x = game.cursor.0;
        st.cursor_y = game.cursor.1;

        st.process_world_map_events(gui_w, gui_h);

        // Update hovered world coordinates + biome info on World Map
        if st.map.open {
            let (hx, hz) = st.screen_to_world(st.cursor_x, st.cursor_y, gui_w, gui_h);
            let hbx = hx.floor() as i32;
            let hbz = hz.floor() as i32;
            st.map.hovered_block = [hbx, hbz];
            if let Some(cell) = st.terrain.get_column(hbx, hbz).filter(|c| c.is_explored()) {
                st.map.hovered_y = Some(cell.height);
                let bid = st
                    .terrain
                    .biomes
                    .name(cell.biome)
                    .unwrap_or("minecraft:plains")
                    .to_owned();
                if st.map.hovered_biome_id != bid {
                    st.map.hovered_biome_name = localize_biome_name(&bid);
                    st.map.hovered_biome_id = bid;
                }
            } else {
                st.map.hovered_y = None;
                st.map.hovered_biome_id.clear();
                st.map.hovered_biome_name.clear();
            }
        }

        if let Some(job) = &rt.preload_job {
            st.map.preload.completed = job.done.load(Ordering::Relaxed);
            st.map.preload.total = job.total;
            if st.map.preload.completed >= st.map.preload.total {
                st.map.preload.active = false;
            }
        }
        if let Some((_, expire)) = st.map.toast
            && st.tick_counter > expire
        {
            st.map.toast = None;
        }

        let mut force_rescan = false;
        while let Some(cmd) = st.commands.pop_front() {
            if matches!(cmd, PendingCommand::ForceRescanLoaded) {
                force_rescan = true;
            } else {
                pending_cmds.push(cmd);
            }
        }

        let (ca, cy) = st.effective_cave_mode();
        let cave_y_changed = ca && (st.terrain.last_cave_y != cy || !st.terrain.last_cave_active);
        if ca && cave_y_changed {
            st.terrain.last_cave_y = cy;
            st.terrain.last_cave_active = true;
            st.terrain
                .prune_stale_cave_chunks(cy, |cx, cz| game.world.chunks.contains_key(&(cx, cz)));
        } else if !ca {
            st.terrain.last_cave_active = false;
        }

        (
            ca,
            cy,
            st.config.realistic_colors,
            force_rescan,
            st.map.open,
        )
    };

    if force_rescan {
        rt.scanned_chunk_ptrs.clear();
        rt.cave_scanned_ptrs.clear();
    }

    for cmd in pending_cmds {
        match cmd {
            PendingCommand::Teleport { x, y, z } => {
                game.handle_chat_input(&format!("/tp {x} {y} {z}"));
            }
            PendingCommand::CopyClipboard(txt) => {
                game.clipboard.set_text(&txt);
            }
            PendingCommand::RequestPreload {
                center_x,
                center_z,
                radius_chunks,
            } => {
                if let Some(prev) = rt.preload_job.take() {
                    prev.cancel.store(true, Ordering::Relaxed);
                }
                if let Some(integ) = &game.integrated
                    && let Some(terrain) = integ.terrain(&dim_id)
                {
                    let eng_biomes = game
                        .world_biomes
                        .clone()
                        .unwrap_or_else(|| Arc::clone(game.res.biomes()));
                    let job = spawn_background_preload(
                        terrain,
                        rt.block_looks.clone(),
                        eng_biomes,
                        center_x,
                        center_z,
                        radius_chunks,
                        dim_id.clone(),
                        realistic_colors,
                    );
                    let total = job.total;
                    rt.preload_job = Some(job);
                    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
                    st.map.preload = PreloadState {
                        active: true,
                        completed: 0,
                        total,
                        center_x,
                        center_z,
                    };
                } else {
                    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
                    st.set_toast(if rt.russian_ui {
                        "Прогружены видимые чанки".to_owned()
                    } else {
                        "Scanned loaded chunks".to_owned()
                    });
                }
            }
            PendingCommand::ForceRescanLoaded => {}
        }
    }

    // Incremental budgeted chunk surface & cave scanner: max 350 us per frame (zero steady-state allocs)
    if valid_player && !game.world.chunks.is_empty() {
        let eng_biomes = game
            .world_biomes
            .as_deref()
            .unwrap_or_else(|| game.res.biomes());
        let min_y = game.world.shape.min_y;
        let max_y = game.world.shape.max_y();
        let has_ceiling = game.world.shape.has_ceiling;
        let air_id = game.world.air;
        let p_cx = floor_div(px.floor() as i32, 16);
        let p_cz = floor_div(pz.floor() as i32, 16);

        let cave_behind = cave_active
            && game
                .world
                .chunks
                .get(&(p_cx, p_cz))
                .is_some_and(|ch| rt.cave_scanned_ptrs.get(&(p_cx, p_cz)) != Some(&(Arc::as_ptr(ch) as usize, cave_y)));
        let budget_us = if force_rescan || map_open || cave_behind {
            1800
        } else {
            450
        };
        let start = Instant::now();

        let rebuild_queue = force_rescan
            || rt.chunk_queue_len != game.world.chunks.len()
            || rt.chunk_queue_player != (p_cx, p_cz)
            || (rt.frame_count % 32 == 0);
        if rebuild_queue {
            rt.chunk_queue.clear();
            rt.chunk_queue.extend(game.world.chunks.keys().copied());
            rt.chunk_queue.sort_unstable_by_key(|&(cx, cz)| {
                let dx = cx - p_cx;
                let dz = cz - p_cz;
                dx * dx + dz * dz
            });
            rt.chunk_queue_len = game.world.chunks.len();
            rt.chunk_queue_player = (p_cx, p_cz);
        }

        {
            let st = state().lock().unwrap_or_else(|e| e.into_inner());
            if rt.biome_world_key != world_key || rt.biome_cache.len() < st.terrain.biomes.len() {
                rt.biome_cache = st.terrain.biomes.clone();
                rt.biome_world_key = world_key.clone();
            }
        }

        let mut scanned_batch: Vec<(i32, i32, [ColumnCell; 256])> = Vec::new();
        let mut cave_batch: Vec<(i32, i32, [ColumnCell; 256])> = Vec::new();
        let mut biomes_dirty = false;

        let BridgeRuntime {
            chunk_queue,
            scanned_chunk_ptrs,
            cave_scanned_ptrs,
            block_looks,
            biome_cache,
            ..
        } = &mut *rt;

        // First scan any unloaded/edited chunks right around player, then work radially outward.
        for &(cx, cz) in chunk_queue.iter() {
            if start.elapsed().as_micros() >= budget_us {
                break;
            }
            let Some(chunk) = game.world.chunks.get(&(cx, cz)) else {
                continue;
            };
            let ptr = Arc::as_ptr(chunk) as usize;
            if scanned_chunk_ptrs.get(&(cx, cz)) != Some(&ptr) {
                let cells = scan_chunk_surface(
                    chunk,
                    block_looks,
                    eng_biomes,
                    biome_cache,
                    &mut biomes_dirty,
                    air_id,
                    min_y,
                    max_y,
                    realistic_colors,
                    has_ceiling,
                );
                scanned_chunk_ptrs.insert((cx, cz), ptr);
                scanned_batch.push((cx, cz, cells));
            }
            if cave_active
                && (cx - p_cx).abs() <= 16
                && (cz - p_cz).abs() <= 16
                && cave_scanned_ptrs.get(&(cx, cz)) != Some(&(ptr, cave_y))
            {
                let cells = scan_chunk_cave(
                    chunk,
                    block_looks,
                    eng_biomes,
                    biome_cache,
                    &mut biomes_dirty,
                    air_id,
                    min_y,
                    max_y,
                    cave_y,
                );
                cave_scanned_ptrs.insert((cx, cz), (ptr, cave_y));
                cave_batch.push((cx, cz, cells));
            }
        }

        let flush_cap = if !cave_batch.is_empty() || map_open { 24 } else { 8 };
        if biomes_dirty || !scanned_batch.is_empty() || !cave_batch.is_empty() || !map_open {
            let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
            if biomes_dirty {
                st.terrain.biomes = rt.biome_cache.clone();
                st.biomes_dirty = true;
            }
            for (cx, cz, cells) in scanned_batch {
                st.terrain.put_surface_chunk(cx, cz, &cells);
            }
            for (cx, cz, cells) in cave_batch {
                st.terrain.put_cave_chunk(cx, cz, cave_y, cells);
            }
            st.terrain.flush_dirty_queue(flush_cap);
        } else {
            let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
            st.terrain.flush_dirty_queue(flush_cap);
        }
    }

    // Drain any queued GPU patches to `game.map_patches` so atlas stays synced before drawing.
    if let Some(mod_rect) = game.gui.atlas.sprites.get(MOD_IMAGE_SPRITE).copied() {
        let base_x = (mod_rect.u0 * game.gui.atlas.width as f32).round() as u32;
        let base_y = (mod_rect.v0 * game.gui.atlas.height as f32).round() as u32;
        let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
        for p in st.terrain.pending_patches.drain(..) {
            game.map_patches.push(Patch {
                x: base_x + p.x,
                y: base_y + p.y,
                width: p.width,
                height: p.height,
                rgba: p.rgba,
            });
        }
    }
}

/// Draw 3D World Waypoints + Minimap HUD before menu blur/overlay pass (`draw_extras`).
pub fn on_draw_extras(game: &mut Game<'_>, batch: &mut HudBatch) {
    if game.ui.hide_gui || !matches!(game.screen, Screen::Playing | Screen::Chat) {
        return;
    }
    let atlas = &game.gui.atlas;
    let Some(mod_rect) = atlas.sprites.get(MOD_IMAGE_SPRITE).copied() else {
        return;
    };

    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    if !st.player.valid || st.map.open {
        return;
    }

    if st.config.show_waypoints_3d {
        draw_world_waypoints(&st, atlas, batch);
    }
    if st.config.enabled {
        draw_minimap(&mut st, atlas, batch, mod_rect);
    }

    // Flush any newly touched tiles right into map_patches for immediate visibility.
    let base_x = (mod_rect.u0 * atlas.width as f32).round() as u32;
    let base_y = (mod_rect.v0 * atlas.height as f32).round() as u32;
    for p in st.terrain.pending_patches.drain(..) {
        game.map_patches.push(Patch {
            x: base_x + p.x,
            y: base_y + p.y,
            width: p.width,
            height: p.height,
            rgba: p.rgba,
        });
    }
}

/// Draw crisp unblurred Fullscreen World Map (`M`) AFTER `Screen::Mod` backdrop (`hud_layers` RETURN).
pub fn on_hud_layers(game: &mut Game<'_>, batch: &mut HudBatch) {
    let atlas = &game.gui.atlas;
    let Some(mod_rect) = atlas.sprites.get(MOD_IMAGE_SPRITE).copied() else {
        return;
    };
    let mut st = state().lock().unwrap_or_else(|e| e.into_inner());
    if !st.map.open {
        return;
    }

    draw_world_map(&mut st, atlas, batch, mod_rect);

    let base_x = (mod_rect.u0 * atlas.width as f32).round() as u32;
    let base_y = (mod_rect.v0 * atlas.height as f32).round() as u32;
    for p in st.terrain.pending_patches.drain(..) {
        game.map_patches.push(Patch {
            x: base_x + p.x,
            y: base_y + p.y,
            width: p.width,
            height: p.height,
            rgba: p.rgba,
        });
    }
}

fn draw_text_scaled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &str,
    x: f32,
    y: f32,
    scale: f32,
    argb: u32,
    shadow: bool,
) {
    let start = batch.vertices.len();
    batch.text(atlas, text, x, y, hud_color(argb), shadow);
    if (scale - 1.0).abs() > 1e-3 {
        for v in &mut batch.vertices[start..] {
            v.pos[0] = x + (v.pos[0] - x) * scale;
            v.pos[1] = y + (v.pos[1] - y) * scale;
        }
    }
}

fn draw_rect_outline(batch: &mut HudBatch, atlas: &HudAtlas, x: f32, y: f32, w: f32, h: f32, t: f32, argb: u32) {
    let c = hud_color(argb);
    batch.fill(atlas, x, y, w, t, c);
    batch.fill(atlas, x, y + h - t, w, t, c);
    batch.fill(atlas, x, y + t, t, (h - 2.0 * t).max(0.0), c);
    batch.fill(atlas, x + w - t, y + t, t, (h - 2.0 * t).max(0.0), c);
}

fn draw_diamond(batch: &mut HudBatch, atlas: &HudAtlas, cx: f32, cy: f32, r: f32, fill_argb: u32, border_argb: u32) {
    let uv = white_uv(atlas);
    let border = (r * 0.42).clamp(0.45, 0.95);
    let mut push_dia = |rad: f32, col: u32| {
        let pts = [
            [cx, cy - rad],
            [cx + rad, cy],
            [cx, cy + rad],
            [cx - rad, cy],
        ];
        batch.quad(pts, [uv; 4], hud_color(col));
    };
    push_dia(r + border, border_argb);
    push_dia(r, fill_argb);
}

fn draw_circle_fan(batch: &mut HudBatch, atlas: &HudAtlas, cx: f32, cy: f32, r: f32, argb: u32) {
    let uv = white_uv(atlas);
    let col = hud_color(argb);
    const N: usize = 64;
    let center = HudVertex {
        pos: [cx, cy],
        uv,
        color: col,
        depth: 0.0,
        glint_uv: [0.0, 0.0],
        text: 0,
    };
    for i in 0..N {
        let a0 = (i as f32) * std::f32::consts::TAU / (N as f32);
        let a1 = ((i + 1) as f32) * std::f32::consts::TAU / (N as f32);
        batch.vertices.push(center);
        batch.vertices.push(HudVertex {
            pos: [cx + r * a0.cos(), cy + r * a0.sin()],
            uv,
            color: col,
            depth: 0.0,
            glint_uv: [0.0, 0.0],
            text: 0,
        });
        batch.vertices.push(HudVertex {
            pos: [cx + r * a1.cos(), cy + r * a1.sin()],
            uv,
            color: col,
            depth: 0.0,
            glint_uv: [0.0, 0.0],
            text: 0,
        });
    }
}

fn draw_pixel_icon(batch: &mut HudBatch, atlas: &HudAtlas, x: f32, y: f32, px_size: f32, rows: &[&str], argb: u32, shadow: bool) {
    if shadow {
        let sc = hud_color(0xA0101010);
        for (ry, row) in rows.iter().enumerate() {
            for (rx, ch) in row.chars().enumerate() {
                if ch == '#' {
                    batch.fill(
                        atlas,
                        x + (rx as f32) * px_size + 1.0,
                        y + (ry as f32) * px_size + 1.0,
                        px_size,
                        px_size,
                        sc,
                    );
                }
            }
        }
    }
    let c = hud_color(argb);
    for (ry, row) in rows.iter().enumerate() {
        for (rx, ch) in row.chars().enumerate() {
            if ch == '#' {
                batch.fill(
                    atlas,
                    x + (rx as f32) * px_size,
                    y + (ry as f32) * px_size,
                    px_size,
                    px_size,
                    c,
                );
            }
        }
    }
}

const SKULL_7X7: [&str; 7] = [
    ".#####.",
    "#######",
    "#.#.#.#",
    "#######",
    ".#####.",
    ".#.#.#.",
    ".......",
];

fn draw_waypoint_badge(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    cx: f32,
    cy: f32,
    size: f32,
    initial: &str,
    color_argb: u32,
    is_death: bool,
    rim_highlight: bool,
) {
    let half = size * 0.5;
    let x = cx - half;
    let y = cy - half;
    let b = if rim_highlight { 0.7 } else if size <= 7.0 { 0.55 } else { 0.75 };
    let b_col = if rim_highlight { 0xFFECECEC } else { 0xFF101010 };
    batch.fill(atlas, x - b, y - b, size + b * 2.0, size + b * 2.0, hud_color(b_col));
    batch.fill(atlas, x, y, size, size, hud_color(color_argb));

    if is_death {
        let pad = (size * 0.12).max(0.8);
        let px = (size - pad * 2.0) / 7.0;
        draw_pixel_icon(batch, atlas, x + pad, y + pad, px, &SKULL_7X7, 0xFFFFFFFF, false);
    } else {
        let scale = (size / 11.4).clamp(0.56, 1.35);
        let tw = HudBatch::text_width(atlas, initial) * scale;
        let tx = cx - tw * 0.5 + 0.4;
        let ty = cy - 4.0 * scale;
        draw_text_scaled(batch, atlas, initial, tx, ty, scale, contrast_fg(color_argb), false);
    }
}

fn draw_player_arrow(batch: &mut HudBatch, atlas: &HudAtlas, cx: f32, cy: f32, yaw_rad_up: f32, scale: f32, fill: u32) {
    let uv = white_uv(atlas);
    let (s, c) = yaw_rad_up.sin_cos();
    let rot = |lx: f32, ly: f32| -> [f32; 2] {
        [cx + (lx * c - ly * s) * scale, cy + (lx * s + ly * c) * scale]
    };
    let shadow = hud_color(0xCC101010);
    let main = hud_color(fill);
    let tip = rot(0.0, -4.5);
    let r = rot(3.3, 3.2);
    let tail = rot(0.0, 1.6);
    let l = rot(-3.3, 3.2);
    let s_off = [0.8, 0.8];
    batch.quad(
        [
            [tip[0] + s_off[0], tip[1] + s_off[1]],
            [r[0] + s_off[0], r[1] + s_off[1]],
            [tail[0] + s_off[0], tail[1] + s_off[1]],
            [l[0] + s_off[0], l[1] + s_off[1]],
        ],
        [uv; 4],
        shadow,
    );
    batch.quad([tip, r, tail, l], [uv; 4], main);
}

fn companion_sprite_rect(atlas: &HudAtlas, source: OverlaySource, base_r: Rect) -> Option<Rect> {
    let key = match source {
        OverlaySource::Same => return Some(base_r),
        OverlaySource::SheepWool => "entity/sheep/sheep_wool",
        OverlaySource::EndermanEyes => "entity/enderman/enderman_eyes",
        OverlaySource::SpiderEyes => "entity/spider/spider_eyes",
        OverlaySource::DrownedOuter => "entity/zombie/drowned_outer_layer",
        OverlaySource::BoggedOverlay => "entity/skeleton/bogged_overlay",
        OverlaySource::StrayOverlay => "entity/skeleton/stray_overlay",
        OverlaySource::BreezeEyes => "entity/breeze/breeze_eyes",
        OverlaySource::CreakingEyes => "entity/creaking/creaking_eyes",
        OverlaySource::CopperGolemEyes => "entity/copper_golem/copper_golem_eyes",
        OverlaySource::PhantomEyes => "entity/phantom/phantom_eyes",
    };
    atlas.sprites.get(key).copied()
}

fn draw_mob_part(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    part: &MobPart,
    hx: f32,
    hy: f32,
    mw: f32,
    mh: f32,
    base_r: Rect,
    base_tw: f32,
    base_th: f32,
) {
    let px = hx + mw * (f32::from(part.dx16) / 16.0);
    let py = hy + mh * (f32::from(part.dy16) / 16.0);
    let pw = mw * (f32::from(part.dw16) / 16.0);
    let ph = mh * (f32::from(part.dh16) / 16.0);

    if let Some(src_r) = companion_sprite_rect(atlas, part.source, base_r) {
        let (tw, th) = if part.source == OverlaySource::Same {
            (base_tw, base_th)
        } else {
            (
                if src_r.w > 0 { src_r.w as f32 } else { base_tw },
                if src_r.h > 0 { src_r.h as f32 } else { base_th },
            )
        };
        if part.shadow_3d {
            batch.fill(atlas, px - 0.25, py + 0.2, pw + 0.5, ph + 0.25, hud_color(0x8A0A0A0A));
        }
        let uv = sub_rect(
            src_r,
            f32::from(part.px),
            f32::from(part.py),
            f32::from(part.pw),
            f32::from(part.ph),
            tw,
            th,
        );
        batch.rect_uv(px, py, pw, ph, uv, u32::MAX);
        return;
    }

    // Crisp fallback accents if an optional *_eyes / wool companion sprite is not packed in HudAtlas.
    match part.source {
        OverlaySource::SheepWool => {
            batch.fill(atlas, px - 0.15, py - 0.15, pw + 0.3, ph + 0.3, hud_color(0xFFF0EEE8));
        }
        OverlaySource::EndermanEyes => {
            let ey = hy + mh * 0.50;
            let epw = mw * 0.14;
            let eph = (mh * 0.13).max(0.5);
            batch.fill(atlas, hx + mw * 0.13, ey, epw, eph, hud_color(0xFFE066FF));
            batch.fill(atlas, hx + mw * 0.73, ey, epw, eph, hud_color(0xFFE066FF));
        }
        OverlaySource::DrownedOuter => {
            batch.fill(atlas, hx, hy, mw, (mh * 0.16).max(0.6), hud_color(0x90437258));
        }
        OverlaySource::BreezeEyes => {
            let ey = hy + mh * 0.52;
            let eph = (mh * 0.13).max(0.5);
            batch.fill(atlas, hx + mw * 0.18, ey, mw * 0.14, eph, hud_color(0xFFFFEA66));
            batch.fill(atlas, hx + mw * 0.68, ey, mw * 0.14, eph, hud_color(0xFFFFEA66));
        }
        _ => {}
    }
}

fn draw_entity_marker(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    ent: &RadarEntity,
    sx: f32,
    sy: f32,
    tab_mode: bool,
    radius_scale: f32,
) {
    if !tab_mode {
        match ent.category {
            EntityCategory::Mob => {
                draw_diamond(batch, atlas, sx, sy, 1.25 * radius_scale, 0xFFFFEE44, 0xFF141414);
            }
            EntityCategory::Player => {
                draw_diamond(batch, atlas, sx, sy, 1.45 * radius_scale, 0xFFFFFFFF, 0xFF141414);
            }
            EntityCategory::Vehicle => {
                draw_diamond(batch, atlas, sx, sy, 1.35 * radius_scale, 0xFFB855FF, 0xFF141414);
            }
        }
        return;
    }

    // Tab mode: compact, crisp 2.5D layered mob head icons + player nickname!
    let icon_size = 5.0 * radius_scale.clamp(0.85, 1.12);

    match ent.category {
        EntityCategory::Player => {
            let hx = sx - icon_size * 0.5;
            let hy = sy - icon_size * 0.5;
            batch.fill(atlas, hx + 0.3, hy + 0.3, icon_size + 0.6, icon_size + 0.6, hud_color(0x70080808));
            batch.fill(atlas, hx - 0.45, hy - 0.45, icon_size + 0.9, icon_size + 0.9, hud_color(0xDD101010));
            if let Some(&r) = atlas.sprites.get(ent.texture_key.as_str()) {
                let face = sub_rect(r, 8.0, 8.0, 8.0, 8.0, 64.0, 64.0);
                let hat = sub_rect(r, 40.0, 8.0, 8.0, 8.0, 64.0, 64.0);
                batch.rect_uv(hx, hy, icon_size, icon_size, face, u32::MAX);
                batch.rect_uv(hx, hy, icon_size, icon_size, hat, u32::MAX);
            } else {
                batch.fill(atlas, hx, hy, icon_size, icon_size, hud_color(0xFFE6C4A2));
            }
            if !ent.display_name.is_empty() {
                let scale = 0.52;
                let tw = HudBatch::text_width(atlas, &ent.display_name) * scale;
                draw_text_scaled(
                    batch,
                    atlas,
                    &ent.display_name,
                    sx - tw * 0.5,
                    hy + icon_size + 1.0,
                    scale,
                    0xFFFFFFFF,
                    true,
                );
            }
        }
        EntityCategory::Mob => {
            if let Some(&r) = atlas.sprites.get(ent.texture_key.as_str()) {
                let actual_w = if r.w > 0 { r.w as u16 } else { 0 };
                let actual_h = if r.h > 0 { r.h as u16 } else { 0 };
                let spec = mob_model_spec(&ent.species_key, actual_w, actual_h);
                let mw = icon_size * (f32::from(spec.width16) / 16.0);
                let mh = icon_size * (f32::from(spec.height16) / 16.0);
                let hx = sx - mw * 0.5;
                let hy = sy - mh * 0.5;
                let tw = if actual_w > 0 && actual_w == spec.base.tex_w {
                    f32::from(actual_w)
                } else {
                    f32::from(spec.base.tex_w)
                };
                let th = if actual_h > 0 && actual_h == spec.base.tex_h {
                    f32::from(actual_h)
                } else {
                    f32::from(spec.base.tex_h)
                };

                batch.fill(atlas, hx + 0.25, hy + 0.25, mw + 0.55, mh + 0.55, hud_color(0x68080808));
                batch.fill(atlas, hx - 0.45, hy - 0.45, mw + 0.9, mh + 0.9, hud_color(0xDC101010));

                for part in spec.active_parts().iter().filter(|p| p.behind_base) {
                    draw_mob_part(batch, atlas, part, hx, hy, mw, mh, r, tw, th);
                }

                let face = sub_rect(
                    r,
                    f32::from(spec.base.px),
                    f32::from(spec.base.py),
                    f32::from(spec.base.pw),
                    f32::from(spec.base.ph),
                    tw,
                    th,
                );
                batch.rect_uv(hx, hy, mw, mh, face, u32::MAX);

                if let Some([hx0, hy0, hw, hh]) = spec.base.hat {
                    let hat = sub_rect(
                        r,
                        f32::from(hx0),
                        f32::from(hy0),
                        f32::from(hw),
                        f32::from(hh),
                        tw,
                        th,
                    );
                    batch.rect_uv(hx, hy, mw, mh, hat, u32::MAX);
                }

                for part in spec.active_parts().iter().filter(|p| !p.behind_base) {
                    draw_mob_part(batch, atlas, part, hx, hy, mw, mh, r, tw, th);
                }
            } else {
                let spec = mob_model_spec(&ent.species_key, 0, 0);
                let hx = sx - icon_size * 0.5;
                let hy = sy - icon_size * 0.5;
                batch.fill(atlas, hx - 0.45, hy - 0.45, icon_size + 0.9, icon_size + 0.9, hud_color(0xDC101010));
                batch.fill(
                    atlas,
                    hx,
                    hy,
                    icon_size,
                    icon_size,
                    hud_color(0xFF00_0000 | spec.base.fallback_rgb),
                );
                let ch: String = ent
                    .species_key
                    .chars()
                    .next()
                    .unwrap_or('M')
                    .to_uppercase()
                    .collect();
                draw_text_scaled(batch, atlas, &ch, hx + 1.3, hy + 0.5, 0.5, 0xFF121212, false);
            }
        }
        EntityCategory::Vehicle => {
            draw_diamond(batch, atlas, sx, sy, 1.45 * radius_scale, 0xFFB855FF, 0xFF141414);
        }
    }
}

fn draw_world_waypoints(st: &mod_eternity_world_map::state::EternityMapState, atlas: &HudAtlas, batch: &mut HudBatch) {
    let cam = [
        st.player.x,
        st.player.y + st.player.eye_height,
        st.player.z,
    ];
    for wp in &st.waypoints {
        if !wp.enabled || wp.dim != st.terrain.dimension {
            continue;
        }
        let target = [f64::from(wp.x) + 0.5, f64::from(wp.y) + 1.4, f64::from(wp.z) + 0.5];
        if let Some(proj) = project_world_point(
            target,
            cam,
            st.player.yaw_deg,
            st.player.pitch_deg,
            st.player.fov_deg,
            st.player.screen_w,
            st.player.screen_h,
        ) {
            if proj.distance < 3.0 {
                continue;
            }
            draw_waypoint_badge(
                batch,
                atlas,
                proj.x,
                proj.y,
                11.0,
                &wp.initial,
                wp.color,
                wp.is_death,
                false,
            );
            let dist_str = format!("{}m", proj.distance.round() as i32);
            let dw = HudBatch::text_width(atlas, &dist_str) * 0.7;
            draw_text_scaled(
                batch,
                atlas,
                &dist_str,
                proj.x - dw * 0.5,
                proj.y + 7.5,
                0.7,
                0xFFEAEAEA,
                true,
            );
        }
    }
}

fn draw_minimap(
    st: &mut mod_eternity_world_map::state::EternityMapState,
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    mod_rect: Rect,
) {
    let mr = minimap_rect(
        st.config.corner,
        st.config.minimap_size,
        st.player.screen_w,
        st.player.screen_h,
    );
    let inner_r = mr.inner_size * 0.5;

    // 1. Interior black unexplored void background
    match st.config.shape {
        MinimapShape::Square => {
            batch.fill(
                atlas,
                mr.inner_x,
                mr.inner_y,
                mr.inner_size,
                mr.inner_size,
                hud_color(0xFF000000),
            );
        }
        MinimapShape::Round => {
            draw_circle_fan(batch, atlas, mr.center_x, mr.center_y, inner_r + 0.35, 0xFF000000);
        }
    }

    // 2. Visible terrain tiles clipped on stack against Square or 32-gon Round mask
    let px_per_block = st.config.minimap_zoom;
    let half_world = (f64::from(inner_r) * std::f64::consts::SQRT_2) / f64::from(px_per_block);
    let tx_min = floor_div((st.player.x - half_world).floor() as i32, TILE_BLOCKS) - 1;
    let tx_max = floor_div((st.player.x + half_world).ceil() as i32, TILE_BLOCKS) + 1;
    let tz_min = floor_div((st.player.z - half_world).floor() as i32, TILE_BLOCKS) - 1;
    let tz_max = floor_div((st.player.z + half_world).ceil() as i32, TILE_BLOCKS) + 1;

    let (cave_active, _) = st.effective_cave_mode();
    let px_player = st.player.x;
    let pz_player = st.player.z;
    let yaw = st.player.yaw_deg;
    let rot = st.config.rotate_with_player;
    let mut upload_budget = 32usize;
    let du = mod_rect.u1 - mod_rect.u0;
    let dv = mod_rect.v1 - mod_rect.v0;

    for tz in tz_min..=tz_max {
        for tx in tx_min..=tx_max {
            let uv_opt = if cave_active {
                st.terrain
                    .touch_and_get_uv(TileKey::CaveDetail { tx, tz }, &mut upload_budget)
                    .or_else(|| {
                        if st.terrain.cave_chunks.is_empty() {
                            st.terrain
                                .touch_and_get_uv(TileKey::SurfaceDetail { tx, tz }, &mut upload_budget)
                        } else {
                            None
                        }
                    })
            } else {
                st.terrain
                    .touch_and_get_uv(TileKey::SurfaceDetail { tx, tz }, &mut upload_budget)
            };
            let Some(suv) = uv_opt else {
                continue;
            };
            let u0 = mod_rect.u0 + suv.u0 * du;
            let v0 = mod_rect.v0 + suv.v0 * dv;
            let u1 = mod_rect.u0 + suv.u1 * du;
            let v1 = mod_rect.v0 + suv.v1 * dv;

            let wx0 = f64::from(tx * TILE_BLOCKS);
            let wz0 = f64::from(tz * TILE_BLOCKS);
            let wx1 = wx0 + f64::from(TILE_BLOCKS);
            let wz1 = wz0 + f64::from(TILE_BLOCKS);

            let quad = [
                ClipVertex {
                    pos: world_to_minimap(wx0, wz0, px_player, pz_player, yaw, rot, px_per_block, mr.center_x, mr.center_y),
                    uv: [u0, v0],
                },
                ClipVertex {
                    pos: world_to_minimap(wx1, wz0, px_player, pz_player, yaw, rot, px_per_block, mr.center_x, mr.center_y),
                    uv: [u1, v0],
                },
                ClipVertex {
                    pos: world_to_minimap(wx1, wz1, px_player, pz_player, yaw, rot, px_per_block, mr.center_x, mr.center_y),
                    uv: [u1, v1],
                },
                ClipVertex {
                    pos: world_to_minimap(wx0, wz1, px_player, pz_player, yaw, rot, px_per_block, mr.center_x, mr.center_y),
                    uv: [u0, v1],
                },
            ];

            let poly = match st.config.shape {
                MinimapShape::Square => clip_quad_rect(
                    quad,
                    mr.inner_x,
                    mr.inner_y,
                    mr.inner_x + mr.inner_size,
                    mr.inner_y + mr.inner_size,
                ),
                MinimapShape::Round => clip_quad_circle(quad, mr.center_x, mr.center_y, inner_r + 0.55),
            };

            let verts = poly.slice();
            if verts.len() < 3 {
                continue;
            }
            let c_white = hud_color(0xFFFFFFFF);
            let anchor = HudVertex {
                pos: verts[0].pos,
                uv: verts[0].uv,
                color: c_white,
                depth: 0.0,
                glint_uv: [0.0, 0.0],
                text: 0,
            };
            for i in 1..verts.len() - 1 {
                batch.vertices.push(anchor);
                batch.vertices.push(HudVertex {
                    pos: verts[i].pos,
                    uv: verts[i].uv,
                    color: c_white,
                    depth: 0.0,
                    glint_uv: [0.0, 0.0],
                    text: 0,
                });
                batch.vertices.push(HudVertex {
                    pos: verts[i + 1].pos,
                    uv: verts[i + 1].uv,
                    color: c_white,
                    depth: 0.0,
                    glint_uv: [0.0, 0.0],
                    text: 0,
                });
            }
        }
    }

    // 3. Entity radar inside minimap interior (crisp compact Xaero diamonds)
    if st.config.show_entities {
        let clip_rad = inner_r - 2.6;
        for ent in &st.entities {
            let sp = world_to_minimap(
                ent.x,
                ent.z,
                px_player,
                pz_player,
                yaw,
                rot,
                px_per_block,
                mr.center_x,
                mr.center_y,
            );
            let dx = sp[0] - mr.center_x;
            let dy = sp[1] - mr.center_y;
            let inside = match st.config.shape {
                MinimapShape::Square => dx.abs() <= clip_rad && dy.abs() <= clip_rad,
                MinimapShape::Round => dx * dx + dy * dy <= clip_rad * clip_rad,
            };
            if inside {
                draw_entity_marker(batch, atlas, ent, sp[0], sp[1], st.player.tab_held, 1.0);
            }
        }
    }

    // 3b. Interior (unclamped) waypoints drawn BEFORE frame & player arrow so arrow stays readable!
    let rim_r = inner_r + 1.0;
    for wp in &st.waypoints {
        if !wp.enabled || wp.dim != st.terrain.dimension {
            continue;
        }
        let sp = world_to_minimap(
            f64::from(wp.x) + 0.5,
            f64::from(wp.z) + 0.5,
            px_player,
            pz_player,
            yaw,
            rot,
            px_per_block,
            mr.center_x,
            mr.center_y,
        );
        let dx = sp[0] - mr.center_x;
        let dy = sp[1] - mr.center_y;
        let (cx, cy, clamped) = match st.config.shape {
            MinimapShape::Square => clamp_to_square(dx, dy, rim_r),
            MinimapShape::Round => clamp_to_circle(dx, dy, rim_r),
        };
        if !clamped {
            draw_waypoint_badge(
                batch,
                atlas,
                mr.center_x + cx,
                mr.center_y + cy,
                if wp.is_death { 7.5 } else { 6.5 },
                &wp.initial,
                wp.color,
                wp.is_death,
                false,
            );
        }
    }

    // 4. Beveled Xaero metallic/stone frame (Screenshots 1, 2 & 3)
    match st.config.shape {
        MinimapShape::Square => {
            draw_rect_outline(batch, atlas, mr.outer_x, mr.outer_y, mr.outer_size, mr.outer_size, 1.0, 0xFF0E0E0E);
            let top_col = hud_color(0xFF969696);
            let top_col2 = hud_color(0xFF747474);
            let bot_col = hud_color(0xFF424242);
            let bot_col2 = hud_color(0xFF565656);
            batch.fill(atlas, mr.outer_x + 1.0, mr.outer_y + 1.0, mr.outer_size - 2.0, 1.0, top_col);
            batch.fill(atlas, mr.outer_x + 1.0, mr.outer_y + 2.0, 1.0, mr.outer_size - 3.0, top_col);
            batch.fill(atlas, mr.outer_x + 2.0, mr.outer_y + 2.0, mr.outer_size - 4.0, 1.0, top_col2);
            batch.fill(atlas, mr.outer_x + 2.0, mr.outer_y + 3.0, 1.0, mr.outer_size - 5.0, top_col2);
            batch.fill(atlas, mr.outer_x + 1.0, mr.outer_y + mr.outer_size - 2.0, mr.outer_size - 2.0, 1.0, bot_col);
            batch.fill(atlas, mr.outer_x + mr.outer_size - 2.0, mr.outer_y + 1.0, 1.0, mr.outer_size - 3.0, bot_col);
            batch.fill(atlas, mr.outer_x + 2.0, mr.outer_y + mr.outer_size - 3.0, mr.outer_size - 4.0, 1.0, bot_col2);
            batch.fill(atlas, mr.outer_x + mr.outer_size - 3.0, mr.outer_y + 2.0, 1.0, mr.outer_size - 5.0, bot_col2);
            draw_rect_outline(batch, atlas, mr.inner_x - 1.0, mr.inner_y - 1.0, mr.inner_size + 2.0, mr.inner_size + 2.0, 1.0, 0xFF101010);
        }
        MinimapShape::Round => {
            let uv = white_uv(atlas);
            const SEGS: usize = 64;
            let ring_solid = |batch: &mut HudBatch, r_in: f32, r_out: f32, col: u32| {
                let hc = hud_color(col);
                for i in 0..SEGS {
                    let a0 = (i as f32) * std::f32::consts::TAU / (SEGS as f32);
                    let a1 = ((i + 1) as f32) * std::f32::consts::TAU / (SEGS as f32);
                    let (s0, c0) = a0.sin_cos();
                    let (s1, c1) = a1.sin_cos();
                    batch.quad(
                        [
                            [mr.center_x + r_out * c0, mr.center_y + r_out * s0],
                            [mr.center_x + r_out * c1, mr.center_y + r_out * s1],
                            [mr.center_x + r_in * c1, mr.center_y + r_in * s1],
                            [mr.center_x + r_in * c0, mr.center_y + r_in * s0],
                        ],
                        [uv; 4],
                        hc,
                    );
                }
            };
            // Inner dark rim overlaps terrain edge cleanly -> zero polygonal gaps!
            ring_solid(batch, inner_r - 0.35, inner_r + 1.05, 0xFF101010);
            // Directional metallic bevel matching square highlight/shadow lighting
            for i in 0..SEGS {
                let a0 = (i as f32) * std::f32::consts::TAU / (SEGS as f32);
                let a1 = ((i + 1) as f32) * std::f32::consts::TAU / (SEGS as f32);
                let amid = 0.5 * (a0 + a1);
                let light = ((amid + 2.356_194_5).cos() * 0.5 + 0.5).clamp(0.0, 1.0);
                let lum = (68.0 + light * 86.0).round() as u8;
                let hc = rgba(lum, lum, lum, 255);
                let (s0, c0) = a0.sin_cos();
                let (s1, c1) = a1.sin_cos();
                let r_in = inner_r + 0.85;
                let r_out = inner_r + 3.05;
                batch.quad(
                    [
                        [mr.center_x + r_out * c0, mr.center_y + r_out * s0],
                        [mr.center_x + r_out * c1, mr.center_y + r_out * s1],
                        [mr.center_x + r_in * c1, mr.center_y + r_in * s1],
                        [mr.center_x + r_in * c0, mr.center_y + r_in * s0],
                    ],
                    [uv; 4],
                    hc,
                );
            }
            ring_solid(batch, inner_r + 2.85, inner_r + 3.90, 0xFF0E0E0E);
        }
    }

    // 5. Compact Cardinal blue badges tucked cleanly near frame inner edges (С/В/Ю/З | N/E/S/W)
    let dirs = if st.config.russian_lang {
        [
            ("С", 0.0f64, -1000.0f64),
            ("В", 1000.0, 0.0),
            ("Ю", 0.0, 1000.0),
            ("З", -1000.0, 0.0),
        ]
    } else {
        [
            ("N", 0.0f64, -1000.0f64),
            ("E", 1000.0, 0.0),
            ("S", 0.0, 1000.0),
            ("W", -1000.0, 0.0),
        ]
    };
    let badge_rad = inner_r - 3.9;
    let b_sz = 6.5f32;
    for (lbl, dwx, dwz) in dirs {
        let p = world_to_minimap(
            px_player + dwx,
            pz_player + dwz,
            px_player,
            pz_player,
            yaw,
            rot,
            1.0,
            mr.center_x,
            mr.center_y,
        );
        let dx = p[0] - mr.center_x;
        let dy = p[1] - mr.center_y;
        let (cx, cy, _) = match st.config.shape {
            MinimapShape::Square => clamp_to_square(dx, dy, badge_rad),
            MinimapShape::Round => clamp_to_circle(dx, dy, badge_rad),
        };
        let bx = mr.center_x + cx - b_sz * 0.5;
        let by = mr.center_y + cy - b_sz * 0.5;
        batch.fill(atlas, bx - 0.5, by - 0.5, b_sz + 1.0, b_sz + 1.0, hud_color(0xFF0B1136));
        batch.fill(atlas, bx, by, b_sz, b_sz, hud_color(0xEB22358E));
        let scale = 0.56f32;
        let tw = HudBatch::text_width(atlas, lbl) * scale;
        draw_text_scaled(
            batch,
            atlas,
            lbl,
            bx + b_sz * 0.5 - tw * 0.5 + 0.3,
            by + 1.0,
            scale,
            0xFFFFFFFF,
            false,
        );
    }

    // 6. Player directional arrow at center (above interior waypoints!)
    let arrow_rot = if rot { 0.0 } else { yaw.to_radians() + std::f32::consts::PI };
    draw_player_arrow(batch, atlas, mr.center_x, mr.center_y, arrow_rot, 0.72, 0xFFFFFFFF);

    // 7. Rim-clamped out-of-range waypoints drawn cleanly on top of the frame rim
    for wp in &st.waypoints {
        if !wp.enabled || wp.dim != st.terrain.dimension {
            continue;
        }
        let sp = world_to_minimap(
            f64::from(wp.x) + 0.5,
            f64::from(wp.z) + 0.5,
            px_player,
            pz_player,
            yaw,
            rot,
            px_per_block,
            mr.center_x,
            mr.center_y,
        );
        let dx = sp[0] - mr.center_x;
        let dy = sp[1] - mr.center_y;
        let (cx, cy, clamped) = match st.config.shape {
            MinimapShape::Square => clamp_to_square(dx, dy, rim_r),
            MinimapShape::Round => clamp_to_circle(dx, dy, rim_r),
        };
        if clamped {
            draw_waypoint_badge(
                batch,
                atlas,
                mr.center_x + cx,
                mr.center_y + cy,
                6.5,
                &wp.initial,
                wp.color,
                false,
                true,
            );
        }
    }

    // 8. Centered coordinates right below frame (Screenshot 1 & 3)
    let coord_txt = format!(
        "{}, {}, {}",
        px_player.floor() as i32,
        st.player.y.floor() as i32,
        pz_player.floor() as i32
    );
    let cw = HudBatch::text_width(atlas, &coord_txt);
    draw_text_scaled(
        batch,
        atlas,
        &coord_txt,
        (mr.center_x - cw * 0.5).floor(),
        mr.coord_y,
        1.0,
        0xFFFFFFFF,
        true,
    );
}

fn draw_world_map(
    st: &mut mod_eternity_world_map::state::EternityMapState,
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    mod_rect: Rect,
) {
    let w = st.player.screen_w;
    let h = st.player.screen_h;
    let layout = WorldMapLayout::compute(w, h);

    // 1. Pure black unexplored backdrop matching Screenshot 2!
    batch.fill(atlas, 0.0, 0.0, w, h, hud_color(0xFF000000));

    let zoom = st.map.zoom;
    let (cave_active, _) = st.effective_cave_mode();
    let half_w_world = f64::from(w * 0.5) / f64::from(zoom);
    let half_h_world = f64::from(h * 0.5) / f64::from(zoom);
    let min_wx = st.map.center_x - half_w_world;
    let max_wx = st.map.center_x + half_w_world;
    let min_wz = st.map.center_z - half_h_world;
    let max_wz = st.map.center_z + half_h_world;

    let mut upload_budget = 64usize;
    let du = mod_rect.u1 - mod_rect.u0;
    let dv = mod_rect.v1 - mod_rect.v0;

    let map_cx = st.map.center_x;
    let map_cz = st.map.center_z;
    let map_z = f64::from(zoom.max(mod_eternity_world_map::state::MIN_ZOOM));
    let draw_world_tile = |batch: &mut HudBatch,
                           wx0: i32,
                           wz0: i32,
                           span_blocks: i32,
                           suv: mod_eternity_world_map::atlas_cache::SlotUv,
                           tint: u32| {
        let sx0 = w * 0.5 + ((f64::from(wx0) - map_cx) * map_z) as f32;
        let sy0 = h * 0.5 + ((f64::from(wz0) - map_cz) * map_z) as f32;
        let sx1 = w * 0.5 + ((f64::from(wx0 + span_blocks) - map_cx) * map_z) as f32;
        let sy1 = h * 0.5 + ((f64::from(wz0 + span_blocks) - map_cz) * map_z) as f32;
        let r = Rect {
            u0: mod_rect.u0 + suv.u0 * du,
            v0: mod_rect.v0 + suv.v0 * dv,
            u1: mod_rect.u0 + suv.u1 * du,
            v1: mod_rect.v0 + suv.v1 * dv,
            w: 64,
            h: 64,
        };
        batch.rect_uv(sx0, sy0, sx1 - sx0, sy1 - sy0, r, tint);
    };

    let tx_vis0 = floor_div(min_wx.floor() as i32, TILE_BLOCKS);
    let tx_vis1 = floor_div(max_wx.ceil() as i32, TILE_BLOCKS);
    let tz_vis0 = floor_div(min_wz.floor() as i32, TILE_BLOCKS);
    let tz_vis1 = floor_div(max_wz.ceil() as i32, TILE_BLOCKS);

    if !cave_active {
        let rx0 = floor_div(min_wx.floor() as i32, REGION_SIZE);
        let rx1 = floor_div(max_wx.ceil() as i32, REGION_SIZE);
        let rz0 = floor_div(min_wz.floor() as i32, REGION_SIZE);
        let rz1 = floor_div(max_wz.ceil() as i32, REGION_SIZE);

        // Collect ONLY explored regions inside viewport + tile bitmasks (skips black void in O(1)).
        let mut visible_regs: Vec<(i32, i32, u64)> = Vec::new();
        for rz in rz0..=rz1 {
            for rx in rx0..=rx1 {
                if let Some(reg) = st.terrain.regions.get(&(rx, rz))
                    && reg.explored_columns > 0
                {
                    visible_regs.push((rx, rz, reg.explored_tile_mask));
                }
            }
        }

        // Count explored 64x64 & 128x128 tiles intersecting viewport cleanly using masks.
        let mut detail_count = 0usize;
        let mut mid2_count = 0usize;
        let mut mid4_count = 0usize;
        if zoom >= 0.34 {
            for &(rx, rz, mask) in &visible_regs {
                let base_tx = rx * 8;
                let base_tz = rz * 8;
                let lx0 = (tx_vis0 - base_tx).clamp(0, 7);
                let lx1 = (tx_vis1 - base_tx).clamp(0, 7);
                let lz0 = (tz_vis0 - base_tz).clamp(0, 7);
                let lz1 = (tz_vis1 - base_tz).clamp(0, 7);
                for lz in lz0..=lz1 {
                    for lx in lx0..=lx1 {
                        if (mask & (1u64 << ((lz as u32) * 8 + (lx as u32)))) != 0 {
                            detail_count += 1;
                        }
                    }
                }
            }
        }
        if zoom < 0.34 || detail_count > 200 {
            for &(rx, rz, mask) in &visible_regs {
                let base_mx = rx * 4;
                let base_mz = rz * 4;
                let mx_vis0 = floor_div(min_wx.floor() as i32, 128);
                let mx_vis1 = floor_div(max_wx.ceil() as i32, 128);
                let mz_vis0 = floor_div(min_wz.floor() as i32, 128);
                let mz_vis1 = floor_div(max_wz.ceil() as i32, 128);
                let mx0 = (mx_vis0 - base_mx).clamp(0, 3);
                let mx1 = (mx_vis1 - base_mx).clamp(0, 3);
                let mz0 = (mz_vis0 - base_mz).clamp(0, 3);
                let mz1 = (mz_vis1 - base_mz).clamp(0, 3);
                for mz in mz0..=mz1 {
                    for mx in mx0..=mx1 {
                        let m = 0x0303u64 << ((mz as u32) * 16 + (mx as u32) * 2);
                        if (mask & m) != 0 {
                            mid2_count += 1;
                        }
                    }
                }
                for qz in 0..2u8 {
                    for qx in 0..2u8 {
                        let m = 0x0F0F_0F0Fu64 << (u32::from(qz) * 32 + u32::from(qx) * 4);
                        if (mask & m) != 0 {
                            mid4_count += 1;
                        }
                    }
                }
            }
        }

        // Select crispest mip tier where 1 texel stays <= ~1 screen pixel (eliminates zoom-out blur!).
        if zoom >= 0.34 && detail_count <= 200 {
            for &(rx, rz, mask) in &visible_regs {
                let base_tx = rx * 8;
                let base_tz = rz * 8;
                let lx0 = (tx_vis0 - base_tx).clamp(0, 7);
                let lx1 = (tx_vis1 - base_tx).clamp(0, 7);
                let lz0 = (tz_vis0 - base_tz).clamp(0, 7);
                let lz1 = (tz_vis1 - base_tz).clamp(0, 7);
                for lz in lz0..=lz1 {
                    for lx in lx0..=lx1 {
                        if (mask & (1u64 << ((lz as u32) * 8 + (lx as u32)))) == 0 {
                            continue;
                        }
                        let tx = base_tx + lx;
                        let tz = base_tz + lz;
                        if let Some(suv) = st
                            .terrain
                            .touch_and_get_uv(TileKey::SurfaceDetail { tx, tz }, &mut upload_budget)
                        {
                            draw_world_tile(batch, tx * TILE_BLOCKS, tz * TILE_BLOCKS, TILE_BLOCKS, suv, u32::MAX);
                        } else if let Some(suv) = st.terrain.touch_and_get_uv(
                            TileKey::Mid2 {
                                tx: floor_div(tx, 2),
                                tz: floor_div(tz, 2),
                            },
                            &mut upload_budget,
                        ) {
                            draw_world_tile(
                                batch,
                                floor_div(tx, 2) * 128,
                                floor_div(tz, 2) * 128,
                                128,
                                suv,
                                u32::MAX,
                            );
                        }
                    }
                }
            }
        } else if zoom >= 0.17 && mid2_count <= 210 {
            let mx_vis0 = floor_div(min_wx.floor() as i32, 128);
            let mx_vis1 = floor_div(max_wx.ceil() as i32, 128);
            let mz_vis0 = floor_div(min_wz.floor() as i32, 128);
            let mz_vis1 = floor_div(max_wz.ceil() as i32, 128);
            for &(rx, rz, mask) in &visible_regs {
                let base_mx = rx * 4;
                let base_mz = rz * 4;
                let mx0 = (mx_vis0 - base_mx).clamp(0, 3);
                let mx1 = (mx_vis1 - base_mx).clamp(0, 3);
                let mz0 = (mz_vis0 - base_mz).clamp(0, 3);
                let mz1 = (mz_vis1 - base_mz).clamp(0, 3);
                for mz in mz0..=mz1 {
                    for mx in mx0..=mx1 {
                        let m = 0x0303u64 << ((mz as u32) * 16 + (mx as u32) * 2);
                        if (mask & m) == 0 {
                            continue;
                        }
                        let tx = base_mx + mx;
                        let tz = base_mz + mz;
                        if let Some(suv) = st
                            .terrain
                            .touch_and_get_uv(TileKey::Mid2 { tx, tz }, &mut upload_budget)
                        {
                            draw_world_tile(batch, tx * 128, tz * 128, 128, suv, u32::MAX);
                        } else if let Some(suv) = st.terrain.touch_and_get_uv(
                            TileKey::Mid4 {
                                rx,
                                rz,
                                qx: (mx >> 1) as u8,
                                qz: (mz >> 1) as u8,
                            },
                            &mut upload_budget,
                        ) {
                            draw_world_tile(
                                batch,
                                rx * REGION_SIZE + (mx >> 1) * 256,
                                rz * REGION_SIZE + (mz >> 1) * 256,
                                256,
                                suv,
                                u32::MAX,
                            );
                        }
                    }
                }
            }
        } else if zoom >= 0.085 && mid4_count <= 210 {
            for &(rx, rz, mask) in &visible_regs {
                for qz in 0..2u8 {
                    for qx in 0..2u8 {
                        let m = 0x0F0F_0F0Fu64 << (u32::from(qz) * 32 + u32::from(qx) * 4);
                        if (mask & m) == 0 {
                            continue;
                        }
                        if let Some(suv) = st
                            .terrain
                            .touch_and_get_uv(TileKey::Mid4 { rx, rz, qx, qz }, &mut upload_budget)
                        {
                            draw_world_tile(
                                batch,
                                rx * REGION_SIZE + i32::from(qx) * 256,
                                rz * REGION_SIZE + i32::from(qz) * 256,
                                256,
                                suv,
                                u32::MAX,
                            );
                        }
                    }
                }
            }
        } else {
            for &(rx, rz, _) in &visible_regs {
                if let Some(suv) = st
                    .terrain
                    .touch_and_get_uv(TileKey::RegionOverview { rx, rz }, &mut upload_budget)
                {
                    draw_world_tile(batch, rx * REGION_SIZE, rz * REGION_SIZE, REGION_SIZE, suv, u32::MAX);
                }
            }
        }
    } else if !st.terrain.cave_chunks.is_empty() {
        // Cave Mode: pure #000000 void outside caves (never bleed surface terrain!).
        let mut cave_detail_tiles: Vec<(i32, i32)> = st
            .terrain
            .cave_chunks
            .keys()
            .filter_map(|&(cx, cz)| {
                let tx = floor_div(cx, 4);
                let tz = floor_div(cz, 4);
                (tx >= tx_vis0 && tx <= tx_vis1 && tz >= tz_vis0 && tz <= tz_vis1)
                    .then_some((tx, tz))
            })
            .collect();
        cave_detail_tiles.sort_unstable();
        cave_detail_tiles.dedup();
        if zoom >= 0.34 && cave_detail_tiles.len() <= 200 {
            for (tx, tz) in cave_detail_tiles {
                if let Some(suv) = st
                    .terrain
                    .touch_and_get_uv(TileKey::CaveDetail { tx, tz }, &mut upload_budget)
                {
                    draw_world_tile(batch, tx * TILE_BLOCKS, tz * TILE_BLOCKS, TILE_BLOCKS, suv, u32::MAX);
                } else if let Some(suv) = st.terrain.touch_and_get_uv(
                    TileKey::CaveMid2 {
                        tx: floor_div(tx, 2),
                        tz: floor_div(tz, 2),
                    },
                    &mut upload_budget,
                ) {
                    draw_world_tile(
                        batch,
                        floor_div(tx, 2) * 128,
                        floor_div(tz, 2) * 128,
                        128,
                        suv,
                        u32::MAX,
                    );
                }
            }
        } else {
            let mut cave_mid_tiles: Vec<(i32, i32)> = cave_detail_tiles
                .into_iter()
                .map(|(tx, tz)| (floor_div(tx, 2), floor_div(tz, 2)))
                .collect();
            cave_mid_tiles.sort_unstable();
            cave_mid_tiles.dedup();
            for (mx, mz) in cave_mid_tiles {
                if let Some(suv) = st
                    .terrain
                    .touch_and_get_uv(TileKey::CaveMid2 { tx: mx, tz: mz }, &mut upload_budget)
                {
                    draw_world_tile(batch, mx * 128, mz * 128, 128, suv, u32::MAX);
                }
            }
        }
    }

    // 2. Chunk grid overlay when toggled
    if st.config.chunk_grid && zoom >= 0.35 {
        let cx0 = floor_div(min_wx.floor() as i32, 16);
        let cx1 = floor_div(max_wx.ceil() as i32, 16);
        let cz0 = floor_div(min_wz.floor() as i32, 16);
        let cz1 = floor_div(max_wz.ceil() as i32, 16);
        let grid_col = hud_color(0x38FFFFFF);
        if (cx1 - cx0) <= 120 && (cz1 - cz0) <= 120 {
            for cx in cx0..=cx1 {
                let sx = st.world_to_screen(f64::from(cx * 16), 0.0, w, h)[0];
                batch.fill(atlas, sx, 0.0, 1.0, h, grid_col);
            }
            for cz in cz0..=cz1 {
                let sy = st.world_to_screen(0.0, f64::from(cz * 16), w, h)[1];
                batch.fill(atlas, 0.0, sy, w, 1.0, grid_col);
            }
        }
    }

    // 3. Entities on World Map (compact & overlap-throttled when zoomed out!)
    if st.config.show_entities_on_map {
        let dia_scale = if zoom >= 0.65 {
            1.0
        } else if zoom >= 0.32 {
            0.84
        } else {
            0.70
        };
        let mut cell_bloom = [0u64; 32];
        for ent in &st.entities {
            let sp = st.world_to_screen(ent.x, ent.z, w, h);
            if sp[0] >= -16.0 && sp[0] <= w + 16.0 && sp[1] >= -16.0 && sp[1] <= h + 16.0 {
                if !st.player.tab_held && ent.category == EntityCategory::Mob && zoom < 0.65 {
                    let bx = (sp[0] * 0.28).floor() as i32;
                    let by = (sp[1] * 0.28).floor() as i32;
                    let hash = (bx.wrapping_mul(73856093) ^ by.wrapping_mul(19349663)) as u32;
                    let slot = ((hash >> 6) & 31) as usize;
                    let bit = 1u64 << (hash & 63);
                    if (cell_bloom[slot] & bit) != 0 {
                        continue;
                    }
                    cell_bloom[slot] |= bit;
                }
                draw_entity_marker(batch, atlas, ent, sp[0], sp[1], st.player.tab_held, dia_scale);
            }
        }
    }

    // 4. Waypoints on World Map + Hovered Waypoint Tooltip
    let mut hovered_wp: Option<(f32, f32, String)> = None;
    if st.config.show_waypoints_on_map {
        for wp in &st.waypoints {
            if wp.dim != st.terrain.dimension {
                continue;
            }
            let sp = st.world_to_screen(f64::from(wp.x) + 0.5, f64::from(wp.z) + 0.5, w, h);
            if sp[0] >= -20.0 && sp[0] <= w + 20.0 && sp[1] >= -20.0 && sp[1] <= h + 20.0 {
                let sz = if wp.enabled { 8.5 } else { 6.5 };
                let col = if wp.enabled { wp.color } else { (wp.color & 0x00FF_FFFF) | 0x6000_0000 };
                draw_waypoint_badge(batch, atlas, sp[0], sp[1], sz, &wp.initial, col, wp.is_death, false);
                let dx = sp[0] - st.cursor_x;
                let dy = sp[1] - st.cursor_y;
                if dx * dx + dy * dy <= 110.0 && hovered_wp.is_none() {
                    hovered_wp = Some((sp[0], sp[1], format!("{} ({}, {}, {})", wp.name, wp.x, wp.y, wp.z)));
                }
            }
        }
    }

    // 5. Player red arrow on World Map (matches Screenshot 2 red triangle!)
    if st.player.valid {
        let pp = st.world_to_screen(st.player.x, st.player.z, w, h);
        let arrow_rot = st.player.yaw_deg.to_radians() + std::f32::consts::PI;
        draw_player_arrow(batch, atlas, pp[0], pp[1], arrow_rot, 1.05, 0xFFE52424);
    }

    if let Some((wx, wy, tip)) = hovered_wp
        && st.map.active_panel == ActivePanel::None
    {
        let tw = HudBatch::text_width(atlas, &tip) * 0.8 + 8.0;
        let tx = (wx - tw * 0.5).clamp(4.0, (w - tw - 4.0).max(4.0));
        let ty = (wy - 22.0).max(28.0);
        batch.fill(atlas, tx, ty, tw, 13.0, hud_color(0xDD141414));
        draw_rect_outline(batch, atlas, tx, ty, tw, 13.0, 1.0, 0xFF6A6A6A);
        draw_text_scaled(batch, atlas, &tip, tx + 4.0, ty + 3.0, 0.8, 0xFFFFFFFF, true);
    }

    // 6. Top-Center hovered X Y Z + Localized Biome Readout (Screenshot 2)
    let [hbx, hbz] = st.map.hovered_block;
    let top_coords = match st.map.hovered_y {
        Some(hy) => format!("X: {hbx} Y: {hy} Z: {hbz}"),
        None => format!("X: {hbx} Z: {hbz}"),
    };
    let tcw = HudBatch::text_width(atlas, &top_coords);
    draw_text_scaled(batch, atlas, &top_coords, (w - tcw) * 0.5, 5.0, 1.0, 0xFFFFFFFF, true);
    if !st.map.hovered_biome_name.is_empty() {
        let bw = HudBatch::text_width(atlas, &st.map.hovered_biome_name);
        draw_text_scaled(
            batch,
            atlas,
            &st.map.hovered_biome_name,
            (w - bw) * 0.5,
            16.0,
            1.0,
            0xFFDEDEDE,
            true,
        );
    }

    // 7. Bottom-Center Zoom Readout (e.g. `0.698x` matching Screenshot 2)
    let zoom_txt = format!("{zoom:.3}x");
    let zw = HudBatch::text_width(atlas, &zoom_txt);
    draw_text_scaled(batch, atlas, &zoom_txt, (w - zw) * 0.5, h - 13.0, 1.0, 0xFFFFFFFF, true);

    // 8. Top-Right Compass Cross (N/E/S/W diagonal cross matching Screenshot 2)
    let cr = layout.compass_rect;
    let ccx = cr.x + cr.w * 0.5;
    let ccy = cr.y + cr.h * 0.5;
    let arm = 9.0;
    let uv = white_uv(atlas);
    let c_white = hud_color(0xFFFFFFFF);
    for (dx, dy) in [(1.0f32, 1.0f32), (-1.0, 1.0)] {
        let nx = -dy * 0.7;
        let ny = dx * 0.7;
        batch.quad(
            [
                [ccx - arm * dx - nx, ccy - arm * dy - ny],
                [ccx + arm * dx - nx, ccy + arm * dy - ny],
                [ccx + arm * dx + nx, ccy + arm * dy + ny],
                [ccx - arm * dx + nx, ccy - arm * dy + ny],
            ],
            [uv; 4],
            c_white,
        );
    }
    let (n, e, s, wl) = if st.config.russian_lang {
        ("С", "В", "Ю", "З")
    } else {
        ("N", "E", "S", "W")
    };
    draw_text_scaled(batch, atlas, n, ccx - 3.0, cr.y - 1.0, 0.85, 0xFFFFFFFF, true);
    draw_text_scaled(batch, atlas, e, cr.x + cr.w - 6.0, ccy - 4.0, 0.85, 0xFFFFFFFF, true);
    draw_text_scaled(batch, atlas, s, ccx - 3.0, cr.y + cr.h - 8.0, 0.85, 0xFFFFFFFF, true);
    draw_text_scaled(batch, atlas, wl, cr.x - 1.0, ccy - 4.0, 0.85, 0xFFFFFFFF, true);

    // 9. Left & Right Toolbars (Screenshot 2 matching buttons + icons!)
    draw_toolbar_buttons(st, atlas, batch, &layout);

    // 10. Preload Progress / Toast Pill
    if st.map.preload.active {
        let p = &st.map.preload;
        let txt = if st.config.russian_lang {
            format!("Прогрузка карты: {}/{} чанков", p.completed, p.total)
        } else {
            format!("Scanning area: {}/{} chunks", p.completed, p.total)
        };
        let pw = HudBatch::text_width(atlas, &txt) + 14.0;
        let px = (w - pw) * 0.5;
        batch.fill(atlas, px, 29.0, pw, 16.0, hud_color(0xCC181818));
        draw_rect_outline(batch, atlas, px, 29.0, pw, 16.0, 1.0, 0xFF5A5A5A);
        draw_text_scaled(batch, atlas, &txt, px + 7.0, 33.0, 0.9, 0xFF88FF88, true);
    } else if let Some((msg, _)) = &st.map.toast {
        let tw = HudBatch::text_width(atlas, msg) + 14.0;
        let tx = (w - tw) * 0.5;
        batch.fill(atlas, tx, 29.0, tw, 16.0, hud_color(0xCC181818));
        draw_rect_outline(batch, atlas, tx, 29.0, tw, 16.0, 1.0, 0xFF6A6A6A);
        draw_text_scaled(batch, atlas, msg, tx + 7.0, 33.0, 0.9, 0xFFFFFFFF, true);
    }

    // 11. Active Modals / Panels (Waypoint Create/Edit, Context Menu, Settings, Waypoints List, Help)
    draw_active_panel(st, atlas, batch, &layout);
}

const GEAR_7X7: [&str; 7] = [
    ".#.#.#.",
    "..###..",
    "###.###",
    "#..#..#",
    "###.###",
    "..###..",
    ".#.#.#.",
];

const PRELOAD_7X7: [&str; 7] = [
    ".#####.",
    "#.....#",
    "#.##...",
    "#......",
    "...##.#",
    "#.....#",
    ".#####.",
];

const CAVE_7X7: [&str; 7] = [
    "..###..",
    ".#####.",
    "##...##",
    "#.....#",
    "#.....#",
    "#.....#",
    "#######",
];

const EXIT_7X7: [&str; 7] = [
    "####...",
    "#...#..",
    "#.#.###",
    "#..####",
    "#.#.###",
    "#...#..",
    "####...",
];

const FACE_7X7: [&str; 7] = [
    "#######",
    "#.....#",
    "#.#.#.#",
    "#.....#",
    "#..#..#",
    "#.#.#.#",
    "#######",
];

fn draw_toolbar_buttons(
    st: &mod_eternity_world_map::state::EternityMapState,
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    layout: &WorldMapLayout,
) {
    let ru = st.config.russian_lang;
    for &(id, rect) in &layout.buttons {
        let hovered = st.map.hovered_button == Some(id);
        let active_tint = match id {
            MapButtonId::ChunkGrid => st.config.chunk_grid,
            MapButtonId::CaveMode => st.config.cave_mode != CaveMode::Off,
            MapButtonId::ToggleWaypoints => st.config.show_waypoints_on_map,
            MapButtonId::ToggleEntities => st.config.show_entities_on_map,
            MapButtonId::CenterPlayer => st.map.follow_player,
            _ => true,
        };
        if hovered {
            batch.fill(atlas, rect.x, rect.y, rect.w, rect.h, hud_color(0x90282828));
            draw_rect_outline(batch, atlas, rect.x, rect.y, rect.w, rect.h, 1.0, 0xFF888888);
        } else {
            batch.fill(atlas, rect.x, rect.y, rect.w, rect.h, hud_color(0x55121212));
        }

        let col = if active_tint || hovered { 0xFFFFFFFF } else { 0xFF707070 };
        let ix = rect.x + 3.0;
        let iy = rect.y + 3.0;
        match id {
            MapButtonId::Settings => draw_pixel_icon(batch, atlas, ix, iy, 2.0, &GEAR_7X7, col, true),
            MapButtonId::QuickWaypoint => {
                draw_diamond(batch, atlas, rect.x + 10.0, rect.y + 10.0, 4.0, col, 0xFF101010);
                batch.fill(atlas, rect.x + 13.0, rect.y + 13.0, 4.0, 1.5, hud_color(col));
                batch.fill(atlas, rect.x + 14.2, rect.y + 11.8, 1.5, 4.0, hud_color(col));
            }
            MapButtonId::PreloadRegion => draw_pixel_icon(batch, atlas, ix, iy, 2.0, &PRELOAD_7X7, col, true),
            MapButtonId::WaypointsList => {
                draw_rect_outline(batch, atlas, rect.x + 4.0, rect.y + 4.0, 12.0, 12.0, 2.0, col);
            }
            MapButtonId::CaveMode => {
                draw_pixel_icon(batch, atlas, ix, iy, 2.0, &CAVE_7X7, col, true);
                let badge = match st.config.cave_mode {
                    CaveMode::Auto => Some(("A", 0xFF55C8FF)),
                    CaveMode::Manual => Some(("M", 0xFFFFCC44)),
                    CaveMode::Off => None,
                };
                if let Some((btxt, bcol)) = badge {
                    draw_text_scaled(batch, atlas, btxt, rect.x + 13.0, rect.y + 12.0, 0.58, bcol, true);
                }
            }
            MapButtonId::HelpMenu => {
                for r in 0..3 {
                    batch.fill(atlas, rect.x + 4.0, rect.y + 5.0 + (r as f32) * 4.0, 12.0, 2.0, hud_color(col));
                }
            }
            MapButtonId::ZoomIn => {
                batch.fill(atlas, rect.x + 5.0, rect.y + 9.0, 10.0, 2.0, hud_color(col));
                batch.fill(atlas, rect.x + 9.0, rect.y + 5.0, 2.0, 10.0, hud_color(col));
            }
            MapButtonId::ZoomOut => {
                batch.fill(atlas, rect.x + 5.0, rect.y + 9.0, 10.0, 2.0, hud_color(col));
            }
            MapButtonId::ResetZoom => {
                draw_text_scaled(batch, atlas, "1x", rect.x + 4.5, rect.y + 6.0, 0.95, col, true);
            }
            MapButtonId::ExitMap => draw_pixel_icon(batch, atlas, ix, iy, 2.0, &EXIT_7X7, col, true),
            MapButtonId::ChunkGrid => {
                draw_rect_outline(batch, atlas, rect.x + 4.0, rect.y + 4.0, 12.0, 12.0, 1.0, col);
                batch.fill(atlas, rect.x + 9.5, rect.y + 4.0, 1.0, 12.0, hud_color(col));
                batch.fill(atlas, rect.x + 4.0, rect.y + 9.5, 12.0, 1.0, hud_color(col));
            }
            MapButtonId::CenterPlayer => draw_pixel_icon(batch, atlas, ix, iy, 2.0, &FACE_7X7, col, true),
            MapButtonId::ToggleWaypoints => {
                batch.fill(atlas, rect.x + 5.0, rect.y + 5.0, 10.0, 10.0, hud_color(if active_tint { 0xFFD93B3B } else { 0xFF555555 }));
                draw_rect_outline(batch, atlas, rect.x + 5.0, rect.y + 5.0, 10.0, 10.0, 1.0, col);
                draw_text_scaled(batch, atlas, "W", rect.x + 7.2, rect.y + 7.2, 0.72, col, false);
                if !active_tint {
                    let uv = white_uv(atlas);
                    batch.quad(
                        [
                            [rect.x + 3.0, rect.y + 15.0],
                            [rect.x + 15.0, rect.y + 3.0],
                            [rect.x + 16.5, rect.y + 4.5],
                            [rect.x + 4.5, rect.y + 16.5],
                        ],
                        [uv; 4],
                        hud_color(0xFFE53838),
                    );
                }
            }
            MapButtonId::ToggleEntities => {
                draw_diamond(batch, atlas, rect.x + 10.0, rect.y + 10.0, 4.5, if active_tint { 0xFFFFEE44 } else { col }, 0xFF101010);
                if !active_tint {
                    let uv = white_uv(atlas);
                    batch.quad(
                        [
                            [rect.x + 3.0, rect.y + 15.0],
                            [rect.x + 15.0, rect.y + 3.0],
                            [rect.x + 16.5, rect.y + 4.5],
                            [rect.x + 4.5, rect.y + 16.5],
                        ],
                        [uv; 4],
                        hud_color(0xFFE53838),
                    );
                }
            }
        }
    }

    // Inline Cave Mode switcher popup: [Авто] | [Ручн] | [Выкл] + Y-slice slider
    if st.map.cave_slider_open {
        let tr = layout.cave_slider_track;
        batch.fill(atlas, tr.x, tr.y, tr.w, tr.h, hud_color(0xE0121418));
        draw_rect_outline(batch, atlas, tr.x, tr.y, tr.w, tr.h, 1.0, 0xFF687080);

        let pills = [
            (CaveMode::Auto, if ru { "Авто" } else { "Auto" }),
            (CaveMode::Manual, if ru { "Ручн" } else { "Manual" }),
            (CaveMode::Off, if ru { "Выкл" } else { "Off" }),
        ];
        let bw = 47.0;
        let bh = 14.0;
        let py = tr.y + 3.0;
        for (idx, &(mode, plabel)) in pills.iter().enumerate() {
            let px = tr.x + 3.0 + (idx as f32) * 49.5;
            let active = st.config.cave_mode == mode;
            let pill_hov = st.cursor_x >= px
                && st.cursor_x <= px + bw
                && st.cursor_y >= py
                && st.cursor_y <= py + bh;
            let bg = if active {
                if mode == CaveMode::Off { 0xE87A2E2E } else { 0xE82860A8 }
            } else if pill_hov {
                0xE82B2E36
            } else {
                0xE81B1D22
            };
            let border = if active {
                if mode == CaveMode::Off { 0xFFE06060 } else { 0xFF68B4FF }
            } else {
                0xFF424650
            };
            batch.fill(atlas, px, py, bw, bh, hud_color(bg));
            draw_rect_outline(batch, atlas, px, py, bw, bh, 1.0, border);
            let tw = HudBatch::text_width(atlas, plabel) * 0.72;
            draw_text_scaled(
                batch,
                atlas,
                plabel,
                px + (bw - tw) * 0.5,
                py + 4.0,
                0.72,
                if active { 0xFFFFFFFF } else { 0xFFC0C6D0 },
                false,
            );
        }

        let (ca, cy) = st.effective_cave_mode();
        let shown_y = if st.config.cave_mode == CaveMode::Auto && ca {
            cy
        } else {
            st.config.cave_y
        };
        let frac = ((shown_y as f32 + 64.0) / 384.0).clamp(0.0, 1.0);
        let fill_w = (tr.w - 16.0) * frac;
        let lbl = match st.config.cave_mode {
            CaveMode::Auto => {
                if ca {
                    format!("Y: {} ({})", cy, if ru { "авто" } else { "auto" })
                } else if ru {
                    "Авто (поверхность)".to_owned()
                } else {
                    "Auto (surface)".to_owned()
                }
            }
            CaveMode::Manual => format!("Cave Y: {}", st.config.cave_y),
            CaveMode::Off => if ru { "Пещеры выкл.".to_owned() } else { "Cave: Off".to_owned() },
        };
        draw_text_scaled(batch, atlas, &lbl, tr.x + 6.0, tr.y + 19.5, 0.72, 0xFFE8EEF8, false);

        let bar_col = match st.config.cave_mode {
            CaveMode::Manual => 0xFF55A8FF,
            CaveMode::Auto => 0xFF3E94D8,
            CaveMode::Off => 0xFF606060,
        };
        batch.fill(atlas, tr.x + 8.0, tr.y + 29.5, tr.w - 16.0, 3.0, hud_color(0xFF404040));
        batch.fill(atlas, tr.x + 8.0, tr.y + 29.5, fill_w, 3.0, hud_color(bar_col));
        batch.fill(atlas, tr.x + 6.0 + fill_w, tr.y + 27.0, 4.0, 8.0, hud_color(0xFFFFFFFF));
    } else if st.map.hovered_button == Some(MapButtonId::CaveMode) {
        if let Some(cr) = layout.button_rect(MapButtonId::CaveMode) {
            let mode_lbl = match (st.config.cave_mode, ru) {
                (CaveMode::Auto, true) => "Авто",
                (CaveMode::Manual, true) => "Ручной",
                (CaveMode::Off, true) => "Выкл",
                (CaveMode::Auto, false) => "Auto",
                (CaveMode::Manual, false) => "Manual",
                (CaveMode::Off, false) => "Off",
            };
            let txt = if ru {
                format!("Режим пещер: {mode_lbl}")
            } else {
                format!("Cave Mode: {mode_lbl}")
            };
            let bw = HudBatch::text_width(atlas, &txt) + 12.0;
            batch.fill(atlas, cr.x + cr.w + 6.0, cr.y + 2.0, bw, 16.0, hud_color(0xD8141414));
            draw_text_scaled(batch, atlas, &txt, cr.x + cr.w + 12.0, cr.y + 6.0, 0.95, 0xFFFFFFFF, true);
        }
    } else if let Some(hb) = st.map.hovered_button {
        if let Some(br) = layout.button_rect(hb) {
            let label = match (hb, ru) {
                (MapButtonId::Settings, true) => "Настройки карты",
                (MapButtonId::Settings, false) => "Map Settings",
                (MapButtonId::QuickWaypoint, true) => "Создать метку",
                (MapButtonId::QuickWaypoint, false) => "New Waypoint",
                (MapButtonId::PreloadRegion, true) => "Прогрузить область (512x512)",
                (MapButtonId::PreloadRegion, false) => "Scan / Preload Area (512x512)",
                (MapButtonId::WaypointsList, true) => "Список меток",
                (MapButtonId::WaypointsList, false) => "Waypoints",
                (MapButtonId::HelpMenu, true) => "Управление",
                (MapButtonId::HelpMenu, false) => "Controls & Help",
                (MapButtonId::ZoomIn, _) => "Zoom +",
                (MapButtonId::ZoomOut, _) => "Zoom -",
                (MapButtonId::ResetZoom, _) => "Reset 1.0x",
                (MapButtonId::ExitMap, true) => "Закрыть (M)",
                (MapButtonId::ExitMap, false) => "Close Map (M)",
                (MapButtonId::ChunkGrid, true) => "Сетка чанков",
                (MapButtonId::ChunkGrid, false) => "Chunk Grid",
                (MapButtonId::CenterPlayer, true) => "Центр на игроке",
                (MapButtonId::CenterPlayer, false) => "Center Player",
                (MapButtonId::ToggleWaypoints, true) => "Скрыть/показать метки",
                (MapButtonId::ToggleWaypoints, false) => "Toggle Waypoints",
                (MapButtonId::ToggleEntities, true) => "Скрыть/показать мобов",
                (MapButtonId::ToggleEntities, false) => "Toggle Entities",
                _ => "",
            };
            if !label.is_empty() {
                let lw = HudBatch::text_width(atlas, label) * 0.85 + 10.0;
                let tx = if br.x > st.player.screen_w * 0.5 {
                    br.x - lw - 6.0
                } else {
                    br.x + br.w + 6.0
                };
                batch.fill(atlas, tx, br.y + 2.0, lw, 15.0, hud_color(0xD8141414));
                draw_rect_outline(batch, atlas, tx, br.y + 2.0, lw, 15.0, 1.0, 0xFF555555);
                draw_text_scaled(batch, atlas, label, tx + 5.0, br.y + 5.5, 0.85, 0xFFFFFFFF, false);
            }
        }
    }
}

fn draw_active_panel(
    st: &mod_eternity_world_map::state::EternityMapState,
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    layout: &WorldMapLayout,
) {
    let ru = st.config.russian_lang;
    match st.map.active_panel {
        ActivePanel::None => {}
        ActivePanel::ContextMenu => {
            let Some(cm) = &st.map.context_menu else { return };
            let mw = 148.0;
            let row_h = 18.0;
            let mut items: Vec<&str> = Vec::new();
            if cm.near_waypoint_id.is_some() {
                items.push(if ru { "Удалить метку" } else { "Delete Waypoint" });
            }
            items.push(if ru { "Создать метку..." } else { "Create Waypoint..." });
            items.push(if ru { "Быстрая метка" } else { "Quick Waypoint" });
            items.push(if ru { "Прогрузить область" } else { "Preload Area (512x512)" });
            items.push(if ru { "Телепорт (/tp)" } else { "Teleport Here (/tp)" });
            items.push(if ru { "Копировать XYZ" } else { "Copy Coordinates" });

            let mh = (items.len() as f32) * row_h + 6.0;
            batch.fill(atlas, cm.screen_x, cm.screen_y, mw, mh, hud_color(0xEE181818));
            draw_rect_outline(batch, atlas, cm.screen_x, cm.screen_y, mw, mh, 1.0, 0xFF707070);
            for (i, label) in items.iter().enumerate() {
                let ry = cm.screen_y + 3.0 + (i as f32) * row_h;
                let row_r = UiRect { x: cm.screen_x + 2.0, y: ry, w: mw - 4.0, h: row_h };
                if row_r.contains(st.cursor_x, st.cursor_y) {
                    batch.fill(atlas, row_r.x, row_r.y, row_r.w, row_r.h, hud_color(0xFF334A78));
                }
                draw_text_scaled(batch, atlas, label, cm.screen_x + 7.0, ry + 5.0, 0.85, 0xFFFFFFFF, true);
            }
        }
        ActivePanel::WaypointModal => {
            let bx = layout.modal_box;
            batch.fill(atlas, bx.x, bx.y, bx.w, bx.h, hud_color(0xF0181818));
            draw_rect_outline(batch, atlas, bx.x, bx.y, bx.w, bx.h, 1.0, 0xFF7A7A7A);

            let title = if st.map.modal.editing_id.is_some() {
                if ru { "Редактировать метку" } else { "Edit Waypoint" }
            } else if ru {
                "Новая метка"
            } else {
                "New Waypoint"
            };
            let title_full = format!("{title} ({}, {})", st.map.modal.x, st.map.modal.z);
            draw_text_scaled(batch, atlas, &title_full, bx.x + 12.0, bx.y + 8.0, 0.9, 0xFFFFFFFF, true);

            // Name & Initial boxes
            let name_active = st.map.modal.active_field == ModalField::Name;
            batch.fill(atlas, bx.x + 12.0, bx.y + 28.0, bx.w - 66.0, 18.0, hud_color(0xFF0E0E0E));
            draw_rect_outline(
                batch,
                atlas,
                bx.x + 12.0,
                bx.y + 28.0,
                bx.w - 66.0,
                18.0,
                1.0,
                if name_active { 0xFFFFFFFF } else { 0xFF5A5A5A },
            );
            let name_disp = if st.map.modal.name.is_empty() {
                if ru { "Название..." } else { "Name..." }
            } else {
                st.map.modal.name.as_str()
            };
            draw_text_scaled(
                batch,
                atlas,
                name_disp,
                bx.x + 16.0,
                bx.y + 33.0,
                0.85,
                if st.map.modal.name.is_empty() { 0xFF7A7A7A } else { 0xFFFFFFFF },
                false,
            );

            let init_active = st.map.modal.active_field == ModalField::Initial;
            batch.fill(atlas, bx.x + bx.w - 48.0, bx.y + 28.0, 36.0, 18.0, hud_color(0xFF0E0E0E));
            draw_rect_outline(
                batch,
                atlas,
                bx.x + bx.w - 48.0,
                bx.y + 28.0,
                36.0,
                18.0,
                1.0,
                if init_active { 0xFFFFFFFF } else { 0xFF5A5A5A },
            );
            draw_text_scaled(batch, atlas, &st.map.modal.initial, bx.x + bx.w - 42.0, bx.y + 33.0, 0.9, 0xFFFFFFFF, false);

            // Y Controls: `-`, input, `+`
            batch.fill(atlas, bx.x + 12.0, bx.y + 54.0, 18.0, 18.0, hud_color(0xFF2B2B2B));
            draw_text_scaled(batch, atlas, "-", bx.x + 18.0, bx.y + 59.0, 0.9, 0xFFFFFFFF, false);

            let y_active = st.map.modal.active_field == ModalField::Y;
            batch.fill(atlas, bx.x + 32.0, bx.y + 54.0, 68.0, 18.0, hud_color(0xFF0E0E0E));
            draw_rect_outline(
                batch,
                atlas,
                bx.x + 32.0,
                bx.y + 54.0,
                68.0,
                18.0,
                1.0,
                if y_active { 0xFFFFFFFF } else { 0xFF5A5A5A },
            );
            let y_lbl = format!("Y: {}", st.map.modal.y_text);
            draw_text_scaled(batch, atlas, &y_lbl, bx.x + 37.0, bx.y + 59.0, 0.85, 0xFFFFFFFF, false);

            batch.fill(atlas, bx.x + 102.0, bx.y + 54.0, 18.0, 18.0, hud_color(0xFF2B2B2B));
            draw_text_scaled(batch, atlas, "+", bx.x + 108.0, bx.y + 59.0, 0.9, 0xFFFFFFFF, false);

            // Preview badge on right of Y row
            draw_waypoint_badge(
                batch,
                atlas,
                bx.x + bx.w - 28.0,
                bx.y + 63.0,
                14.0,
                &st.map.modal.initial,
                PALETTE[st.map.modal.color_idx % PALETTE.len()],
                false,
                false,
            );

            // 12 color swatches (2x6)
            for (i, &col) in PALETTE.iter().enumerate() {
                let c_x = bx.x + 12.0 + ((i % 6) as f32) * 24.0;
                let c_y = bx.y + 82.0 + ((i / 6) as f32) * 22.0;
                batch.fill(atlas, c_x, c_y, 20.0, 18.0, hud_color(col));
                let border = if st.map.modal.color_idx == i { 0xFFFFFFFF } else { 0xFF333333 };
                draw_rect_outline(batch, atlas, c_x, c_y, 20.0, 18.0, if st.map.modal.color_idx == i { 2.0 } else { 1.0 }, border);
            }

            // Action buttons
            batch.fill(atlas, bx.x + 12.0, bx.y + bx.h - 28.0, 66.0, 20.0, hud_color(0xFF2E6E3E));
            draw_text_scaled(
                batch,
                atlas,
                if ru { "Сохранить" } else { "Save" },
                bx.x + 19.0,
                bx.y + bx.h - 22.0,
                0.82,
                0xFFFFFFFF,
                true,
            );
            batch.fill(atlas, bx.x + 84.0, bx.y + bx.h - 28.0, 64.0, 20.0, hud_color(0xFF3A3A3A));
            draw_text_scaled(
                batch,
                atlas,
                if ru { "Отмена" } else { "Cancel" },
                bx.x + 93.0,
                bx.y + bx.h - 22.0,
                0.82,
                0xFFFFFFFF,
                true,
            );
            if st.map.modal.editing_id.is_some() {
                batch.fill(atlas, bx.x + bx.w - 68.0, bx.y + bx.h - 28.0, 56.0, 20.0, hud_color(0xFF882828));
                draw_text_scaled(
                    batch,
                    atlas,
                    if ru { "Удалить" } else { "Delete" },
                    bx.x + bx.w - 61.0,
                    bx.y + bx.h - 22.0,
                    0.82,
                    0xFFFFFFFF,
                    true,
                );
            }
        }
        ActivePanel::Settings => {
            let sw = 236.0f32.min(layout.screen_w - 14.0);
            let sh = 222.0f32.min(layout.screen_h - 14.0);
            let sx = (layout.screen_w - sw) * 0.5;
            let sy = (layout.screen_h - sh) * 0.5;
            batch.fill(atlas, sx, sy, sw, sh, hud_color(0xF0181818));
            draw_rect_outline(batch, atlas, sx, sy, sw, sh, 1.0, 0xFF7A7A7A);

            draw_text_scaled(
                batch,
                atlas,
                if ru { "Eternity world map — Настройки" } else { "Eternity world map — Settings" },
                sx + 12.0,
                sy + 6.0,
                0.86,
                0xFFFFFFFF,
                true,
            );
            draw_text_scaled(batch, atlas, "X", sx + sw - 18.0, sy + 6.0, 0.9, 0xFFCCCCCC, true);

            let size_lbl = if ru {
                format!("Размер миникарты: {} px", st.config.minimap_size.round() as i32)
            } else {
                format!("Minimap Size: {} px", st.config.minimap_size.round() as i32)
            };
            draw_text_scaled(batch, atlas, &size_lbl, sx + 18.0, sy + 20.0, 0.80, 0xFFEAEAEA, false);

            let frac = ((st.config.minimap_size - 64.0) / 192.0).clamp(0.0, 1.0);
            let tw = sw - 36.0;
            batch.fill(atlas, sx + 18.0, sy + 37.0, tw, 4.0, hud_color(0xFF454545));
            batch.fill(atlas, sx + 18.0, sy + 37.0, tw * frac, 4.0, hud_color(0xFF55A8FF));
            batch.fill(atlas, sx + 16.0 + tw * frac, sy + 33.0, 5.0, 12.0, hud_color(0xFFFFFFFF));

            let presets = [84.0f32, 104.0, 140.0, 180.0];
            for (i, &p) in presets.iter().enumerate() {
                let cx = sx + 12.0 + (i as f32) * 53.0;
                let sel = (st.config.minimap_size - p).abs() < 2.0;
                batch.fill(atlas, cx, sy + 50.0, 49.0, 15.0, hud_color(if sel { 0xFF335C99 } else { 0xFF2D2D2D }));
                draw_text_scaled(batch, atlas, &format!("{}px", p as i32), cx + 9.0, sy + 53.5, 0.78, 0xFFFFFFFF, false);
            }

            let z_lbl = if ru {
                format!("Зум миникарты: {:.2}x", st.config.minimap_zoom)
            } else {
                format!("Minimap Zoom: {:.2}x", st.config.minimap_zoom)
            };
            draw_text_scaled(batch, atlas, &z_lbl, sx + 14.0, sy + 72.5, 0.80, 0xFFEAEAEA, false);
            batch.fill(atlas, sx + sw - 54.0, sy + 69.0, 20.0, 15.0, hud_color(0xFF2B2B2B));
            draw_rect_outline(batch, atlas, sx + sw - 54.0, sy + 69.0, 20.0, 15.0, 1.0, 0xFF4A4A4A);
            draw_text_scaled(batch, atlas, "-", sx + sw - 46.5, sy + 72.5, 0.85, 0xFFFFFFFF, false);
            batch.fill(atlas, sx + sw - 32.0, sy + 69.0, 20.0, 15.0, hud_color(0xFF2B2B2B));
            draw_rect_outline(batch, atlas, sx + sw - 32.0, sy + 69.0, 20.0, 15.0, 1.0, 0xFF4A4A4A);
            draw_text_scaled(batch, atlas, "+", sx + sw - 24.5, sy + 72.5, 0.85, 0xFFFFFFFF, false);

            let rows = [
                if ru {
                    format!("Форма: {}", match st.config.shape { MinimapShape::Square => "Квадрат", MinimapShape::Round => "Круг" })
                } else {
                    format!("Shape: {}", match st.config.shape { MinimapShape::Square => "Square", MinimapShape::Round => "Round" })
                },
                if ru {
                    format!("Позиция: {:?}", st.config.corner)
                } else {
                    format!("Position: {:?}", st.config.corner)
                },
                if ru {
                    format!("Поворот за игроком: {}", if st.config.rotate_with_player { "Да" } else { "Север" })
                } else {
                    format!("Rotate Map: {}", if st.config.rotate_with_player { "Player" } else { "North Up" })
                },
                if ru {
                    format!("Режим пещер: {:?}", st.config.cave_mode)
                } else {
                    format!("Cave Mode: {:?}", st.config.cave_mode)
                },
                if ru {
                    format!("Радар сущностей: {}", if st.config.show_entities { "Вкл" } else { "Выкл" })
                } else {
                    format!("Entity Radar: {}", if st.config.show_entities { "ON" } else { "OFF" })
                },
                if ru {
                    format!("3D метки в мире: {}", if st.config.show_waypoints_3d { "Вкл" } else { "Выкл" })
                } else {
                    format!("3D Waypoints: {}", if st.config.show_waypoints_3d { "ON" } else { "OFF" })
                },
                if ru {
                    format!("Метка смерти: {}", if st.config.auto_death_waypoint { "Вкл" } else { "Выкл" })
                } else {
                    format!("Death Waypoint: {}", if st.config.auto_death_waypoint { "ON" } else { "OFF" })
                },
                if ru {
                    format!("Метки на карте: {}", if st.config.show_waypoints_on_map { "Вкл" } else { "Выкл" })
                } else {
                    format!("Map Waypoints: {}", if st.config.show_waypoints_on_map { "ON" } else { "OFF" })
                },
            ];
            for (idx, txt) in rows.iter().enumerate() {
                let ry = sy + 90.0 + (idx as f32) * 15.5;
                batch.fill(atlas, sx + 12.0, ry, sw - 24.0, 14.0, hud_color(0xFF242424));
                draw_rect_outline(batch, atlas, sx + 12.0, ry, sw - 24.0, 14.0, 1.0, 0xFF424242);
                draw_text_scaled(batch, atlas, txt, sx + 18.0, ry + 3.0, 0.78, 0xFFFFFFFF, false);
            }
        }
        ActivePanel::WaypointsList => {
            let pw = 256.0f32.min(layout.screen_w - 20.0);
            let ph = 200.0f32.min(layout.screen_h - 20.0);
            let px = (layout.screen_w - pw) * 0.5;
            let py = (layout.screen_h - ph) * 0.5;
            batch.fill(atlas, px, py, pw, ph, hud_color(0xF0181818));
            draw_rect_outline(batch, atlas, px, py, pw, ph, 1.0, 0xFF7A7A7A);
            draw_text_scaled(
                batch,
                atlas,
                if ru { "Метки текущего мира" } else { "World Waypoints" },
                px + 12.0,
                py + 8.0,
                0.95,
                0xFFFFFFFF,
                true,
            );
            draw_text_scaled(batch, atlas, "X", px + pw - 18.0, py + 8.0, 0.9, 0xFFCCCCCC, true);

            let dim_wps: Vec<_> = st
                .waypoints
                .iter()
                .filter(|w| w.dim == st.terrain.dimension)
                .collect();
            if dim_wps.is_empty() {
                draw_text_scaled(
                    batch,
                    atlas,
                    if ru { "Меток пока нет. Кликните по карте!" } else { "No waypoints. Click anywhere on the map!" },
                    px + 14.0,
                    py + 40.0,
                    0.82,
                    0xFFAAAAAA,
                    false,
                );
                return;
            }
            let skip = st.map.waypoints_scroll.min(dim_wps.len().saturating_sub(1));
            for (row_idx, wp) in dim_wps.iter().skip(skip).take(8).enumerate() {
                let ry = py + 28.0 + (row_idx as f32) * 20.0;
                batch.fill(atlas, px + 10.0, ry, 16.0, 16.0, hud_color(if wp.enabled { wp.color } else { 0xFF333333 }));
                draw_text_scaled(batch, atlas, if wp.enabled { "✓" } else { " " }, px + 14.0, ry + 3.0, 0.85, contrast_fg(wp.color), false);

                let lbl = format!("{} ({}, {}, {})", wp.name, wp.x, wp.y, wp.z);
                draw_text_scaled(batch, atlas, &lbl, px + 32.0, ry + 4.0, 0.8, 0xFFFFFFFF, false);

                batch.fill(atlas, px + pw - 62.0, ry, 24.0, 16.0, hud_color(0xFF34496E));
                draw_text_scaled(batch, atlas, if ru { "Изм" } else { "Edt" }, px + pw - 58.0, ry + 4.0, 0.72, 0xFFFFFFFF, false);

                batch.fill(atlas, px + pw - 34.0, ry, 24.0, 16.0, hud_color(0xFF7E2B2B));
                draw_text_scaled(batch, atlas, "Del", px + pw - 30.0, ry + 4.0, 0.72, 0xFFFFFFFF, false);
            }
        }
        ActivePanel::Help => {
            let hw = 240.0f32.min(layout.screen_w - 20.0);
            let hh = 152.0;
            let hx = (layout.screen_w - hw) * 0.5;
            let hy = (layout.screen_h - hh) * 0.5;
            batch.fill(atlas, hx, hy, hw, hh, hud_color(0xF0181818));
            draw_rect_outline(batch, atlas, hx, hy, hw, hh, 1.0, 0xFF7A7A7A);
            let lines = if ru {
                [
                    "Eternity world map — Управление",
                    "M : Открыть / закрыть карту мира",
                    "B : Быстрая метка на позиции",
                    "ЛКМ по карте : Создать / изменить метку",
                    "ПКМ по карте : Меню, Телепорт, Прогрузка",
                    "Tab : Головы игроков и мобов на радаре",
                    "H / C : Скрыть карту / Режим пещер",
                ]
            } else {
                [
                    "Eternity world map — Controls",
                    "M : Toggle Fullscreen World Map",
                    "B : Quick Waypoint at Player",
                    "Left-Click Map : Create / Edit Waypoint",
                    "Right-Click Map : Menu, /tp, Preload Area",
                    "Tab : Player Skins & Mob Faces on Radar",
                    "H / C : Hide Minimap / Cycle Cave Mode",
                ]
            };
            for (i, line) in lines.iter().enumerate() {
                draw_text_scaled(
                    batch,
                    atlas,
                    line,
                    hx + 12.0,
                    hy + 10.0 + (i as f32) * 19.0,
                    if i == 0 { 0.92 } else { 0.8 },
                    if i == 0 { 0xFFFFFFFF } else { 0xFFDCDCDC },
                    i == 0,
                );
            }
        }
    }
}
