//! Persistent 512x512 world regions + zero-allocation 225-slot GPU tile pool inside 1024x1024 MOD_IMAGE.

use crate::terrain::{ColumnCell, shade_column_pixel};
use std::collections::{HashMap, HashSet, VecDeque};

pub const ATLAS_SIZE: u32 = 1024;
pub const SLOT_INNER: u32 = 64;
pub const SLOT_PAD: u32 = 1;
pub const SLOT_STRIDE: u32 = SLOT_INNER + SLOT_PAD * 2; // 66 px
pub const GRID_COLS: u32 = ATLAS_SIZE / SLOT_STRIDE; // 15
pub const GRID_ROWS: u32 = ATLAS_SIZE / SLOT_STRIDE; // 15
pub const TOTAL_SLOTS: usize = (GRID_COLS * GRID_ROWS) as usize; // 225

pub const REGION_SIZE: i32 = 512;
pub const REGION_SHIFT: i32 = 9;
pub const REGION_MASK: i32 = 511;
pub const OVERVIEW_SIZE: usize = 64; // 8x8 blocks -> 1 pixel

pub const TILE_CHUNKS: i32 = 4;
pub const TILE_BLOCKS: i32 = 64; // 4 chunks * 16 blocks

#[inline]
pub fn floor_div(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TileKey {
    /// 64x64 surface blocks (4x4 chunks) at 1:1 texel resolution.
    SurfaceDetail { tx: i32, tz: i32 },
    /// 64x64 cave-slice blocks (4x4 chunks) at 1:1 texel resolution.
    CaveDetail { tx: i32, tz: i32 },
    /// 128x128 cave-slice blocks (8x8 chunks) downsampled 2:1 for zoomed-out Cave Mode.
    CaveMid2 { tx: i32, tz: i32 },
    /// 128x128 surface blocks downsampled 2:1 into 64x64 texels (sharp mid zoom-out).
    Mid2 { tx: i32, tz: i32 },
    /// 256x256 surface blocks (quadrant of 512x512 region) downsampled 4:1 into 64x64 texels.
    Mid4 { rx: i32, rz: i32, qx: u8, qz: u8 },
    /// 512x512 block region downsampled 8:1 into 64x64 overview texels.
    RegionOverview { rx: i32, rz: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingPatch {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Bijective table mapping compact u16 biome IDs stored on disk to canonical biome names ("minecraft:plains").
#[derive(Clone, Debug, Default)]
pub struct BiomeTable {
    names: Vec<String>,
    lookup: HashMap<String, u16>,
}

impl BiomeTable {
    pub fn lookup_id(&self, name: &str) -> Option<u16> {
        self.lookup.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn intern(&mut self, name: &str) -> u16 {
        if let Some(&id) = self.lookup.get(name) {
            return id;
        }
        let id = self.names.len().min(u16::MAX as usize - 1) as u16;
        if (id as usize) == self.names.len() {
            self.names.push(name.to_owned());
            self.lookup.insert(name.to_owned(), id);
        }
        id
    }

    pub fn name(&self, id: u16) -> Option<&str> {
        self.names.get(usize::from(id)).map(String::as_str)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.names.len() * 24);
        out.extend_from_slice(b"ATLSBMS1");
        out.extend_from_slice(&(self.names.len() as u16).to_le_bytes());
        for n in &self.names {
            let b = n.as_bytes();
            let l = b.len().min(255) as u8;
            out.push(l);
            out.extend_from_slice(&b[..l as usize]);
        }
        out
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 10 || &data[0..8] != b"ATLSBMS1" {
            return None;
        }
        let count = u16::from_le_bytes([data[8], data[9]]) as usize;
        let mut pos = 10;
        let mut out = Self::default();
        for _ in 0..count {
            let l = *data.get(pos)? as usize;
            pos += 1;
            let s = std::str::from_utf8(data.get(pos..pos + l)?).ok()?;
            pos += l;
            out.intern(s);
        }
        Some(out)
    }
}

/// Persistent 512x512 block region + derived 64x64 overview RGBA cache.
pub struct RegionBuffer {
    pub cells: Box<[ColumnCell]>, // 512 * 512
    pub overview_rgba: Box<[[u8; 4]]>, // 64 * 64
    pub overview_dirty: bool,
    pub dirty_to_disk: bool,
    pub explored_columns: u32,
    /// Bitmask over the 8x8 grid of 64x64-block tiles inside this region (`bit = tlz*8 + tlx`).
    pub explored_tile_mask: u64,
}

impl RegionBuffer {
    pub fn new_empty() -> Self {
        Self {
            cells: vec![ColumnCell::default(); (REGION_SIZE * REGION_SIZE) as usize].into_boxed_slice(),
            overview_rgba: vec![[0, 0, 0, 255]; OVERVIEW_SIZE * OVERVIEW_SIZE].into_boxed_slice(),
            overview_dirty: true,
            dirty_to_disk: false,
            explored_columns: 0,
            explored_tile_mask: 0,
        }
    }

    #[inline]
    pub fn has_detail_tile(&self, tlx: i32, tlz: i32) -> bool {
        let bit = (tlz as u32) * 8 + (tlx as u32);
        (self.explored_tile_mask & (1u64 << bit)) != 0
    }

    #[inline]
    pub fn has_mid2_tile(&self, mx: i32, mz: i32) -> bool {
        let bx = (mx as u32) * 2;
        let bz = (mz as u32) * 2;
        let m = 0x0303u64 << (bz * 8 + bx);
        (self.explored_tile_mask & m) != 0
    }

    #[inline]
    pub fn has_mid4_tile(&self, qx: u8, qz: u8) -> bool {
        let bx = u32::from(qx) * 4;
        let bz = u32::from(qz) * 4;
        let m = 0x0F0F_0F0Fu64 << (bz * 8 + bx);
        (self.explored_tile_mask & m) != 0
    }

    #[inline]
    pub fn index(lx: i32, lz: i32) -> usize {
        (lz as usize) * (REGION_SIZE as usize) + (lx as usize)
    }

    #[inline]
    pub fn get(&self, lx: i32, lz: i32) -> ColumnCell {
        self.cells[Self::index(lx, lz)]
    }

    pub fn set_chunk_surface(&mut self, clx: i32, clz: i32, chunk_cells: &[ColumnCell; 256]) {
        let base_x = clx * 16;
        let base_z = clz * 16;
        let mut any_explored = false;
        for z in 0..16 {
            let row = (base_z + z) as usize * REGION_SIZE as usize + base_x as usize;
            for x in 0..16 {
                let incoming = chunk_cells[(z * 16 + x) as usize];
                let slot = &mut self.cells[row + x as usize];
                if incoming.is_explored() {
                    any_explored = true;
                    if !slot.is_explored() {
                        self.explored_columns += 1;
                    }
                }
                *slot = incoming;
            }
        }
        if any_explored {
            let bit = ((clz as u32 >> 2) * 8) + (clx as u32 >> 2);
            self.explored_tile_mask |= 1u64 << bit;
        }
        self.overview_dirty = true;
        self.dirty_to_disk = true;
    }

    /// Fill unexplored cells in `self` from a previously persisted `disk` region without
    /// clobbering freshly scanned live chunks.
    pub fn merge_unexplored_from(&mut self, disk: &RegionBuffer) -> bool {
        let mut merged_any = false;
        for (idx, &dc) in disk.cells.iter().enumerate() {
            if dc.is_explored() && !self.cells[idx].is_explored() {
                self.cells[idx] = dc;
                self.explored_columns += 1;
                merged_any = true;
            }
        }
        if merged_any {
            self.explored_tile_mask |= disk.explored_tile_mask;
            self.overview_dirty = true;
            self.dirty_to_disk = true;
        }
        merged_any
    }

    /// Recompute the 64x64 overview texture preserving crisp land/water separation.
    pub fn ensure_overview(&mut self) {
        if !self.overview_dirty {
            return;
        }
        self.overview_dirty = false;
        for oz in 0..OVERVIEW_SIZE as i32 {
            for ox in 0..OVERVIEW_SIZE as i32 {
                let c = downsample_region_box(self, None, 0, 0, ox * 8, oz * 8, 8);
                self.overview_rgba[(oz as usize) * OVERVIEW_SIZE + (ox as usize)] = c;
            }
        }
    }

    /// Encode explored cells sparsely (only runs of explored cells!) so unvisited ocean/air regions stay tiny.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4096);
        out.extend_from_slice(b"ATLSRGN1");
        let total = REGION_SIZE as usize * REGION_SIZE as usize;
        let mut i = 0;
        while i < total {
            // Skip unexplored run.
            let mut skip = 0usize;
            while i < total && !self.cells[i].is_explored() && skip < 0x3FFF_FFFF {
                i += 1;
                skip += 1;
            }
            if skip > 0 {
                write_varint(&mut out, (skip as u32) << 1);
            }
            // Write explored run.
            let start = i;
            let mut run = 0usize;
            while i < total && self.cells[i].is_explored() && run < 0x3FFF_FFFF {
                i += 1;
                run += 1;
            }
            if run > 0 {
                write_varint(&mut out, ((run as u32) << 1) | 1);
                for c in &self.cells[start..start + run] {
                    out.extend_from_slice(&c.rgb);
                    out.extend_from_slice(&c.height.to_le_bytes());
                    out.extend_from_slice(&c.biome.to_le_bytes());
                    out.push(c.water_depth);
                    out.push(c.flags);
                }
            }
        }
        out
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 8 || &data[0..8] != b"ATLSRGN1" {
            return None;
        }
        let mut reg = Self::new_empty();
        let mut pos = 8;
        let mut idx = 0usize;
        let total = reg.cells.len();
        while pos < data.len() && idx < total {
            let tag = read_varint(data, &mut pos)?;
            let is_explored_run = (tag & 1) != 0;
            let count = (tag >> 1) as usize;
            if idx + count > total {
                return None;
            }
            if !is_explored_run {
                idx += count;
            } else {
                let bytes_needed = count * 9;
                if pos + bytes_needed > data.len() {
                    return None;
                }
                for _ in 0..count {
                    let c = ColumnCell {
                        rgb: [data[pos], data[pos + 1], data[pos + 2]],
                        height: i16::from_le_bytes([data[pos + 3], data[pos + 4]]),
                        biome: u16::from_le_bytes([data[pos + 5], data[pos + 6]]),
                        water_depth: data[pos + 7],
                        flags: data[pos + 8],
                    };
                    reg.cells[idx] = c;
                    if c.is_explored() {
                        reg.explored_columns += 1;
                        let lx = (idx as i32) & REGION_MASK;
                        let lz = (idx as i32) >> REGION_SHIFT;
                        let bit = ((lz as u32 >> 6) * 8) + (lx as u32 >> 6);
                        reg.explored_tile_mask |= 1u64 << bit;
                    }
                    pos += 9;
                    idx += 1;
                }
            }
        }
        reg.overview_dirty = true;
        reg.dirty_to_disk = false;
        Some(reg)
    }
}

/// Crisp material-separating box downsample: avoids blending water & shoreline mud together.
fn downsample_region_box(
    reg: &RegionBuffer,
    store: Option<&TerrainStore>,
    wx_base: i32,
    wz_base: i32,
    lx0: i32,
    lz0: i32,
    step: i32,
) -> [u8; 4] {
    let mut land_r = 0u32;
    let mut land_g = 0u32;
    let mut land_b = 0u32;
    let mut land_n = 0u32;

    let mut water_r = 0u32;
    let mut water_g = 0u32;
    let mut water_b = 0u32;
    let mut water_n = 0u32;

    for dz in 0..step {
        let lz = lz0 + dz;
        let row = (lz as usize) * (REGION_SIZE as usize);
        for dx in 0..step {
            let lx = lx0 + dx;
            let cell = reg.cells[row + (lx as usize)];
            if !cell.is_explored() {
                continue;
            }
            let north = if lz > 0 {
                Some(reg.cells[(lz as usize - 1) * (REGION_SIZE as usize) + (lx as usize)])
            } else {
                store.and_then(|s| s.get_column(wx_base + lx, wz_base + lz - 1))
            };
            let west = if lx > 0 {
                Some(reg.cells[row + (lx as usize - 1)])
            } else {
                store.and_then(|s| s.get_column(wx_base + lx - 1, wz_base + lz))
            };
            let nw = if lz > 0 && lx > 0 {
                Some(reg.cells[(lz as usize - 1) * (REGION_SIZE as usize) + (lx as usize - 1)])
            } else {
                store.and_then(|s| s.get_column(wx_base + lx - 1, wz_base + lz - 1))
            };
            let nw_step = nw
                .filter(|p| p.is_explored())
                .map_or(0, |p| cell.height - p.height);
            let px = shade_column_pixel(cell, north, west, nw_step, false);
            if cell.is_water() {
                water_r += u32::from(px[0]);
                water_g += u32::from(px[1]);
                water_b += u32::from(px[2]);
                water_n += 1;
            } else {
                land_r += u32::from(px[0]);
                land_g += u32::from(px[1]);
                land_b += u32::from(px[2]);
                land_n += 1;
            }
        }
    }

    let total = land_n + water_n;
    if total == 0 {
        return [0, 0, 0, 255];
    }
    if water_n > land_n {
        [(water_r / water_n) as u8, (water_g / water_n) as u8, (water_b / water_n) as u8, 255]
    } else if land_n > 0 {
        [(land_r / land_n) as u8, (land_g / land_n) as u8, (land_b / land_n) as u8, 255]
    } else {
        [(water_r / water_n) as u8, (water_g / water_n) as u8, (water_b / water_n) as u8, 255]
    }
}

fn write_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_varint(data: &[u8], pos: &mut usize) -> Option<u32> {
    let mut val = 0u32;
    let mut shift = 0u32;
    for _ in 0..5 {
        let b = *data.get(*pos)?;
        *pos += 1;
        val |= u32::from(b & 0x7F) << shift;
        if (b & 0x80) == 0 {
            return Some(val);
        }
        shift += 7;
    }
    None
}

#[derive(Clone, Debug)]
struct SlotMeta {
    key: Option<TileKey>,
    last_used: u64,
    dirty: bool,
}

impl Default for SlotMeta {
    fn default() -> Self {
        Self {
            key: None,
            last_used: 0,
            dirty: false,
        }
    }
}

/// UV rectangle inside the 1024x1024 MOD_IMAGE sprite region (normalized to [0..1] inside MOD_IMAGE).
#[derive(Clone, Copy, Debug)]
pub struct SlotUv {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// Manages 225 cached GPU 64x64 terrain tiles inside Horizon's MOD_IMAGE slot (1024x1024) + surface/cave store.
pub struct TerrainStore {
    pub world_key: String,
    pub dimension: String,
    pub regions: HashMap<(i32, i32), RegionBuffer>,
    pub cave_chunks: HashMap<(i32, i32), (i32, Box<[ColumnCell; 256]>)>,
    pub last_cave_y: i32,
    pub last_cave_active: bool,
    pub biomes: BiomeTable,

    slots: Vec<SlotMeta>,
    key_to_slot: HashMap<TileKey, usize>,
    dirty_queue: VecDeque<TileKey>,
    dirty_set: HashSet<TileKey>,
    pub frame_seq: u64,
    pub pending_patches: Vec<PendingPatch>,
}

impl TerrainStore {
    pub fn new() -> Self {
        Self {
            world_key: String::new(),
            dimension: "minecraft:overworld".to_owned(),
            regions: HashMap::new(),
            cave_chunks: HashMap::new(),
            last_cave_y: 40,
            last_cave_active: false,
            biomes: BiomeTable::default(),
            slots: vec![SlotMeta::default(); TOTAL_SLOTS],
            key_to_slot: HashMap::with_capacity(TOTAL_SLOTS),
            dirty_queue: VecDeque::new(),
            dirty_set: HashSet::new(),
            frame_seq: 1,
            pending_patches: Vec::new(),
        }
    }

    pub fn switch_world_dim(&mut self, world_key: String, dimension: String) {
        if self.world_key == world_key && self.dimension == dimension {
            return;
        }
        self.world_key = world_key;
        self.dimension = dimension;
        self.regions.clear();
        self.cave_chunks.clear();
        self.clear_gpu_slots();
    }

    pub fn clear_gpu_slots(&mut self) {
        for slot in &mut self.slots {
            *slot = SlotMeta::default();
        }
        self.key_to_slot.clear();
        self.dirty_queue.clear();
        self.dirty_set.clear();
    }

    pub fn slot_pixel_origin(slot: usize) -> (u32, u32) {
        let col = (slot as u32) % GRID_COLS;
        let row = (slot as u32) / GRID_COLS;
        (col * SLOT_STRIDE, row * SLOT_STRIDE)
    }

    pub fn slot_uv_in_mod_image(slot: usize) -> SlotUv {
        let (ox, oy) = Self::slot_pixel_origin(slot);
        let inv = 1.0 / (ATLAS_SIZE as f32);
        SlotUv {
            u0: ((ox + SLOT_PAD) as f32) * inv,
            v0: ((oy + SLOT_PAD) as f32) * inv,
            u1: ((ox + SLOT_PAD + SLOT_INNER) as f32) * inv,
            v1: ((oy + SLOT_PAD + SLOT_INNER) as f32) * inv,
        }
    }

    #[inline]
    pub fn get_column(&self, wx: i32, wz: i32) -> Option<ColumnCell> {
        let rx = floor_div(wx, REGION_SIZE);
        let rz = floor_div(wz, REGION_SIZE);
        let lx = wx - rx * REGION_SIZE;
        let lz = wz - rz * REGION_SIZE;
        self.regions.get(&(rx, rz)).map(|r| r.get(lx, lz))
    }

    #[inline]
    pub fn get_cave_column(&self, wx: i32, wz: i32) -> Option<ColumnCell> {
        let cx = floor_div(wx, 16);
        let cz = floor_div(wz, 16);
        let lx = wx - cx * 16;
        let lz = wz - cz * 16;
        self.cave_chunks
            .get(&(cx, cz))
            .map(|(_, c)| c[(lz * 16 + lx) as usize])
    }

    /// Merge a region loaded from disk into the in-memory store (preserving any freshly scanned chunks)
    /// and invalidate cached GPU tiles covering this region so restored terrain renders immediately.
    pub fn merge_disk_region(&mut self, rx: i32, rz: i32, disk: RegionBuffer) {
        let changed = match self.regions.get_mut(&(rx, rz)) {
            Some(existing) => existing.merge_unexplored_from(&disk),
            None => {
                self.regions.insert((rx, rz), disk);
                true
            }
        };
        if changed {
            let base_tx = rx * 8;
            let base_tz = rz * 8;
            for dtz in 0..8 {
                for dtx in 0..8 {
                    self.mark_tile_dirty(TileKey::SurfaceDetail {
                        tx: base_tx + dtx,
                        tz: base_tz + dtz,
                    });
                }
            }
            let base_m2x = rx * 4;
            let base_m2z = rz * 4;
            for dmz in 0..4 {
                for dmx in 0..4 {
                    self.mark_tile_dirty(TileKey::Mid2 {
                        tx: base_m2x + dmx,
                        tz: base_m2z + dmz,
                    });
                }
            }
            for qz in 0..2u8 {
                for qx in 0..2u8 {
                    self.mark_tile_dirty(TileKey::Mid4 { rx, rz, qx, qz });
                }
            }
            self.mark_tile_dirty(TileKey::RegionOverview { rx, rz });
        }
    }

    /// Record scanned 16x16 surface chunk and mark affected GPU tiles dirty across mip pyramid.
    pub fn put_surface_chunk(&mut self, cx: i32, cz: i32, cells: &[ColumnCell; 256]) {
        let rx = floor_div(cx, 32);
        let rz = floor_div(cz, 32);
        let clx = cx - rx * 32;
        let clz = cz - rz * 32;
        let reg = self
            .regions
            .entry((rx, rz))
            .or_insert_with(RegionBuffer::new_empty);
        reg.set_chunk_surface(clx, clz, cells);

        let tx = floor_div(cx, TILE_CHUNKS);
        let tz = floor_div(cz, TILE_CHUNKS);
        self.mark_tile_dirty(TileKey::SurfaceDetail { tx, tz });
        // Mark east, south, and SE diagonal neighbor tiles dirty on seam boundaries (NW relief continuity).
        let east_edge = (cx - tx * TILE_CHUNKS) == TILE_CHUNKS - 1;
        let south_edge = (cz - tz * TILE_CHUNKS) == TILE_CHUNKS - 1;
        if east_edge {
            self.mark_tile_dirty(TileKey::SurfaceDetail { tx: tx + 1, tz });
        }
        if south_edge {
            self.mark_tile_dirty(TileKey::SurfaceDetail { tx, tz: tz + 1 });
        }
        if east_edge && south_edge {
            self.mark_tile_dirty(TileKey::SurfaceDetail { tx: tx + 1, tz: tz + 1 });
        }

        let m2x = floor_div(cx, 8);
        let m2z = floor_div(cz, 8);
        self.mark_tile_dirty(TileKey::Mid2 { tx: m2x, tz: m2z });
        let qx = (clx >> 4) as u8;
        let qz = (clz >> 4) as u8;
        self.mark_tile_dirty(TileKey::Mid4 { rx, rz, qx, qz });
        self.mark_tile_dirty(TileKey::RegionOverview { rx, rz });
    }

    pub fn put_cave_chunk(&mut self, cx: i32, cz: i32, cave_y: i32, cells: [ColumnCell; 256]) {
        self.cave_chunks.insert((cx, cz), (cave_y, Box::new(cells)));
        let tx = floor_div(cx, TILE_CHUNKS);
        let tz = floor_div(cz, TILE_CHUNKS);
        self.mark_tile_dirty(TileKey::CaveDetail { tx, tz });
        let east_edge = (cx - tx * TILE_CHUNKS) == TILE_CHUNKS - 1;
        let south_edge = (cz - tz * TILE_CHUNKS) == TILE_CHUNKS - 1;
        if east_edge {
            self.mark_tile_dirty(TileKey::CaveDetail { tx: tx + 1, tz });
        }
        if south_edge {
            self.mark_tile_dirty(TileKey::CaveDetail { tx, tz: tz + 1 });
        }
        if (cx - tx * TILE_CHUNKS) == 0 {
            self.mark_tile_dirty(TileKey::CaveDetail { tx: tx - 1, tz });
        }
        if (cz - tz * TILE_CHUNKS) == 0 {
            self.mark_tile_dirty(TileKey::CaveDetail { tx, tz: tz - 1 });
        }
        self.mark_tile_dirty(TileKey::CaveMid2 {
            tx: floor_div(cx, 8),
            tz: floor_div(cz, 8),
        });
    }

    /// Evict unloaded chunks belonging to a different Y slice so old slices never form stale rings.
    pub fn prune_stale_cave_chunks<F>(&mut self, active_y: i32, is_loaded: F)
    where
        F: Fn(i32, i32) -> bool,
    {
        let mut dropped = false;
        self.cave_chunks.retain(|&(cx, cz), &mut (cy, _)| {
            let keep = cy == active_y || is_loaded(cx, cz);
            if !keep {
                dropped = true;
            }
            keep
        });
        if dropped {
            for idx in 0..self.slots.len() {
                if let Some(key @ (TileKey::CaveDetail { .. } | TileKey::CaveMid2 { .. })) =
                    self.slots[idx].key
                {
                    self.slots[idx].dirty = true;
                    if self.dirty_set.insert(key) {
                        self.dirty_queue.push_back(key);
                    }
                }
            }
        }
    }

    /// Keep existing cave data rendered while chunks overwrite progressively in place (zero flicker!).
    pub fn invalidate_cave_tiles(&mut self) {}

    pub fn mark_tile_dirty(&mut self, key: TileKey) {
        if let Some(&slot_idx) = self.key_to_slot.get(&key) {
            self.slots[slot_idx].dirty = true;
            if self.dirty_set.insert(key) {
                self.dirty_queue.push_back(key);
            }
        }
    }

    /// Touch or allocate an LRU slot for rendering `key`. If missing or dirty, returns UV once built,
    /// scheduling immediate patch upload.
    pub fn touch_and_get_uv(&mut self, key: TileKey, max_builds_this_frame: &mut usize) -> Option<SlotUv> {
        let slot_idx = if let Some(&idx) = self.key_to_slot.get(&key) {
            self.slots[idx].last_used = self.frame_seq;
            if self.slots[idx].dirty && *max_builds_this_frame > 0 {
                *max_builds_this_frame -= 1;
                self.build_slot_patch(idx, key);
            }
            idx
        } else {
            if *max_builds_this_frame == 0 {
                return None;
            }
            // Don't even allocate a slot for completely unexplored region/tile.
            if !self.has_any_explored(key) {
                return None;
            }
            *max_builds_this_frame -= 1;
            let idx = self.alloc_slot_for(key);
            self.build_slot_patch(idx, key);
            idx
        };
        Some(Self::slot_uv_in_mod_image(slot_idx))
    }

    /// Drain queued dirty updates inside GPU upload budget.
    pub fn flush_dirty_queue(&mut self, max_builds: usize) {
        let mut built = 0;
        while built < max_builds {
            let Some(key) = self.dirty_queue.pop_front() else {
                break;
            };
            self.dirty_set.remove(&key);
            if let Some(&slot_idx) = self.key_to_slot.get(&key) {
                if self.slots[slot_idx].dirty {
                    self.build_slot_patch(slot_idx, key);
                    built += 1;
                }
            }
        }
    }

    fn alloc_slot_for(&mut self, key: TileKey) -> usize {
        if let Some(empty) = self.slots.iter().position(|s| s.key.is_none()) {
            self.key_to_slot.insert(key, empty);
            self.slots[empty] = SlotMeta {
                key: Some(key),
                last_used: self.frame_seq,
                dirty: false,
            };
            return empty;
        }
        // Evict least recently used slot.
        let mut lru_idx = 0;
        let mut min_seq = u64::MAX;
        for (i, meta) in self.slots.iter().enumerate() {
            if meta.last_used < min_seq {
                min_seq = meta.last_used;
                lru_idx = i;
            }
        }
        if let Some(old_key) = self.slots[lru_idx].key.take() {
            self.key_to_slot.remove(&old_key);
        }
        self.key_to_slot.insert(key, lru_idx);
        self.slots[lru_idx] = SlotMeta {
            key: Some(key),
            last_used: self.frame_seq,
            dirty: false,
        };
        lru_idx
    }

    fn has_any_explored(&self, key: TileKey) -> bool {
        match key {
            TileKey::RegionOverview { rx, rz } => self
                .regions
                .get(&(rx, rz))
                .is_some_and(|r| r.explored_columns > 0),
            TileKey::Mid4 { rx, rz, qx, qz } => self
                .regions
                .get(&(rx, rz))
                .is_some_and(|r| r.has_mid4_tile(qx, qz)),
            TileKey::Mid2 { tx, tz } => {
                let rx = floor_div(tx, 4);
                let rz = floor_div(tz, 4);
                let mx = tx - rx * 4;
                let mz = tz - rz * 4;
                self.regions
                    .get(&(rx, rz))
                    .is_some_and(|r| r.has_mid2_tile(mx, mz))
            }
            TileKey::SurfaceDetail { tx, tz } => {
                let rx = floor_div(tx, 8);
                let rz = floor_div(tz, 8);
                let tlx = tx - rx * 8;
                let tlz = tz - rz * 8;
                self.regions
                    .get(&(rx, rz))
                    .is_some_and(|r| r.has_detail_tile(tlx, tlz))
            }
            TileKey::CaveDetail { tx, tz } => {
                let cx0 = tx * TILE_CHUNKS;
                let cz0 = tz * TILE_CHUNKS;
                for cz in cz0..cz0 + TILE_CHUNKS {
                    for cx in cx0..cx0 + TILE_CHUNKS {
                        if self.cave_chunks.contains_key(&(cx, cz)) {
                            return true;
                        }
                    }
                }
                false
            }
            TileKey::CaveMid2 { tx, tz } => {
                let cx0 = tx * 8;
                let cz0 = tz * 8;
                for cz in cz0..cz0 + 8 {
                    for cx in cx0..cx0 + 8 {
                        if self.cave_chunks.contains_key(&(cx, cz)) {
                            return true;
                        }
                    }
                }
                false
            }
        }
    }

    fn cave_pixel_rgba(&self, wx: i32, wz: i32) -> [u8; 4] {
        let Some(cell) = self.get_cave_column(wx, wz).filter(|c| c.is_explored()) else {
            return [0, 0, 0, 0];
        };
        if (cell.flags & ColumnCell::FLAG_CAVE) != 0 {
            let n = self
                .get_cave_column(wx, wz - 1)
                .filter(|c| (c.flags & ColumnCell::FLAG_CAVE) != 0);
            let w = self
                .get_cave_column(wx - 1, wz)
                .filter(|c| (c.flags & ColumnCell::FLAG_CAVE) != 0);
            let nw_step = self
                .get_cave_column(wx - 1, wz - 1)
                .filter(|c| (c.flags & ColumnCell::FLAG_CAVE) != 0)
                .map_or(0, |p| cell.height - p.height);
            return shade_column_pixel(cell, n, w, nw_step, true);
        }
        let has_adj_cave = [(0, -1), (0, 1), (-1, 0), (1, 0)].into_iter().any(|(dx, dz)| {
            self.get_cave_column(wx + dx, wz + dz)
                .is_some_and(|c| (c.flags & ColumnCell::FLAG_CAVE) != 0)
        });
        if has_adj_cave {
            [42, 42, 50, 240]
        } else {
            [10, 10, 14, 155]
        }
    }

    fn build_slot_patch(&mut self, slot_idx: usize, key: TileKey) {
        self.slots[slot_idx].dirty = false;
        self.dirty_set.remove(&key);

        let stride = SLOT_STRIDE as usize;
        let is_cave_tile = matches!(key, TileKey::CaveDetail { .. } | TileKey::CaveMid2 { .. });
        let mut rgba = vec![0u8; stride * stride * 4];
        if !is_cave_tile {
            for p in rgba.chunks_exact_mut(4) {
                p[3] = 255;
            }
        }

        match key {
            TileKey::SurfaceDetail { tx, tz } => {
                let wx0 = tx * TILE_BLOCKS;
                let wz0 = tz * TILE_BLOCKS;
                let rx = floor_div(wx0, REGION_SIZE);
                let rz = floor_div(wz0, REGION_SIZE);
                let lx0 = wx0 - rx * REGION_SIZE;
                let lz0 = wz0 - rz * REGION_SIZE;
                if let Some(reg) = self.regions.get(&(rx, rz)) {
                    for py in 0..SLOT_INNER as i32 {
                        let lz = lz0 + py;
                        for px in 0..SLOT_INNER as i32 {
                            let lx = lx0 + px;
                            let cell = reg.get(lx, lz);
                            let north = if lz > 0 {
                                Some(reg.get(lx, lz - 1))
                            } else {
                                self.get_column(wx0 + px, wz0 + py - 1)
                            };
                            let west = if lx > 0 {
                                Some(reg.get(lx - 1, lz))
                            } else {
                                self.get_column(wx0 + px - 1, wz0 + py)
                            };
                            let nw = if lz > 0 && lx > 0 {
                                Some(reg.get(lx - 1, lz - 1))
                            } else {
                                self.get_column(wx0 + px - 1, wz0 + py - 1)
                            };
                            let nw_step = nw
                                .filter(|p| p.is_explored())
                                .map_or(0, |p| cell.height - p.height);
                            let px_rgba = shade_column_pixel(cell, north, west, nw_step, false);
                            let dst = ((py as usize + 1) * stride + (px as usize + 1)) * 4;
                            rgba[dst..dst + 4].copy_from_slice(&px_rgba);
                        }
                    }
                }
            }
            TileKey::Mid2 { tx, tz } => {
                let wx_r0 = floor_div(tx, 4) * REGION_SIZE;
                let wz_r0 = floor_div(tz, 4) * REGION_SIZE;
                let rx = floor_div(tx, 4);
                let rz = floor_div(tz, 4);
                let mx = tx - rx * 4;
                let mz = tz - rz * 4;
                let lx0 = mx * 128;
                let lz0 = mz * 128;
                if let Some(reg) = self.regions.get(&(rx, rz)) {
                    for py in 0..SLOT_INNER as i32 {
                        for px in 0..SLOT_INNER as i32 {
                            let px_rgba = downsample_region_box(
                                reg,
                                Some(self),
                                wx_r0,
                                wz_r0,
                                lx0 + px * 2,
                                lz0 + py * 2,
                                2,
                            );
                            let dst = ((py as usize + 1) * stride + (px as usize + 1)) * 4;
                            rgba[dst..dst + 4].copy_from_slice(&px_rgba);
                        }
                    }
                }
            }
            TileKey::Mid4 { rx, rz, qx, qz } => {
                let wx_r0 = rx * REGION_SIZE;
                let wz_r0 = rz * REGION_SIZE;
                let lx0 = i32::from(qx) * 256;
                let lz0 = i32::from(qz) * 256;
                if let Some(reg) = self.regions.get(&(rx, rz)) {
                    for py in 0..SLOT_INNER as i32 {
                        for px in 0..SLOT_INNER as i32 {
                            let px_rgba = downsample_region_box(
                                reg,
                                Some(self),
                                wx_r0,
                                wz_r0,
                                lx0 + px * 4,
                                lz0 + py * 4,
                                4,
                            );
                            let dst = ((py as usize + 1) * stride + (px as usize + 1)) * 4;
                            rgba[dst..dst + 4].copy_from_slice(&px_rgba);
                        }
                    }
                }
            }
            TileKey::CaveDetail { tx, tz } => {
                let wx0 = tx * TILE_BLOCKS;
                let wz0 = tz * TILE_BLOCKS;
                for py in 0..SLOT_INNER as i32 {
                    for px in 0..SLOT_INNER as i32 {
                        let px_rgba = self.cave_pixel_rgba(wx0 + px, wz0 + py);
                        let dst = ((py as usize + 1) * stride + (px as usize + 1)) * 4;
                        rgba[dst..dst + 4].copy_from_slice(&px_rgba);
                    }
                }
            }
            TileKey::CaveMid2 { tx, tz } => {
                let wx0 = tx * 128;
                let wz0 = tz * 128;
                for py in 0..SLOT_INNER as i32 {
                    for px in 0..SLOT_INNER as i32 {
                        let bx = wx0 + px * 2;
                        let bz = wz0 + py * 2;
                        let mut cr = 0u32;
                        let mut cg = 0u32;
                        let mut cb = 0u32;
                        let mut cn = 0u32;
                        let mut wall = false;
                        let mut rock = false;
                        for dz in 0..2 {
                            for dx in 0..2 {
                                let p = self.cave_pixel_rgba(bx + dx, bz + dz);
                                match p[3] {
                                    255 => {
                                        cr += u32::from(p[0]);
                                        cg += u32::from(p[1]);
                                        cb += u32::from(p[2]);
                                        cn += 1;
                                    }
                                    240 => wall = true,
                                    155 => rock = true,
                                    _ => {}
                                }
                            }
                        }
                        let px_rgba = if cn > 0 {
                            [(cr / cn) as u8, (cg / cn) as u8, (cb / cn) as u8, 255]
                        } else if wall {
                            [42, 42, 50, 240]
                        } else if rock {
                            [10, 10, 14, 155]
                        } else {
                            [0, 0, 0, 0]
                        };
                        let dst = ((py as usize + 1) * stride + (px as usize + 1)) * 4;
                        rgba[dst..dst + 4].copy_from_slice(&px_rgba);
                    }
                }
            }
            TileKey::RegionOverview { rx, rz } => {
                if let Some(reg) = self.regions.get_mut(&(rx, rz)) {
                    reg.ensure_overview();
                    for py in 0..OVERVIEW_SIZE {
                        for px in 0..OVERVIEW_SIZE {
                            let px_rgba = reg.overview_rgba[py * OVERVIEW_SIZE + px];
                            let dst = ((py + 1) * stride + (px + 1)) * 4;
                            rgba[dst..dst + 4].copy_from_slice(&px_rgba);
                        }
                    }
                }
            }
        }

        // Clamped 1px gutter extrusion prevents bilinear/edge UV bleeding when tiles rotate.
        let inner = SLOT_INNER as usize;
        for py in 1..=inner {
            let row_start = py * stride * 4;
            let first = row_start + 4;
            let last = row_start + inner * 4;
            rgba.copy_within(first..first + 4, row_start);
            rgba.copy_within(last..last + 4, row_start + (inner + 1) * 4);
        }
        let top_src = stride * 4;
        rgba.copy_within(top_src..top_src + stride * 4, 0);
        let bot_src = inner * stride * 4;
        let bot_dst = (inner + 1) * stride * 4;
        rgba.copy_within(bot_src..bot_src + stride * 4, bot_dst);

        let (ox, oy) = Self::slot_pixel_origin(slot_idx);
        self.pending_patches.push(PendingPatch {
            x: ox,
            y: oy,
            width: SLOT_STRIDE,
            height: SLOT_STRIDE,
            rgba,
        });
    }
}
