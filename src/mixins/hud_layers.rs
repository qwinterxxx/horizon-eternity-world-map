fn on_hud_layers(game: &mut Game<'_>, mut batch: HudBatch) -> HudBatch {
    crate::game::eternity_world_map_bridge::on_hud_layers(game, &mut batch);
    batch
}
