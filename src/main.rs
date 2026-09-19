//! Chessie — play chess against an engine at a rating of your choosing,
//! on an oversized board in a courtyard you can walk around.

mod board;
mod camera;
mod chess;
mod engine;
mod eval;
mod interaction;
mod models;
mod paths;
mod pieces;
mod strength;
mod ui;
mod world;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::winit::{UpdateMode, WinitSettings};
use std::time::Duration;

use crate::chess::{Game, HumanSide};
use crate::engine::Engine;
use crate::interaction::{LastMoveText, Selection};
use crate::strength::{Rating, Rng};

fn main() {
    // Bevy resolves assets relative to BEVY_ASSET_ROOT, falling back to the
    // cargo manifest and then the executable's directory. Point it at
    // whichever of those actually holds the files, so the app works the same
    // from a bundle, a folder, or `cargo run`.
    if std::env::var_os("BEVY_ASSET_ROOT").is_none() {
        if let Some(root) = std::env::current_exe().ok().and_then(|exe| paths::asset_root(&exe)) {
            std::env::set_var("BEVY_ASSET_ROOT", root);
        }
    }

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Chessie".into(),
                resolution: (1440u32, 900u32).into(),
                ..default()
            }),
            ..default()
        }))
        // Mesh picking is not part of DefaultPlugins; we need it to click pieces.
        .add_plugins(MeshPickingPlugin)
        .insert_resource(ui_picking_settings())
        // Paced at runtime by `pace_frames`: full rate while something is
        // happening, eased off while the board waits for you.
        .insert_resource(WinitSettings {
            focused_mode: UpdateMode::Continuous,
            unfocused_mode: UpdateMode::reactive(Duration::from_secs_f64(IDLE_HEARTBEAT)),
        })
        .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default())
        .init_resource::<Game>()
        .init_resource::<HumanSide>()
        .init_resource::<Rating>()
        .init_resource::<Rng>()
        .init_resource::<Selection>()
        .init_resource::<LastMoveText>()
        .init_resource::<camera::BoardCamera>()
        .init_resource::<ui::ShowPerf>()
        .init_resource::<ui::ShowHelp>()
        .init_resource::<chess::Review>()
        .init_resource::<ui::RatingEntry>()
        .init_resource::<interaction::Dragging>()
        .init_resource::<board::DroppedFrom>()
        .init_resource::<interaction::PendingPromotion>()
        .insert_resource(Engine::launch(true))
        .insert_resource(eval::Analyser(Engine::launch(false)))
        .init_resource::<eval::Evaluation>()
        .add_systems(
            Startup,
            (
                board::setup_board_assets,
                world::setup_world,
                camera::spawn_camera,
                ui::setup_hud,
                models::load_piece_models,
                configure_engine,
            ),
        )
        .add_observer(interaction::on_click)
        .add_observer(interaction::on_drag_start)
        .add_observer(interaction::on_drag_end)
        .add_systems(
            Update,
            (
                // Input and camera.
                (camera::orbit_camera, camera::drive_camera),
                // Game flow.
                //
                // A modal reader has to run *after* everything it suppresses.
                // Those systems skip while the modal is open, but the modal
                // closes itself on the same keypress — so if it went first,
                // they would wake to find it shut and act on that key too.
                // Pressing R to promote to a rook would promote and then
                // restart the game; Q would promote and zoom the camera.
                (
                    ui::rating_entry_input.after(ui::restart_game),
                    ui::adjust_rating,
                    ui::review_input,
                    ui::restart_game,
                    ui::swap_sides,
                    interaction::promotion_input
                        .after(ui::restart_game)
                        .after(ui::swap_sides)
                        .after(ui::review_input)
                        .after(ui::rating_entry_input)
                        .after(ui::toggle_help)
                        .after(camera::orbit_camera),
                    interaction::drag_piece,
                    interaction::engine_turn,
                    interaction::engine_reply,
                    eval::request_analysis,
                    eval::receive_analysis,
                ),
                // Presentation.
                (
                    models::install_piece_models,
                    board::sync_pieces,
                    board::sync_captured,
                    board::animate_moves,
                    board::animate_captures,
                    interaction::update_highlights,
                    ui::update_hud,
                    ui::update_eval_bar,
                    ui::toggle_help,
                    ui::update_promotion_panel,
                    ui::update_perf,
                    interaction::clear_drag_flag,
                    pace_frames,
                ),
            ),
        )
        .run();
}

/// Push the starting rating into the engine once it is up.
fn configure_engine(engine: Res<Engine>, rating: Res<Rating>) {
    engine.new_game();
    engine.set_elo(rating.0);
}

/// The HUD is read-only: panels, the evaluation bar, the hint line. None of it
/// is meant to be clicked, and by default every UI node blocks picking of the
/// meshes behind it, so the panels were swallowing clicks on the pieces under
/// them. Requiring an explicit marker turns that off for everything; nothing
/// in this app opts back in.
fn ui_picking_settings() -> UiPickingSettings {
    UiPickingSettings {
        require_markers: true,
    }
}

/// How long everything must be still before drawing stops.
const IDLE_AFTER: f32 = 0.35;
/// Once idle the app only redraws on input. This is just a safety heartbeat in
/// case something changes without an event to announce it.
const IDLE_HEARTBEAT: f64 = 1.0;

/// Nothing in this scene animates by itself, so once you stop touching it there
/// is nothing new to draw. This puts the app to sleep until you do something,
/// the way a conventional chess app does, and wakes it instantly on input.
fn pace_frames(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    engine: Res<Engine>,
    analyser: Res<eval::Analyser>,
    board_camera: Res<camera::BoardCamera>,
    views: Query<&Transform, With<camera::BoardView>>,
    animating: Query<(), Or<(With<board::MoveAnimation>, With<board::Capturing>)>>,
    mut settings: ResMut<WinitSettings>,
    mut idle_since: Local<Option<f32>>,
    mut was_idle: Local<bool>,
) {
    let now = time.elapsed_secs();

    // The camera eases toward its target, so it counts as busy until it lands,
    // otherwise drawing would stop mid-swing.
    let camera_moving = views
        .single()
        .map(|tf| camera::camera_is_moving(&board_camera, tf))
        .unwrap_or(false);

    let busy = keys.get_pressed().next().is_some()
        || buttons.get_pressed().next().is_some()
        || motion.delta != Vec2::ZERO
        || scroll.delta != Vec2::ZERO
        || engine.thinking
        || analyser.0.thinking
        || camera_moving
        || !animating.is_empty();

    if busy {
        *idle_since = None;
    } else if idle_since.is_none() {
        *idle_since = Some(now);
    }

    let idle = idle_since.map(|t| now - t > IDLE_AFTER).unwrap_or(false);
    if idle != *was_idle {
        *was_idle = idle;
        // Reactive still wakes instantly on input, so this costs no latency.
        settings.focused_mode = if idle {
            UpdateMode::reactive(Duration::from_secs_f64(IDLE_HEARTBEAT))
        } else {
            UpdateMode::Continuous
        };
    }
}

#[cfg(test)]
mod picking_tests {
    use super::*;

    #[test]
    fn the_hud_does_not_swallow_clicks_on_the_board() {
        // Bevy's UI picking backend defaults to `require_markers: false`,
        // which makes every UI node block picking of the meshes behind it.
        // The panels sit over the board, so the pieces under them were
        // unclickable. Requiring a marker disables UI picking entirely here,
        // because nothing in this app asks for it.
        assert!(
            ui_picking_settings().require_markers,
            "UI picking would block clicks on the pieces beneath the HUD"
        );
    }
}
