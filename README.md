# Eternity world map

![Eternity world map icon](icon.png)

Zero-overhead native minimap, persistent fullscreen World Map, Cave Mode, 2.5D mob head radar, and interactive 3D/radar waypoints built from scratch in Rust for [Horizon](https://github.com/horizon-rust/horizon) (Minecraft `26.3`).

## Features

- **Minimap HUD**
  - Square and smooth Round minimap modes with configurable size (`64px–256px`), zoom, corner placement, and player rotation lock.
  - Slope-shaded biome-accurate terrain coloring plus crisp cardinal badges (`N/E/S/W` / `С/В/Ю/З`).
  - Zero-allocation LRU GPU tile atlas cache (`1024x1024`) with budgeted incremental chunk scanning.
- **Persistent Fullscreen World Map (`M`)**
  - Multi-tier mip-pyramid (`1:1`, `2:1`, `4:1`, `8:1`) prevents zoom-out shimmering and blur.
  - Background `512x512` area preload/scan button, chunk grid overlay, cursor biome & coordinate readout, and right-click context actions (`Teleport`, `Copy coords`, `Create waypoint`).
- **Cave Mode**
  - Automatic surface/roof detection plus a World Map switcher (`Auto` / `Manual` / `Off` and Y-slice slider `-64..320`), filtering open sky above the cave ceiling so surface terrain never bleeds in.
- **2.5D Mob & Player Radar**
  - Compact entity diamonds by default; hold `Tab` to show crisp vanilla-accurate multi-layer mob heads (`3D` snouts, beaks, noses, ears, horns, emissive eye overlays, and baby texture sizing) and player skins.
- **Waypoints**
  - Click any coordinate on the World Map (`M`) or press `B` to create named color-coded waypoints with in-world 3D beams, minimap rim clamping, and optional automatic Death waypoints.

## Controls

| Key | Action |
| --- | --- |
| **M** | Open / close fullscreen World Map |
| **B** | Create waypoint at player position |
| **+** / **-** | Zoom minimap in / out |
| **H** | Show / hide minimap |
| **C** | Cycle Cave Mode (`Auto` / `Manual` / `Off`) |
| **Tab** | Hold for mob & player head icons on radar/map |

All bindings can be changed in **Options > Controls**. Settings are available both on the World Map (`⚙`) and in **Options > Mods > Eternity world map**.

## Compatibility

- Pinned to Horizon Nightly `2026-11-20` (`0a875da91dbb0fdcb7ba8c42c11341aa229af51a`) and `horizon-mixins` `e59da9094f0c2fdda6c627fe9eff22dec6a906f0`.

## Build & Verify

```sh
cargo horizon-mod check target/horizon-mod/eternity_world_map.hmod
cargo test --lib
horizon-launcher native-build --instance "<instance>" --trust-native-code
```

Or download `eternity_world_map-0.1.0.hmod` from [Releases](https://github.com/qwinterxxx/eternity-world-map/releases) and verify its signature with [`horizon-mods.pub`](horizon-mods.pub):

```sh
minisign -Vm eternity_world_map-0.1.0.hmod -P RWQjqg8rypRO1FcAIBc+P7vxdFaU1fjCss8evaMaYSBDL21Fy12PGnH8
```

## License

MIT — see [LICENSE](LICENSE).
