//! Heads-up display: opponent rating, game status, and controls.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use shakmaty::Color as ChessColor;

use crate::camera::BoardCamera;
use crate::chess::{Game, GameEnd, HumanSide, Review};
use crate::engine::{Engine, ENGINE_MAX_ELO, UI_MIN_ELO};
use crate::eval::Evaluation;
use crate::interaction::{LastMoveText, PendingPromotion, Selection, PROMOTION_CHOICES};
use crate::strength::Rating;

#[derive(Component)]
pub struct StatusText;
#[derive(Component)]
pub struct HelpPanel;
#[derive(Component)]
pub struct PromotionPanel;
#[derive(Component)]
pub struct PromotionText;

/// Everything the keyboard does, kept out of the way behind `H`.
const HELP_TEXT: &str = "\
Controls

  Click or drag a piece      move
  Q R B N                    choose the piece when a pawn promotes
  Right-drag / W A S D       look around
  Scroll / [  ]              zoom
  Left / Right               step through the game
  Up / Down                  jump to the start / back to live
  T                          type the opponent's rating
  -  =                       nudge the rating (hold to repeat)
  F                          swap sides
  R                          restart
  F3                         performance
  H                          close this";

/// Which corner a floating panel is pinned to. Named so two panels cannot
/// quietly end up stacked on top of each other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
}

/// Gap between a pinned panel and the edges of the window.
const PANEL_INSET: f32 = 12.0;

/// The status panel stays in the corner nearest the board's own side.
pub const STATUS_CORNER: Corner = Corner::TopLeft;
/// The controls list sits on the right, clear of the status panel it used to
/// cover.
pub const HELP_CORNER: Corner = Corner::TopRight;
/// The performance readout moves out of its way, to the far corner.
pub const PERF_CORNER: Corner = Corner::BottomRight;

/// Absolute positioning for a panel pinned to `corner`.
fn pinned(corner: Corner) -> Node {
    let mut node = Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    match corner {
        Corner::TopLeft => {
            node.top = Val::Px(10.0);
            node.left = Val::Px(EVAL_BAR_WIDTH + 10.0);
        }
        Corner::TopRight => {
            node.top = Val::Px(10.0);
            node.right = Val::Px(PANEL_INSET);
        }
        Corner::BottomRight => {
            node.bottom = Val::Px(PANEL_INSET);
            node.right = Val::Px(PANEL_INSET);
        }
    }
    node
}

/// Whether the help panel is showing.
#[derive(Resource, Default)]
pub struct ShowHelp(pub bool);
#[derive(Component)]
pub struct HintText;
#[derive(Component)]
pub struct EvalWhite;
#[derive(Component)]
pub struct EvalBlack;
#[derive(Component)]
pub struct EvalLabel;
/// Width of the bar down the left edge.
const EVAL_BAR_WIDTH: f32 = 26.0;

#[derive(Component)]
pub struct PerfText;
#[derive(Component)]
pub struct PerfPanel;

/// Typing a rating directly, rather than nudging it a step at a time.
#[derive(Resource, Default)]
pub struct RatingEntry {
    pub active: bool,
    pub buffer: String,
}

impl RatingEntry {
    /// What the player has typed so far, clamped into the supported range.
    pub fn parsed(&self) -> Option<u32> {
        self.buffer
            .parse::<u32>()
            .ok()
            .map(|v| v.clamp(UI_MIN_ELO, ENGINE_MAX_ELO))
    }

    pub fn begin(&mut self) {
        self.active = true;
        self.buffer.clear();
    }

    pub fn cancel(&mut self) {
        self.active = false;
        self.buffer.clear();
    }

    pub fn push_digit(&mut self, c: char) {
        // Four digits covers the whole range.
        if self.buffer.len() < 4 {
            self.buffer.push(c);
        }
    }
}

/// Number keys, top row and numpad.
const DIGIT_KEYS: [(KeyCode, char); 20] = [
    (KeyCode::Digit0, '0'),
    (KeyCode::Digit1, '1'),
    (KeyCode::Digit2, '2'),
    (KeyCode::Digit3, '3'),
    (KeyCode::Digit4, '4'),
    (KeyCode::Digit5, '5'),
    (KeyCode::Digit6, '6'),
    (KeyCode::Digit7, '7'),
    (KeyCode::Digit8, '8'),
    (KeyCode::Digit9, '9'),
    (KeyCode::Numpad0, '0'),
    (KeyCode::Numpad1, '1'),
    (KeyCode::Numpad2, '2'),
    (KeyCode::Numpad3, '3'),
    (KeyCode::Numpad4, '4'),
    (KeyCode::Numpad5, '5'),
    (KeyCode::Numpad6, '6'),
    (KeyCode::Numpad7, '7'),
    (KeyCode::Numpad8, '8'),
    (KeyCode::Numpad9, '9'),
];

/// `T` opens a box to type the opponent's rating; Enter accepts it.
pub fn rating_entry_input(
    keys: Res<ButtonInput<KeyCode>>,
    promotion: Res<PendingPromotion>,
    mut entry: ResMut<RatingEntry>,
    mut rating: ResMut<Rating>,
    engine: Res<Engine>,
) {
    if promotion.is_open() {
        return;
    }
    if !entry.active {
        if keys.just_pressed(KeyCode::KeyT) {
            entry.begin();
        }
        return;
    }

    for (key, ch) in DIGIT_KEYS {
        if keys.just_pressed(key) {
            entry.push_digit(ch);
        }
    }
    if keys.just_pressed(KeyCode::Backspace) {
        entry.buffer.pop();
    }
    if keys.just_pressed(KeyCode::Escape) {
        entry.cancel();
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        if let Some(value) = entry.parsed() {
            rating.0 = value;
            engine.set_elo(value);
        }
        entry.cancel();
    }
}

/// Single letter for a piece, as used on the captured piles.
fn piece_letter(role: shakmaty::Role) -> char {
    match role {
        shakmaty::Role::Pawn => 'P',
        shakmaty::Role::Knight => 'N',
        shakmaty::Role::Bishop => 'B',
        shakmaty::Role::Rook => 'R',
        shakmaty::Role::Queen => 'Q',
        shakmaty::Role::King => 'K',
    }
}

/// F3 toggles the performance readout.
#[derive(Resource, Default)]
pub struct ShowPerf(pub bool);

pub fn setup_hud(mut commands: Commands) {
    // One small panel, low contrast, close to the corner. The board is the
    // thing worth looking at.
    commands.spawn((
        Node {
            padding: UiRect::axes(Val::Px(9.0), Val::Px(6.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            ..pinned(STATUS_CORNER)
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.34)),
        children![(
            Text::new(""),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.88)),
            StatusText,
        )],
    ));

    // The rest of the detail lives behind H, out of the way until wanted.
    commands.spawn((
        Node {
            padding: UiRect::axes(Val::Px(11.0), Val::Px(9.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            display: Display::None,
            ..pinned(HELP_CORNER)
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.76)),
        HelpPanel,
        children![(
            Text::new(HELP_TEXT),
            TextFont {
                font_size: FontSize::Px(14.0),
                ..default()
            },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.92)),
        )],
    ));

    // The promotion picker, centred and unmissable when it matters.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(70.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            display: Display::None,
            ..default()
        },
        PromotionPanel,
        children![(
            Node {
                padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.92)),
            children![(
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(19.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.87, 0.5)),
                PromotionText,
            )],
        )],
    ));

    // A single dim line, rather than a running list of controls.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(14.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![(
            Text::new(""),
            TextFont {
                font_size: FontSize::Px(13.0),
                ..default()
            },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.5)),
            HintText,
        )],
    ));

    // Evaluation bar down the left edge: White fills from the bottom, Black
    // from the top, so the taller half is the side that is better off.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Px(EVAL_BAR_WIDTH),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        BackgroundColor(Color::srgb(0.10, 0.10, 0.12)),
        children![
            (
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(50.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.14, 0.13, 0.16)),
                EvalBlack,
            ),
            (
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(50.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::FlexStart,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.90, 0.88, 0.82)),
                EvalWhite,
                children![(
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.15, 0.15, 0.15)),
                    EvalLabel,
                )],
            ),
        ],
    ));

    // Performance readout, top right, hidden until F3.
    commands.spawn((
        Node {
            padding: UiRect::all(Val::Px(9.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            display: Display::None,
            ..pinned(PERF_CORNER)
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        PerfPanel,
        children![(
            Text::new(""),
            TextFont {
                font_size: FontSize::Px(13.0),
                ..default()
            },
            TextColor(Color::srgb(0.6, 1.0, 0.7)),
            PerfText,
        )],
    ));
}

/// `-` and `=` adjust the opponent's rating; hold Shift for bigger steps.
pub fn adjust_rating(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    entry: Res<RatingEntry>,
    mut rating: ResMut<Rating>,
    mut held: Local<f32>,
    mut since_repeat: Local<f32>,
    engine: Res<Engine>,
) {
    if entry.active {
        return;
    }

    let up = keys.pressed(KeyCode::Equal) || keys.pressed(KeyCode::NumpadAdd);
    let down = keys.pressed(KeyCode::Minus) || keys.pressed(KeyCode::NumpadSubtract);
    let direction: i32 = match (up, down) {
        (true, false) => 1,
        (false, true) => -1,
        _ => {
            *held = 0.0;
            *since_repeat = 0.0;
            return;
        }
    };

    let tapped = keys.just_pressed(KeyCode::Equal)
        || keys.just_pressed(KeyCode::NumpadAdd)
        || keys.just_pressed(KeyCode::Minus)
        || keys.just_pressed(KeyCode::NumpadSubtract);

    let dt = time.delta_secs();
    *held += dt;
    *since_repeat += dt;

    // A tap nudges once. Holding starts repeating, and the longer it is held
    // the bigger each step, so crossing the whole range takes a moment rather
    // than a hundred presses.
    let step = if tapped {
        *since_repeat = 0.0;
        25
    } else if *held > HOLD_DELAY && *since_repeat >= REPEAT_INTERVAL {
        *since_repeat = 0.0;
        match *held {
            h if h < 1.0 => 25,
            h if h < 2.0 => 50,
            h if h < 3.0 => 100,
            _ => 200,
        }
    } else {
        return;
    };

    let step = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        step * 4
    } else {
        step
    };

    rating.0 = if direction > 0 {
        (rating.0 + step).min(ENGINE_MAX_ELO)
    } else {
        rating.0.saturating_sub(step).max(UI_MIN_ELO)
    };
    engine.set_elo(rating.0);
}

/// How long a key must be held before it starts repeating.
const HOLD_DELAY: f32 = 0.35;
/// Gap between repeats once it starts.
const REPEAT_INTERVAL: f32 = 0.06;

/// Arrow keys walk back and forth through the game, so a move you missed can
/// be replayed. The live game carries on underneath.
pub fn review_input(
    keys: Res<ButtonInput<KeyCode>>,
    promotion: Res<PendingPromotion>,
    game: Res<Game>,
    mut review: ResMut<Review>,
) {
    if promotion.is_open() {
        return;
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        review.step_back(&game);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        review.step_forward(&game);
    }
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::Home) {
        review.go_to_start(&game);
    }
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::End) {
        review.go_live();
    }
}

/// `F` swaps which colour you play and begins a fresh game. Changing sides
/// mid-game would leave the engine playing against its own position, so this
/// starts over.
pub fn swap_sides(
    keys: Res<ButtonInput<KeyCode>>,
    promotion: Res<PendingPromotion>,
    entry: Res<RatingEntry>,
    mut human: ResMut<HumanSide>,
    mut game: ResMut<Game>,
    mut selection: ResMut<Selection>,
    mut review: ResMut<Review>,
    mut last_move: ResMut<LastMoveText>,
    mut camera: ResMut<BoardCamera>,
    engine: Res<Engine>,
) {
    if promotion.is_open() || entry.active || !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    human.0 = !human.0;
    game.reset();
    review.go_live();
    selection.clear();
    last_move.0.clear();
    engine.new_game();
    // Look at the board from your own end of it.
    camera.face(human.0);
}

/// `R` starts a fresh game at the current rating.
pub fn restart_game(
    keys: Res<ButtonInput<KeyCode>>,
    promotion: Res<PendingPromotion>,
    entry: Res<RatingEntry>,
    mut game: ResMut<Game>,
    mut selection: ResMut<Selection>,
    mut last_move: ResMut<LastMoveText>,
    mut review: ResMut<Review>,
    engine: Res<Engine>,
) {
    if promotion.is_open() || entry.active || !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    game.reset();
    review.go_live();
    selection.clear();
    last_move.0.clear();
    engine.new_game();
}

pub fn update_hud(
    evaluation: Res<Evaluation>,
    entry: Res<RatingEntry>,
    help: Res<ShowHelp>,
    promotion: Res<PendingPromotion>,
    game: Res<Game>,
    review: Res<Review>,
    rating: Res<Rating>,
    human: Res<HumanSide>,
    engine: Res<Engine>,
    last_move: Res<LastMoveText>,
    mut status_text: Query<&mut Text, (With<StatusText>, Without<HintText>)>,
    mut hint_text: Query<&mut Text, (With<HintText>, Without<StatusText>)>,
) {
    let side = |c: ChessColor| match c {
        ChessColor::White => "White",
        ChessColor::Black => "Black",
    };

    if let Ok(mut text) = status_text.single_mut() {
        let mut lines: Vec<String> = Vec::new();

        // Line one: who you are and who you are playing.
        if entry.active {
            let typed = if entry.buffer.is_empty() {
                "____".to_string()
            } else {
                entry.buffer.clone()
            };
            lines.push(format!("Opponent: {typed} Elo — Enter to set"));
        } else {
            lines.push(format!("{} vs {} Elo", side(human.0), rating.0));
        }

        // Line two: what is happening.
        lines.push(if !engine.available {
            "Engine unavailable".to_string()
        } else if let Some(ply) = review.ply {
            format!(
                "Reviewing move {} of {}",
                ply.div_ceil(2).max(1),
                game.history.len().div_ceil(2).max(1)
            )
        } else {
            match game.ended {
                Some(GameEnd::Checkmate { winner }) => {
                    let who = if winner == human.0 {
                        "You win"
                    } else {
                        "You lose"
                    };
                    format!("Checkmate — {who}")
                }
                Some(GameEnd::Stalemate) => "Stalemate — draw".to_string(),
                Some(GameEnd::InsufficientMaterial) => "Draw — insufficient material".to_string(),
                None if game.turn() == human.0 => {
                    if game.is_in_check() {
                        "Your move — check!".to_string()
                    } else {
                        "Your move".to_string()
                    }
                }
                None if engine.thinking => "Thinking…".to_string(),
                None => "Opponent to move".to_string(),
            }
        });

        // Line three: the material picture, in one line.
        let shown = review.shown_ply(&game);
        let balance = game.material_balance_at(shown);
        let yours = match human.0 {
            ChessColor::White => balance,
            ChessColor::Black => -balance,
        };
        let losses = game.losses_at(shown);
        let (mine, theirs) = match human.0 {
            ChessColor::White => (&losses.white, &losses.black),
            ChessColor::Black => (&losses.black, &losses.white),
        };
        let pile = |roles: &[shakmaty::Role]| -> String {
            roles.iter().map(|r| piece_letter(*r)).collect::<String>()
        };
        let lead = match yours {
            0 => "level".to_string(),
            n if n > 0 => format!("+{n}"),
            n => format!("{n}"),
        };
        lines.push(format!(
            "{lead}   you {}   them {}",
            if mine.is_empty() {
                "–".into()
            } else {
                pile(mine)
            },
            if theirs.is_empty() {
                "–".into()
            } else {
                pile(theirs)
            },
        ));

        if !last_move.0.is_empty() && review.ply.is_none() {
            lines.push(last_move.0.clone());
        }

        **text = lines.join("\n");
    }

    if let Ok(mut text) = hint_text.single_mut() {
        **text = if promotion.is_open() {
            String::new()
        } else if help.0 {
            String::new()
        } else if review.is_reviewing() {
            "Down to return to live   ·   H for controls".to_string()
        } else {
            let _ = &evaluation;
            "H for controls".to_string()
        };
    }
}

/// `H` shows and hides the controls.
pub fn toggle_help(
    keys: Res<ButtonInput<KeyCode>>,
    entry: Res<RatingEntry>,
    promotion: Res<PendingPromotion>,
    mut help: ResMut<ShowHelp>,
    mut panels: Query<&mut Node, With<HelpPanel>>,
) {
    if !entry.active && !promotion.is_open() && keys.just_pressed(KeyCode::KeyH) {
        help.0 = !help.0;
    }
    if !help.is_changed() {
        return;
    }
    for mut node in &mut panels {
        node.display = if help.0 { Display::Flex } else { Display::None };
    }
}

/// Shows the promotion picker while a pawn is waiting to be named.
pub fn update_promotion_panel(
    promotion: Res<PendingPromotion>,
    mut panels: Query<&mut Node, With<PromotionPanel>>,
    mut text: Query<&mut Text, With<PromotionText>>,
) {
    if !promotion.is_changed() {
        return;
    }
    let open = promotion.is_open();
    for mut node in &mut panels {
        node.display = if open { Display::Flex } else { Display::None };
    }
    if !open {
        return;
    }
    if let Ok(mut text) = text.single_mut() {
        let choices: Vec<String> = PROMOTION_CHOICES
            .iter()
            .map(|role| format!("{} {}", piece_letter(*role), role_name(*role)))
            .collect();
        **text = format!("Promote to:   {}   ·   Esc to cancel", choices.join("   "));
    }
}

/// Full name of a piece, for the promotion picker.
fn role_name(role: shakmaty::Role) -> &'static str {
    match role {
        shakmaty::Role::Queen => "Queen",
        shakmaty::Role::Rook => "Rook",
        shakmaty::Role::Bishop => "Bishop",
        shakmaty::Role::Knight => "Knight",
        shakmaty::Role::Pawn => "Pawn",
        shakmaty::Role::King => "King",
    }
}

/// F3 shows frame timing and how much is on screen.
pub fn update_perf(
    keys: Res<ButtonInput<KeyCode>>,
    mut show: ResMut<ShowPerf>,
    diagnostics: Res<DiagnosticsStore>,
    entities: Query<Entity>,
    meshes: Query<&Mesh3d>,
    lights: Query<&PointLight>,
    mut panels: Query<&mut Node, With<PerfPanel>>,
    mut text: Query<&mut Text, With<PerfText>>,
) {
    if keys.just_pressed(KeyCode::F3) {
        show.0 = !show.0;
    }
    for mut node in &mut panels {
        node.display = if show.0 { Display::Flex } else { Display::None };
    }
    if !show.0 {
        return;
    }
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let frame_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    if let Ok(mut text) = text.single_mut() {
        **text = format!(
            "{fps:.0} fps   {frame_ms:.2} ms\nentities {}   meshes {}\nshadow-casting lights {}",
            entities.iter().count(),
            meshes.iter().count(),
            lights.iter().filter(|l| l.shadow_maps_enabled).count(),
        );
    }
}

#[cfg(test)]
mod rating_entry_tests {
    use super::*;

    fn typed(digits: &str) -> RatingEntry {
        let mut entry = RatingEntry::default();
        entry.begin();
        for c in digits.chars() {
            entry.push_digit(c);
        }
        entry
    }

    #[test]
    fn a_typed_rating_is_accepted_as_written() {
        assert_eq!(typed("1750").parsed(), Some(1750));
        assert_eq!(typed("900").parsed(), Some(900));
    }

    #[test]
    fn typing_above_or_below_the_range_is_clamped_not_rejected() {
        assert_eq!(typed("9999").parsed(), Some(ENGINE_MAX_ELO));
        assert_eq!(typed("1").parsed(), Some(UI_MIN_ELO));
    }

    #[test]
    fn an_empty_box_commits_nothing() {
        assert_eq!(typed("").parsed(), None);
    }

    #[test]
    fn the_box_holds_at_most_four_digits() {
        let entry = typed("1234567");
        assert_eq!(entry.buffer, "1234");
    }

    #[test]
    fn cancelling_clears_the_box_and_closes_it() {
        let mut entry = typed("2200");
        entry.cancel();
        assert!(!entry.active);
        assert!(entry.buffer.is_empty());
    }

    #[test]
    fn reopening_starts_from_a_clean_box() {
        let mut entry = typed("2200");
        entry.begin();
        assert!(entry.buffer.is_empty(), "stale digits carried over");
    }

    #[test]
    fn every_reachable_rating_can_be_typed_in_four_digits() {
        for value in [UI_MIN_ELO, 1000, 1320, 2500, ENGINE_MAX_ELO] {
            let text = value.to_string();
            assert!(text.len() <= 4, "{value} needs more than four digits");
            assert_eq!(typed(&text).parsed(), Some(value));
        }
    }
}

/// Sizes the two halves of the bar from the latest evaluation.
pub fn update_eval_bar(
    evaluation: Res<Evaluation>,
    mut white: Query<&mut Node, (With<EvalWhite>, Without<EvalBlack>)>,
    mut black: Query<&mut Node, (With<EvalBlack>, Without<EvalWhite>)>,
    mut label: Query<&mut Text, With<EvalLabel>>,
) {
    if !evaluation.is_changed() {
        return;
    }
    let chance = evaluation.white_chance().clamp(0.0, 1.0);
    if let Ok(mut node) = white.single_mut() {
        node.height = Val::Percent(chance * 100.0);
    }
    if let Ok(mut node) = black.single_mut() {
        node.height = Val::Percent((1.0 - chance) * 100.0);
    }
    if let Ok(mut text) = label.single_mut() {
        **text = evaluation.label();
    }
}

#[cfg(test)]
mod hud_tests {
    use super::*;

    #[test]
    fn the_help_text_covers_every_key_the_game_listens_for() {
        // If a binding is added and not documented here, it is undiscoverable
        // now that the controls are hidden behind H.
        for key in [
            "Click or drag",
            "Right-drag",
            "Scroll",
            "Left / Right",
            "Up / Down",
            "T",
            "F",
            "R",
            "F3",
            "H",
        ] {
            assert!(HELP_TEXT.contains(key), "help does not mention {key}");
        }
    }

    #[test]
    fn the_controls_sit_on_the_right() {
        assert!(
            matches!(HELP_CORNER, Corner::TopRight | Corner::BottomRight),
            "the controls panel should be on the right of the screen"
        );
    }

    #[test]
    fn every_panel_has_its_own_corner() {
        let corners = [STATUS_CORNER, HELP_CORNER, PERF_CORNER];
        for (i, a) in corners.iter().enumerate() {
            for (j, b) in corners.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "panels {i} and {j} are pinned to the same corner");
            }
        }
    }

    #[test]
    fn no_two_panels_share_a_corner() {
        // The controls used to open on top of the status panel, and moving
        // them right would have put them under the performance readout.
        assert_ne!(
            HELP_CORNER, PERF_CORNER,
            "the controls and the performance readout would overlap"
        );
    }

    #[test]
    fn a_right_hand_panel_is_pinned_to_the_right_edge() {
        let node = pinned(Corner::TopRight);
        assert_eq!(node.right, Val::Px(PANEL_INSET));
        assert_eq!(node.left, Val::Auto, "pinning both edges would stretch it");
        assert_eq!(node.position_type, PositionType::Absolute);
    }

    #[test]
    fn the_status_panel_still_clears_the_evaluation_bar() {
        let node = pinned(Corner::TopLeft);
        assert_eq!(node.left, Val::Px(EVAL_BAR_WIDTH + 10.0));
    }

    #[test]
    fn the_help_panel_is_closed_to_begin_with() {
        assert!(
            !ShowHelp::default().0,
            "help should not cover the board on startup"
        );
    }

    #[test]
    fn the_promotion_picker_names_all_four_pieces() {
        let listed: Vec<&str> = PROMOTION_CHOICES.iter().map(|r| role_name(*r)).collect();
        assert_eq!(listed, vec!["Queen", "Rook", "Bishop", "Knight"]);
        for role in PROMOTION_CHOICES {
            // The letter is the key you press, so it must be unambiguous.
            assert!(piece_letter(role).is_ascii_uppercase());
        }
    }

    #[test]
    fn the_promotion_keys_do_not_repeat_each_other() {
        let mut letters: Vec<char> = PROMOTION_CHOICES.iter().map(|r| piece_letter(*r)).collect();
        letters.sort_unstable();
        let before = letters.len();
        letters.dedup();
        assert_eq!(letters.len(), before, "two pieces share a key");
    }
}
