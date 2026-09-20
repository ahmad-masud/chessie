//! Turning clicks into chess moves, and letting the engine take its turn.

use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use shakmaty::{Move, Position, Role, Square};

use crate::board::{
    display_destination, square_to_world, world_to_square, BoardAssets, BoardPiece, BoardSquare,
    DroppedFrom, HighlightTile, PieceOf, BOARD_SURFACE_Y,
};
use crate::chess::{Game, HumanSide, Review};
use crate::engine::{Engine, Reply};
use crate::strength::{apply_weakening, Rating, Rng};

/// Which square the player has picked up, and where it may go.
#[derive(Resource, Default)]
pub struct Selection {
    pub square: Option<Square>,
    pub moves: Vec<Move>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.square = None;
        self.moves.clear();
    }
}

/// Set when a move is played so the HUD can narrate it.
#[derive(Resource, Default)]
pub struct LastMoveText(pub String);

/// What a click on `square` should do. Kept free of ECS types so the rules
/// that matter can be tested without a running app.
#[derive(Debug, PartialEq)]
pub enum ClickOutcome {
    /// Play this move.
    Play(Move),
    /// A pawn reached the far rank: the player has to say what it becomes.
    Promote {
        to: Square,
        /// One move per piece the pawn could turn into.
        options: Vec<Move>,
    },
    /// Pick up the piece on this square.
    Select { square: Square, moves: Vec<Move> },
    /// Put down whatever was held.
    Clear,
}

pub fn resolve_click(
    position: &shakmaty::Chess,
    selection: &Selection,
    human: shakmaty::Color,
    square: Square,
) -> ClickOutcome {
    // Holding a piece, and this square is somewhere it can go.
    if selection.square.is_some() {
        let candidates: Vec<&Move> = selection
            .moves
            .iter()
            // Castling is offered on the king's two-square destination, which
            // is what players aim for. The rook's own square still works, for
            // anyone used to entering it that way.
            .filter(|m| display_destination(m, human) == square || m.to() == square)
            .collect();

        // A promotion offers the same destination four times over, once per
        // piece. Ask rather than assuming a queen: underpromotion to a knight
        // is the whole point of some tactics.
        let promotions: Vec<Move> = candidates
            .iter()
            .filter(|m| m.is_promotion())
            .map(|m| (*m).clone())
            .collect();
        if promotions.len() > 1 {
            return ClickOutcome::Promote {
                to: square,
                options: promotions,
            };
        }
        if let Some(mv) = candidates.first() {
            return ClickOutcome::Play((*mv).clone());
        }
    }

    // Otherwise select, if it is one of our own pieces with somewhere to go.
    let is_own = position
        .board()
        .piece_at(square)
        .map(|p| p.color == human)
        .unwrap_or(false);

    if is_own {
        let moves = Game::legal_moves_from_position(position, square);
        if !moves.is_empty() {
            return ClickOutcome::Select { square, moves };
        }
    }
    ClickOutcome::Clear
}

/// Global click observer. Because pointer events propagate from a piece's
/// child mesh up to its parent, we only handle the leaf components
/// (`PieceOf` and `BoardSquare`) so each click resolves exactly once.
pub fn on_click(
    click: On<Pointer<Click>>,
    mut dragging: ResMut<Dragging>,
    human: Res<HumanSide>,
    mut review: ResMut<Review>,
    mut game: ResMut<Game>,
    mut selection: ResMut<Selection>,
    mut last_move: ResMut<LastMoveText>,
    mut promotion: ResMut<PendingPromotion>,
    engine: Res<Engine>,
    pieces: Query<&BoardPiece>,
    piece_of: Query<&PieceOf>,
    squares: Query<&BoardSquare>,
) {
    if click.event.button != PointerButton::Primary {
        return;
    }
    // This release already committed a drag.
    if dragging.just_dropped {
        dragging.just_dropped = false;
        return;
    }
    if dragging.active() {
        return;
    }
    // A promotion is waiting to be answered; nothing else may be played.
    if promotion.is_open() {
        return;
    }
    // Everything is judged against the position on screen, which may be an
    // earlier one: playing from there starts a new line.
    let shown_ply = review.shown_ply(&game);
    let position = game.position_at(shown_ply);
    if position.is_checkmate() || position.is_stalemate() {
        return;
    }
    if engine.thinking || position.turn() != human.0 {
        return;
    }
    // You are looking at an earlier position; the board on screen is not live.
    if review.is_reviewing() {
        return;
    }

    let entity = click.entity;

    // Resolve the click to a board square.
    let square = if let Ok(PieceOf(parent)) = piece_of.get(entity) {
        match pieces.get(*parent) {
            Ok(p) => p.square,
            Err(_) => return,
        }
    } else if let Ok(BoardSquare(sq)) = squares.get(entity) {
        *sq
    } else {
        return;
    };

    match resolve_click(&position, &selection, human.0, square) {
        ClickOutcome::Play(mv) => {
            // Moving from an earlier position abandons what came after it.
            game.branch_at(shown_ply);
            review.go_live();
            last_move.0 = format!("You played {}", game.move_to_uci(&mv));
            game.play(&mv);
            selection.clear();
        }
        ClickOutcome::Promote { options, .. } => {
            // Hold the move until the player says what the pawn becomes.
            promotion.options = options;
        }
        ClickOutcome::Select { square, moves } => {
            selection.square = Some(square);
            selection.moves = moves;
        }
        ClickOutcome::Clear => selection.clear(),
    }
}

/// Redraws the translucent tiles whenever the selection or position changes.
pub fn update_highlights(
    mut commands: Commands,
    selection: Res<Selection>,
    game: Res<Game>,
    review: Res<Review>,
    assets: Res<BoardAssets>,
    existing: Query<Entity, With<HighlightTile>>,
) {
    if !selection.is_changed() && !game.is_changed() && !review.is_changed() {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }

    let lift = Vec3::Y * 0.05;

    // Flag the king when it is in check, in the position being shown.
    let shown = game.position_at(review.shown_ply(&game));
    if shown.is_check() {
        let king = shown.board().king_of(shown.turn());
        if let Some(sq) = king {
            commands.spawn((
                Mesh3d(assets.tile_mesh.clone()),
                MeshMaterial3d(assets.check_tile.clone()),
                Transform::from_translation(square_to_world(sq) + lift),
                HighlightTile,
                // Highlights sit above the destination square; without this they
                // intercept the ray and the move never registers.
                Pickable::IGNORE,
            ));
        }
    }

    let Some(from) = selection.square else {
        return;
    };

    commands.spawn((
        Mesh3d(assets.tile_mesh.clone()),
        MeshMaterial3d(assets.select_tile.clone()),
        Transform::from_translation(square_to_world(from) + lift),
        HighlightTile,
        Pickable::IGNORE,
    ));

    for mv in &selection.moves {
        let is_capture = mv.is_capture();
        commands.spawn((
            Mesh3d(assets.tile_mesh.clone()),
            MeshMaterial3d(if is_capture {
                assets.capture_tile.clone()
            } else {
                assets.move_tile.clone()
            }),
            // Castling is marked where the king lands, not on the rook.
            Transform::from_translation(
                square_to_world(display_destination(mv, game.turn())) + lift,
            ),
            HighlightTile,
            // Highlights sit above the destination square; without this they
            // intercept the ray and the move never registers.
            Pickable::IGNORE,
        ));
    }
}

/// Asks the engine for a move when it is its turn.
pub fn engine_turn(
    mut engine: ResMut<Engine>,
    game: Res<Game>,
    review: Res<Review>,
    human: Res<HumanSide>,
    rating: Res<Rating>,
) {
    if game.ended.is_some() || engine.thinking || !engine.available {
        return;
    }
    // Do not let the game move on underneath someone reading it back.
    if review.is_reviewing() {
        return;
    }
    if game.turn() == human.0 {
        return;
    }
    let fen = game.fen();
    engine.search(fen, rating.movetime_ms());
}

/// Applies whatever the engine came back with.
pub fn engine_reply(
    mut engine: ResMut<Engine>,
    mut game: ResMut<Game>,
    mut rng: ResMut<Rng>,
    mut last_move: ResMut<LastMoveText>,
    rating: Res<Rating>,
) {
    let Some(reply) = engine.poll() else {
        return;
    };
    match reply {
        Reply::BestMove { uci, .. } => {
            // The answer is for whatever position we asked about. If the
            // player has since taken a move back and played something else,
            // that position is gone and the move must not be applied.
            let asked = engine.searched.take();
            if asked.as_deref() != Some(game.fen().as_str()) {
                return;
            }
            let Some(mv) = game.parse_uci(&uci) else {
                warn!("engine returned a move we could not parse: {uci}");
                return;
            };
            let played = apply_weakening(&game.position.clone(), mv, *rating, &mut rng);
            last_move.0 = format!("Opponent played {}", game.move_to_uci(&played));
            game.play(&played);
        }
        Reply::Failed(err) => {
            error!("engine failure: {err}");
            engine.available = false;
            engine.last_error = Some(err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Color;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    fn select(game: &Game, from: &str) -> Selection {
        Selection {
            square: Some(sq(from)),
            moves: Game::legal_moves_from_position(&game.position, sq(from)),
        }
    }

    #[test]
    fn clicking_an_own_piece_selects_it() {
        let game = Game::default();
        let outcome = resolve_click(
            &game.position,
            &Selection::default(),
            Color::White,
            sq("e2"),
        );
        match outcome {
            ClickOutcome::Select { square, moves } => {
                assert_eq!(square, sq("e2"));
                assert_eq!(moves.len(), 2);
            }
            other => panic!("expected a selection, got {other:?}"),
        }
    }

    #[test]
    fn clicking_an_opponent_piece_clears_instead_of_selecting() {
        let game = Game::default();
        let outcome = resolve_click(
            &game.position,
            &Selection::default(),
            Color::White,
            sq("e7"),
        );
        assert_eq!(outcome, ClickOutcome::Clear);
    }

    #[test]
    fn clicking_an_empty_square_with_nothing_held_clears() {
        let game = Game::default();
        let outcome = resolve_click(
            &game.position,
            &Selection::default(),
            Color::White,
            sq("e4"),
        );
        assert_eq!(outcome, ClickOutcome::Clear);
    }

    #[test]
    fn a_blocked_piece_cannot_be_selected() {
        // The a1 rook has no legal move from the starting position.
        let game = Game::default();
        let outcome = resolve_click(
            &game.position,
            &Selection::default(),
            Color::White,
            sq("a1"),
        );
        assert_eq!(outcome, ClickOutcome::Clear);
    }

    #[test]
    fn clicking_a_legal_destination_plays_the_move() {
        let game = Game::default();
        let selection = select(&game, "e2");
        match resolve_click(&game.position, &selection, Color::White, sq("e4")) {
            ClickOutcome::Play(mv) => assert_eq!(game.move_to_uci(&mv), "e2e4"),
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn clicking_an_illegal_destination_does_not_play() {
        let game = Game::default();
        let selection = select(&game, "e2");
        // e5 is two ranks too far for a pawn on its opening move... plus one.
        let outcome = resolve_click(&game.position, &selection, Color::White, sq("e5"));
        assert_eq!(outcome, ClickOutcome::Clear);
    }

    #[test]
    fn clicking_another_own_piece_reselects_rather_than_clearing() {
        let game = Game::default();
        let selection = select(&game, "e2");
        match resolve_click(&game.position, &selection, Color::White, sq("d2")) {
            ClickOutcome::Select { square, .. } => assert_eq!(square, sq("d2")),
            other => panic!("expected reselection, got {other:?}"),
        }
    }

    #[test]
    fn reaching_the_last_rank_asks_what_the_pawn_becomes() {
        // It used to queen automatically, which quietly rules out the
        // underpromotions that some tactics depend on.
        let mut game = Game::default();
        game.position = "8/1P6/8/8/8/8/8/K6k w - - 0 1"
            .parse::<shakmaty::fen::Fen>()
            .unwrap()
            .into_position(shakmaty::CastlingMode::Standard)
            .unwrap();

        let selection = select(&game, "b7");
        match resolve_click(&game.position, &selection, Color::White, sq("b8")) {
            ClickOutcome::Promote { to, options } => {
                assert_eq!(to, sq("b8"));
                assert_eq!(options.len(), 4, "all four pieces should be offered");
                let roles: Vec<_> = options.iter().filter_map(|m| m.promotion()).collect();
                for role in PROMOTION_CHOICES {
                    assert!(roles.contains(&role), "{role:?} was not offered");
                }
            }
            other => panic!("expected to be asked, got {other:?}"),
        }
    }

    #[test]
    fn the_chosen_piece_is_the_one_played() {
        let mut game = Game::default();
        game.position = "8/1P6/8/8/8/8/8/K6k w - - 0 1"
            .parse::<shakmaty::fen::Fen>()
            .unwrap()
            .into_position(shakmaty::CastlingMode::Standard)
            .unwrap();
        let selection = select(&game, "b7");
        let ClickOutcome::Promote { options, .. } =
            resolve_click(&game.position, &selection, Color::White, sq("b8"))
        else {
            panic!("expected a promotion");
        };
        let pending = PendingPromotion { options };

        for (role, uci) in [
            (Role::Queen, "b7b8q"),
            (Role::Rook, "b7b8r"),
            (Role::Bishop, "b7b8b"),
            (Role::Knight, "b7b8n"),
        ] {
            let mv = pending
                .choose(role)
                .unwrap_or_else(|| panic!("{role:?} missing"));
            assert_eq!(game.move_to_uci(&mv), uci);
        }
    }

    #[test]
    fn an_open_picker_blocks_other_play_until_answered() {
        let pending = PendingPromotion { options: vec![] };
        assert!(!pending.is_open(), "no options means nothing to answer");

        let mut game = Game::default();
        game.position = "8/1P6/8/8/8/8/8/K6k w - - 0 1"
            .parse::<shakmaty::fen::Fen>()
            .unwrap()
            .into_position(shakmaty::CastlingMode::Standard)
            .unwrap();
        let selection = select(&game, "b7");
        let ClickOutcome::Promote { options, .. } =
            resolve_click(&game.position, &selection, Color::White, sq("b8"))
        else {
            panic!("expected a promotion");
        };
        let mut pending = PendingPromotion { options };
        assert!(pending.is_open());
        pending.clear();
        assert!(!pending.is_open(), "abandoning must close the picker");
    }

    #[test]
    fn a_capture_onto_the_last_rank_also_asks() {
        let mut game = Game::default();
        game.position = "r6k/1P6/8/8/8/8/8/K7 w - - 0 1"
            .parse::<shakmaty::fen::Fen>()
            .unwrap()
            .into_position(shakmaty::CastlingMode::Standard)
            .unwrap();
        let selection = select(&game, "b7");
        match resolve_click(&game.position, &selection, Color::White, sq("a8")) {
            ClickOutcome::Promote { options, .. } => {
                assert_eq!(options.len(), 4);
                assert!(options.iter().all(|m| m.is_capture()));
            }
            other => panic!("expected a capturing promotion, got {other:?}"),
        }
    }

    #[test]
    fn an_ordinary_move_is_played_without_asking() {
        let game = Game::default();
        let selection = select(&game, "e2");
        match resolve_click(&game.position, &selection, Color::White, sq("e4")) {
            ClickOutcome::Play(mv) => assert_eq!(game.move_to_uci(&mv), "e2e4"),
            other => panic!("expected a plain move, got {other:?}"),
        }
    }

    #[test]
    fn a_capture_is_offered_as_a_destination() {
        let mut game = Game::default();
        for uci in ["e2e4", "d7d5"] {
            let mv = game.parse_uci(uci).unwrap();
            game.play(&mv);
        }
        let selection = select(&game, "e4");
        match resolve_click(&game.position, &selection, Color::White, sq("d5")) {
            ClickOutcome::Play(mv) => {
                assert!(mv.is_capture());
                assert_eq!(game.move_to_uci(&mv), "e4d5");
            }
            other => panic!("expected a capture, got {other:?}"),
        }
    }
}

/// A pawn waiting to be told what it becomes.
#[derive(Resource, Default)]
pub struct PendingPromotion {
    /// One move per choice, in the order they are offered.
    pub options: Vec<Move>,
}

impl PendingPromotion {
    pub fn is_open(&self) -> bool {
        !self.options.is_empty()
    }

    /// The move that turns the pawn into `role`, if that is on offer.
    pub fn choose(&self, role: Role) -> Option<Move> {
        self.options
            .iter()
            .find(|m| m.promotion() == Some(role))
            .cloned()
    }

    pub fn clear(&mut self) {
        self.options.clear();
    }
}

/// The key for each piece a pawn may become, in the order the picker shows
/// them. Chess notation, so the letter is the piece.
pub const PROMOTION_KEYS: [(KeyCode, Role); 4] = [
    (KeyCode::KeyQ, Role::Queen),
    (KeyCode::KeyR, Role::Rook),
    (KeyCode::KeyB, Role::Bishop),
    (KeyCode::KeyN, Role::Knight),
];

/// The pieces a pawn may become, in the order the picker shows them.
pub const PROMOTION_CHOICES: [Role; 4] = [Role::Queen, Role::Rook, Role::Bishop, Role::Knight];

/// A piece currently held under the pointer.
#[derive(Resource, Default)]
pub struct Dragging {
    pub piece: Option<Entity>,
    pub from: Option<Square>,
    /// Last board position the pointer was over, used to resolve the drop.
    pub over: Option<Square>,
    /// Set when a drag just finished, so the click that follows the same
    /// release is not treated as a second, separate move.
    pub just_dropped: bool,
    /// Where the held piece actually is, so the move can carry on from there
    /// instead of snapping back to the square it came from.
    pub held_at: Option<Vec3>,
    /// Gap between where the pointer struck the board and where the piece
    /// stands. Without it the piece jumps to the pointer the moment it is
    /// grabbed, because a ray through a tall piece meets the board well
    /// beyond that piece's base.
    pub grab_offset: Option<Vec2>,
}

impl Dragging {
    pub fn clear(&mut self) {
        self.piece = None;
        self.from = None;
        self.over = None;
        self.grab_offset = None;
        self.held_at = None;
    }

    pub fn active(&self) -> bool {
        self.piece.is_some()
    }
}

/// How high a held piece floats above the board. Kept small: the higher it
/// rises, the further it slides across the screen away from the pointer.
const DRAG_LIFT: f32 = 0.22;

/// The offset to preserve so a piece does not jump when first grabbed.
pub fn grab_offset(piece: Vec3, pointer_on_board: Vec3) -> Vec2 {
    Vec2::new(piece.x - pointer_on_board.x, piece.z - pointer_on_board.z)
}

/// Where a held piece should stand for a given pointer position.
pub fn dragged_position(pointer_on_board: Vec3, offset: Vec2, lift: f32) -> Vec3 {
    Vec3::new(
        pointer_on_board.x + offset.x,
        BOARD_SURFACE_Y + lift,
        pointer_on_board.z + offset.y,
    )
}

/// Where a screen position lands on the board's surface plane.
fn pointer_to_board(
    camera: &Camera,
    camera_tf: &GlobalTransform,
    cursor: Vec2,
    plane_y: f32,
) -> Option<Vec3> {
    let ray = camera.viewport_to_world(camera_tf, cursor).ok()?;
    let dir = ray.direction.as_vec3();
    // Parallel to the board, or pointing away from it.
    if dir.y.abs() < 1e-5 {
        return None;
    }
    let t = (plane_y - ray.origin.y) / dir.y;
    if t < 0.0 {
        return None;
    }
    Some(ray.origin + dir * t)
}

/// Picking up a piece.
pub fn on_drag_start(
    drag: On<Pointer<DragStart>>,
    human: Res<HumanSide>,
    review: Res<Review>,
    engine: Res<Engine>,
    promotion: Res<PendingPromotion>,
    game: Res<Game>,
    mut dragging: ResMut<Dragging>,
    mut selection: ResMut<Selection>,
    pieces: Query<&BoardPiece>,
    piece_of: Query<&PieceOf>,
) {
    if drag.event.button != PointerButton::Primary {
        return;
    }
    let shown_ply = review.shown_ply(&game);
    let position = game.position_at(shown_ply);
    if position.is_checkmate()
        || position.is_stalemate()
        || engine.thinking
        || position.turn() != human.0
        || promotion.is_open()
    {
        return;
    }

    // Only the piece meshes start a drag; the squares beneath do not.
    let Ok(PieceOf(parent)) = piece_of.get(drag.entity) else {
        return;
    };
    let Ok(piece) = pieces.get(*parent) else {
        return;
    };

    let moves = Game::legal_moves_from_position(&position, piece.square);
    let is_own = position
        .board()
        .piece_at(piece.square)
        .map(|p| p.color == human.0)
        .unwrap_or(false);
    if !is_own || moves.is_empty() {
        return;
    }

    dragging.piece = Some(*parent);
    dragging.from = Some(piece.square);
    dragging.over = Some(piece.square);
    // Reuse the click selection so the legal-move highlights appear.
    selection.square = Some(piece.square);
    selection.moves = moves;
}

/// Carrying it: the piece follows the pointer, floating above the board.
pub fn drag_piece(
    mut dragging: ResMut<Dragging>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(entity) = dragging.piece else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_tf)) = cameras.single() else {
        return;
    };
    let Some(point) = pointer_to_board(camera, camera_tf, cursor, BOARD_SURFACE_Y) else {
        return;
    };

    let Ok(mut tf) = transforms.get_mut(entity) else {
        // The board was rebuilt under us; stop dragging a dead entity.
        dragging.clear();
        return;
    };

    // Taken on the first frame of the drag, while the piece is still on its
    // square, so the grab keeps its position relative to the pointer.
    let offset = *dragging
        .grab_offset
        .get_or_insert_with(|| grab_offset(tf.translation, point));

    let target = dragged_position(point, offset, DRAG_LIFT);
    tf.translation = target;
    // The square under the piece decides the drop, not the raw pointer.
    dragging.over = world_to_square(target);
    dragging.held_at = Some(target);
}

/// Letting go. Dropping on a legal square plays the move; anywhere else the
/// piece goes back where it came from.
pub fn on_drag_end(
    _drag: On<Pointer<DragEnd>>,
    mut dragging: ResMut<Dragging>,
    mut game: ResMut<Game>,
    mut selection: ResMut<Selection>,
    mut last_move: ResMut<LastMoveText>,
    mut dropped: ResMut<DroppedFrom>,
    mut promotion: ResMut<PendingPromotion>,
    mut review: ResMut<Review>,
    human: Res<HumanSide>,
) {
    if !dragging.active() {
        return;
    }
    let target = dragging.over;
    let held_at = dragging.held_at;
    dragging.clear();
    dragging.just_dropped = true;

    let shown_ply = review.shown_ply(&game);
    let position = game.position_at(shown_ply);
    let outcome = match target {
        Some(square) => resolve_click(&position, &selection, human.0, square),
        None => ClickOutcome::Clear,
    };

    match outcome {
        ClickOutcome::Play(mv) => {
            // Let the board finish the move from the hand, not from the square
            // the piece started on.
            if let Some(from) = held_at {
                dropped.0 = Some((mv.to(), from));
            }
            // Moving from an earlier position abandons what came after it.
            game.branch_at(shown_ply);
            review.go_live();
            last_move.0 = format!("You played {}", game.move_to_uci(&mv));
            game.play(&mv);
            selection.clear();
        }
        // Dropped back on its own square. A press that wanders even a pixel
        // counts as a drag, so this is what an ordinary click looks like —
        // leave the piece selected rather than deselecting it.
        ClickOutcome::Promote { options, .. } => {
            // Put the pawn back on its square while the picker is open.
            promotion.options = options;
            game.set_changed();
        }
        ClickOutcome::Select { square, moves } => {
            selection.square = Some(square);
            selection.moves = moves;
            // Force a redraw so the held piece settles onto its square.
            game.set_changed();
        }
        // Dropped somewhere illegal: put it down again, unchanged.
        ClickOutcome::Clear => {
            selection.clear();
            game.set_changed();
        }
    }
}

/// Clears the drag flag if no click followed the release.
pub fn clear_drag_flag(mut dragging: ResMut<Dragging>) {
    if dragging.just_dropped && dragging.piece.is_none() {
        dragging.just_dropped = false;
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;
    use crate::board::square_to_world;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    #[test]
    fn grabbing_a_piece_does_not_move_it() {
        // This was the bug: a ray through a tall piece meets the board beyond
        // its base, so placing the piece at that point made it jump.
        let piece = square_to_world(sq("e2"));
        // Pointer struck the board well past the piece, as it does when you
        // grab a king by the head.
        let pointer = piece + Vec3::new(0.35, 0.0, 0.9);

        let offset = grab_offset(piece, pointer);
        let held = dragged_position(pointer, offset, 0.0);

        assert!(
            (held.x - piece.x).abs() < 1e-5 && (held.z - piece.z).abs() < 1e-5,
            "piece jumped from {piece:?} to {held:?}"
        );
    }

    #[test]
    fn the_piece_follows_the_pointer_one_for_one() {
        let piece = square_to_world(sq("d4"));
        let pointer = piece + Vec3::new(0.2, 0.0, 0.6);
        let offset = grab_offset(piece, pointer);

        let moved_pointer = pointer + Vec3::new(2.4, 0.0, -1.2);
        let held = dragged_position(moved_pointer, offset, 0.0);

        assert!((held.x - (piece.x + 2.4)).abs() < 1e-5);
        assert!((held.z - (piece.z - 1.2)).abs() < 1e-5);
    }

    #[test]
    fn a_grabbed_piece_stays_over_its_own_square() {
        // Grab anywhere on the piece, do not move: the drop square must still
        // be the square it started on.
        for name in ["a1", "e2", "h8", "d5"] {
            let piece = square_to_world(sq(name));
            let pointer = piece + Vec3::new(0.3, 0.0, 0.8);
            let offset = grab_offset(piece, pointer);
            let held = dragged_position(pointer, offset, DRAG_LIFT);
            assert_eq!(world_to_square(held), Some(sq(name)), "{name}");
        }
    }

    #[test]
    fn lifting_changes_only_the_height() {
        let piece = square_to_world(sq("c3"));
        let pointer = piece + Vec3::new(0.1, 0.0, 0.4);
        let offset = grab_offset(piece, pointer);

        let flat = dragged_position(pointer, offset, 0.0);
        let lifted = dragged_position(pointer, offset, DRAG_LIFT);

        assert!((flat.x - lifted.x).abs() < 1e-6);
        assert!((flat.z - lifted.z).abs() < 1e-6);
        assert!(lifted.y > flat.y);
        assert!((lifted.y - (BOARD_SURFACE_Y + DRAG_LIFT)).abs() < 1e-6);
    }

    #[test]
    fn dragging_a_full_square_lands_on_the_next_square() {
        let piece = square_to_world(sq("e2"));
        let pointer = piece + Vec3::new(0.0, 0.0, 0.7);
        let offset = grab_offset(piece, pointer);

        // Two squares up the board is e4.
        let moved = pointer + Vec3::new(0.0, 0.0, crate::board::SQUARE_SIZE * 2.0);
        let held = dragged_position(moved, offset, DRAG_LIFT);
        assert_eq!(world_to_square(held), Some(sq("e4")));
    }
}

#[cfg(test)]
mod drag_release_tests {
    use super::*;
    use crate::chess::Game;
    use shakmaty::Color;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    #[test]
    fn releasing_on_the_starting_square_keeps_the_piece_selected() {
        // A press that wanders a pixel is reported as a drag, so this path is
        // what an ordinary click on a piece goes through.
        let game = Game::default();
        let selection = Selection {
            square: Some(sq("e2")),
            moves: Game::legal_moves_from_position(&game.position, sq("e2")),
        };
        match resolve_click(&game.position, &selection, Color::White, sq("e2")) {
            ClickOutcome::Select { square, .. } => assert_eq!(square, sq("e2")),
            other => panic!("expected the piece to stay selected, got {other:?}"),
        }
    }

    #[test]
    fn releasing_on_a_legal_square_plays_the_move() {
        let game = Game::default();
        let selection = Selection {
            square: Some(sq("e2")),
            moves: Game::legal_moves_from_position(&game.position, sq("e2")),
        };
        match resolve_click(&game.position, &selection, Color::White, sq("e4")) {
            ClickOutcome::Play(mv) => assert_eq!(game.move_to_uci(&mv), "e2e4"),
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn releasing_on_an_illegal_square_plays_nothing() {
        let game = Game::default();
        let selection = Selection {
            square: Some(sq("e2")),
            moves: Game::legal_moves_from_position(&game.position, sq("e2")),
        };
        assert_eq!(
            resolve_click(&game.position, &selection, Color::White, sq("e5")),
            ClickOutcome::Clear
        );
    }
}

#[cfg(test)]
mod castling_tests {
    use super::*;
    use crate::chess::Game;
    use shakmaty::Color;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    fn ready_to_castle() -> Game {
        let mut game = Game::default();
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6"] {
            let mv = game.parse_uci(uci).unwrap();
            game.play(&mv);
        }
        game
    }

    fn king_selected(game: &Game) -> Selection {
        Selection {
            square: Some(sq("e1")),
            moves: Game::legal_moves_from_position(&game.position, sq("e1")),
        }
    }

    #[test]
    fn castling_is_played_by_moving_the_king_two_squares() {
        // What every chess interface does, and what UCI's king-takes-rook
        // notation obscures.
        let game = ready_to_castle();
        let selection = king_selected(&game);
        match resolve_click(&game.position, &selection, Color::White, sq("g1")) {
            ClickOutcome::Play(mv) => {
                assert!(mv.is_castle(), "g1 should castle, got {mv:?}");
                assert_eq!(
                    game.move_to_uci(&mv),
                    "e1g1",
                    "the engine is sent standard UCI, which names the king's square"
                );
            }
            other => panic!("expected a castle, got {other:?}"),
        }
    }

    #[test]
    fn clicking_the_rook_still_castles() {
        let game = ready_to_castle();
        let selection = king_selected(&game);
        match resolve_click(&game.position, &selection, Color::White, sq("h1")) {
            ClickOutcome::Play(mv) => assert!(mv.is_castle()),
            other => panic!("expected a castle, got {other:?}"),
        }
    }

    #[test]
    fn the_highlight_sits_on_the_kings_square_not_the_rooks() {
        let game = ready_to_castle();
        let castle = Game::legal_moves_from_position(&game.position, sq("e1"))
            .into_iter()
            .find(|m| m.is_castle())
            .expect("short castle should be legal");

        assert_eq!(display_destination(&castle, Color::White), sq("g1"));
        assert_eq!(
            castle.to(),
            sq("h1"),
            "shakmaty stores the rook square internally"
        );
    }

    #[test]
    fn queenside_castling_lands_the_king_on_c1() {
        let mut game = Game::default();
        for uci in [
            "d2d4", "d7d5", "b1c3", "b8c6", "c1f4", "c8f5", "d1d2", "d8d7",
        ] {
            let mv = game.parse_uci(uci).unwrap();
            game.play(&mv);
        }
        let selection = king_selected(&game);
        match resolve_click(&game.position, &selection, Color::White, sq("c1")) {
            ClickOutcome::Play(mv) => {
                assert!(mv.is_castle());
                assert_eq!(game.move_to_uci(&mv), "e1c1");
            }
            other => panic!("expected a long castle, got {other:?}"),
        }
    }

    #[test]
    fn black_castles_onto_its_own_back_rank() {
        let mut game = Game::default();
        for uci in ["e2e4", "e7e5", "g1f3", "g8f6", "f1c4", "f8c5", "b1c3"] {
            let mv = game.parse_uci(uci).unwrap();
            game.play(&mv);
        }
        let selection = Selection {
            square: Some(sq("e8")),
            moves: Game::legal_moves_from_position(&game.position, sq("e8")),
        };
        match resolve_click(&game.position, &selection, Color::Black, sq("g8")) {
            ClickOutcome::Play(mv) => {
                assert!(mv.is_castle());
                assert_eq!(game.move_to_uci(&mv), "e8g8");
            }
            other => panic!("expected a castle, got {other:?}"),
        }
    }

    #[test]
    fn an_ordinary_king_move_is_unaffected() {
        let game = ready_to_castle();
        let selection = king_selected(&game);
        match resolve_click(&game.position, &selection, Color::White, sq("e2")) {
            ClickOutcome::Play(mv) => {
                assert!(!mv.is_castle());
                assert_eq!(game.move_to_uci(&mv), "e1e2");
            }
            other => panic!("expected a plain king move, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod playing_black_tests {
    use super::*;
    use crate::chess::Game;
    use shakmaty::Color;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    /// Mirrors the condition `engine_turn` uses.
    fn engine_to_move(game: &Game, human: Color) -> bool {
        game.turn() != human
    }

    #[test]
    fn as_black_the_engine_opens_the_game() {
        let game = Game::default();
        assert!(
            engine_to_move(&game, Color::Black),
            "White opens, so the engine should"
        );
        assert!(!engine_to_move(&game, Color::White));
    }

    #[test]
    fn as_black_it_becomes_your_move_after_white_replies() {
        let mut game = Game::default();
        let e4 = game.parse_uci("e2e4").unwrap();
        game.play(&e4);
        assert!(
            !engine_to_move(&game, Color::Black),
            "should be Black's move now"
        );
    }

    #[test]
    fn as_black_you_cannot_pick_up_whites_pieces() {
        let game = Game::default();
        // e2 is a White pawn; playing Black it must not be selectable.
        assert_eq!(
            resolve_click(
                &game.position,
                &Selection::default(),
                Color::Black,
                sq("e2")
            ),
            ClickOutcome::Clear
        );
    }

    #[test]
    fn as_black_your_own_pieces_are_selectable_on_your_turn() {
        let mut game = Game::default();
        let e4 = game.parse_uci("e2e4").unwrap();
        game.play(&e4);
        match resolve_click(
            &game.position,
            &Selection::default(),
            Color::Black,
            sq("e7"),
        ) {
            ClickOutcome::Select { square, moves } => {
                assert_eq!(square, sq("e7"));
                assert_eq!(moves.len(), 2);
            }
            other => panic!("expected to pick up the pawn, got {other:?}"),
        }
    }

    #[test]
    fn as_black_a_move_plays_normally() {
        let mut game = Game::default();
        game.play(&game.parse_uci("e2e4").unwrap().clone());
        let selection = Selection {
            square: Some(sq("e7")),
            moves: Game::legal_moves_from_position(&game.position, sq("e7")),
        };
        match resolve_click(&game.position, &selection, Color::Black, sq("e5")) {
            ClickOutcome::Play(mv) => assert_eq!(game.move_to_uci(&mv), "e7e5"),
            other => panic!("expected a move, got {other:?}"),
        }
    }
}

/// Q, R, B or N settles a promotion. Escape abandons the move.
pub fn promotion_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut promotion: ResMut<PendingPromotion>,
    mut game: ResMut<Game>,
    mut selection: ResMut<Selection>,
    mut last_move: ResMut<LastMoveText>,
) {
    if !promotion.is_open() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        promotion.clear();
        selection.clear();
        return;
    }
    let picked = PROMOTION_KEYS
        .into_iter()
        .find(|(key, _)| keys.just_pressed(*key));

    let Some((_, role)) = picked else {
        return;
    };
    let Some(mv) = promotion.choose(role) else {
        return;
    };
    last_move.0 = format!("You played {}", game.move_to_uci(&mv));
    game.play(&mv);
    promotion.clear();
    selection.clear();
}
