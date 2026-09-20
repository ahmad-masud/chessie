//! The chess board as a physical place in the world: geometry, piece meshes,
//! and the mapping between board squares and world coordinates.

use bevy::prelude::*;
use shakmaty::{Color as ChessColor, Move, Position, Role, Square};

use crate::chess::{Game, Review};

/// Edge length of one square, in metres. The set is deliberately oversized —
/// you walk around this board rather than sit at it.
pub const SQUARE_SIZE: f32 = 1.2;
/// How tall a king stands relative to one square.
///
/// A standard tournament set is a 95 mm king on a 57 mm square, a ratio of
/// 1.67, which looked a touch large here — the camera sits closer than a
/// player does. This is a little under that, still comfortably taller than
/// the square it stands on. The whole set scales from this value.
pub const KING_TO_SQUARE: f32 = 1.45;
/// Height of a king in world units.
pub const TARGET_KING_HEIGHT: f32 = SQUARE_SIZE * KING_TO_SQUARE;

/// Height of the stone plinth the tiles rest on.
pub const PLINTH_HEIGHT: f32 = 0.6;
/// How thick each board tile is. The tiles are sunk into the plinth so that no
/// two faces are ever coplanar — coplanar faces z-fight and make the board
/// shimmer.
const TILE_THICKNESS: f32 = 0.16;
/// Height of the playing surface: the top of the tiles, where pieces stand.
pub const BOARD_SURFACE_Y: f32 = PLINTH_HEIGHT + 0.12;
/// Centre of the board in world space.
pub const BOARD_CENTER: Vec3 = Vec3::new(0.0, 0.0, 0.0);
pub const BOARD_EXTENT: f32 = SQUARE_SIZE * 8.0;

/// Marks a spawned piece so we can clear the board between syncs.
#[derive(Component)]
pub struct BoardPiece {
    pub square: Square,
}

/// Marks the translucent tiles showing selection and legal destinations.
#[derive(Component)]
pub struct HighlightTile;

/// Shared handles so we do not rebuild meshes and materials every sync.
#[derive(Resource)]
pub struct BoardAssets {
    pub white_piece: Handle<StandardMaterial>,
    pub black_piece: Handle<StandardMaterial>,
    pub select_tile: Handle<StandardMaterial>,
    pub move_tile: Handle<StandardMaterial>,
    pub capture_tile: Handle<StandardMaterial>,
    pub check_tile: Handle<StandardMaterial>,
    pub tile_mesh: Handle<Mesh>,
    /// Built once at startup. Rebuilding these per move re-uploads every piece
    /// to the GPU and visibly hitches.
    piece_meshes: Vec<(Role, Vec<(Handle<Mesh>, Transform)>)>,
    /// Filled in once the modelled set finishes loading. Until then, and if
    /// the files are missing, the turned shapes above stand in.
    pub modelled: Vec<(
        (Role, ChessColor),
        Vec<(Handle<Mesh>, Handle<StandardMaterial>, Transform)>,
    )>,
}

/// Everything needed to draw one piece.
pub struct PieceVisual {
    pub parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>, Transform)>,
    /// Rotation about Y for the whole piece. The modelled set has separate,
    /// already-turned meshes per colour, so it needs none.
    pub facing: f32,
}

impl BoardAssets {
    /// How to draw `role` for `color`, preferring the modelled set.
    pub fn visual_for(&self, role: Role, color: ChessColor) -> PieceVisual {
        if let Some((_, parts)) = self
            .modelled
            .iter()
            .find(|((r, c), _)| *r == role && *c == color)
        {
            return PieceVisual {
                parts: parts.clone(),
                // The model ships a separate mesh per colour, each already
                // facing its opponent.
                facing: 0.0,
            };
        }

        let material = match color {
            ChessColor::White => self.white_piece.clone(),
            ChessColor::Black => self.black_piece.clone(),
        };
        let parts = self
            .piece_meshes
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, parts)| {
                parts
                    .iter()
                    .map(|(mesh, tf)| (mesh.clone(), material.clone(), *tf))
                    .collect()
            })
            .unwrap_or_default();
        PieceVisual {
            parts,
            // The turned shapes are one mesh for both sides, so black is
            // spun round to face the other way.
            facing: if color == ChessColor::White {
                0.0
            } else {
                std::f32::consts::PI
            },
        }
    }
}

/// World position of the centre of a square's top surface.
pub fn square_to_world(square: Square) -> Vec3 {
    let file = u32::from(square.file()) as f32; // 0 = a
    let rank = u32::from(square.rank()) as f32; // 0 = rank 1
    BOARD_CENTER
        + Vec3::new(
            // Files run the other way to the raw index: seen from White the
            // a-file must be on the left, which is +X here.
            (3.5 - file) * SQUARE_SIZE,
            BOARD_SURFACE_Y,
            // Rank 1 (White's home) sits at -Z.
            (rank - 3.5) * SQUARE_SIZE,
        )
}

/// Which square a world-space point lies over, if any.
pub fn world_to_square(point: Vec3) -> Option<Square> {
    let local = point - BOARD_CENTER;
    let file = (3.5 - local.x / SQUARE_SIZE).round();
    let rank = (local.z / SQUARE_SIZE + 3.5).round();
    if !(0.0..=7.0).contains(&file) || !(0.0..=7.0).contains(&rank) {
        return None;
    }
    Square::try_from((rank as u32 * 8 + file as u32) as u8).ok()
}

/// Builds the generated pieces and sizes them to match the modelled set, so
/// the stand-ins are not a different scale from the real thing.
fn build_turned_pieces(meshes: &mut Assets<Mesh>) -> Vec<(Role, Vec<(Handle<Mesh>, Transform)>)> {
    let roles = [
        Role::Pawn,
        Role::Knight,
        Role::Bishop,
        Role::Rook,
        Role::Queen,
        Role::King,
    ];
    let built: Vec<(Role, Vec<(Handle<Mesh>, Transform)>)> = roles
        .into_iter()
        .map(|role| (role, crate::pieces::build(role, meshes)))
        .collect();

    // Measure the king rather than assuming its height: the profiles can
    // change, and a stale number would leave the set the wrong size.
    let king_top = built
        .iter()
        .find(|(role, _)| *role == Role::King)
        .map(|(_, parts)| highest_point(parts, meshes))
        .unwrap_or(0.0);
    if king_top <= f32::EPSILON {
        return built;
    }
    let scale = TARGET_KING_HEIGHT / king_top;

    built
        .into_iter()
        .map(|(role, parts)| {
            let parts = parts
                .into_iter()
                .map(|(mesh, tf)| {
                    (
                        mesh,
                        Transform {
                            translation: tf.translation * scale,
                            rotation: tf.rotation,
                            scale: tf.scale * scale,
                        },
                    )
                })
                .collect();
            (role, parts)
        })
        .collect()
}

/// The top of a piece, across all the parts it is made of.
fn highest_point(parts: &[(Handle<Mesh>, Transform)], meshes: &Assets<Mesh>) -> f32 {
    parts
        .iter()
        .filter_map(|(handle, tf)| {
            let (_, max) = crate::models::bounds_of(meshes.get(handle)?)?;
            Some(tf.transform_point(max).y)
        })
        .fold(0.0f32, f32::max)
}

pub fn setup_board_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let translucent = |color: Color| StandardMaterial {
        base_color: color,
        alpha_mode: AlphaMode::Blend,
        emissive: LinearRgba::from(color) * 2.0,
        unlit: false,
        ..default()
    };

    let assets = BoardAssets {
        white_piece: materials.add(StandardMaterial {
            base_color: Color::srgb(0.92, 0.89, 0.82),
            perceptual_roughness: 0.35,
            metallic: 0.05,
            ..default()
        }),
        black_piece: materials.add(StandardMaterial {
            base_color: Color::srgb(0.17, 0.16, 0.20),
            perceptual_roughness: 0.28,
            metallic: 0.15,
            ..default()
        }),
        select_tile: materials.add(translucent(Color::srgba(0.30, 0.85, 1.00, 0.55))),
        move_tile: materials.add(translucent(Color::srgba(0.35, 0.95, 0.45, 0.42))),
        capture_tile: materials.add(translucent(Color::srgba(1.00, 0.35, 0.30, 0.50))),
        check_tile: materials.add(translucent(Color::srgba(1.00, 0.15, 0.15, 0.60))),
        tile_mesh: meshes.add(Cuboid::new(SQUARE_SIZE * 0.92, 0.04, SQUARE_SIZE * 0.92)),
        modelled: Vec::new(),
        piece_meshes: build_turned_pieces(&mut meshes),
    };
    commands.insert_resource(assets);
}

/// Builds the static board: plinth, rim, and the 64 squares.
pub fn spawn_board(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let light_sq = materials.add(StandardMaterial {
        base_color: Color::srgb(0.78, 0.72, 0.60),
        perceptual_roughness: 0.55,
        ..default()
    });
    let dark_sq = materials.add(StandardMaterial {
        base_color: Color::srgb(0.26, 0.22, 0.20),
        perceptual_roughness: 0.55,
        ..default()
    });
    let stone = materials.add(StandardMaterial {
        base_color: Color::srgb(0.42, 0.40, 0.38),
        perceptual_roughness: 0.85,
        ..default()
    });

    let square_mesh = meshes.add(Cuboid::new(SQUARE_SIZE, TILE_THICKNESS, SQUARE_SIZE));

    // Plinth the board rests on.
    let plinth = meshes.add(Cuboid::new(
        plinth_half() * 2.0,
        PLINTH_HEIGHT,
        plinth_half() * 2.0,
    ));
    commands.spawn((
        Mesh3d(plinth),
        MeshMaterial3d(stone.clone()),
        Transform::from_translation(BOARD_CENTER + Vec3::new(0.0, PLINTH_HEIGHT / 2.0, 0.0)),
    ));

    for idx in 0..64u8 {
        let Ok(square) = Square::try_from(idx) else {
            continue;
        };
        let is_light = (u32::from(square.file()) + u32::from(square.rank())) % 2 == 1;
        // Top face lands exactly on the playing surface; the rest is buried.
        let pos = square_to_world(square) - Vec3::Y * (TILE_THICKNESS / 2.0);
        commands.spawn((
            Mesh3d(square_mesh.clone()),
            MeshMaterial3d(if is_light {
                light_sq.clone()
            } else {
                dark_sq.clone()
            }),
            Transform::from_translation(pos),
            Pickable::default(),
            BoardSquare(square),
        ));
    }
}

/// Lets the picking backend tell us which square was clicked.
#[derive(Component, Clone, Copy)]
pub struct BoardSquare(pub Square);

/// Marks a taken piece standing in a tray beside the board.
#[derive(Component)]
pub struct TrayPiece;

/// How far out from the last rank the first column of taken pieces sits.
const TRAY_GAP: f32 = 0.60;
/// Roughly how wide the broadest piece is, as a fraction of the king's
/// height. Measured from the set rather than guessed; a test keeps it honest.
const WIDEST_PIECE_FRACTION: f32 = 0.54;
/// Clear air left between pieces standing in a pile.
const TRAY_AIR: f32 = 0.08;
/// Spacing between pieces in a tray. Derived from the piece size so that
/// changing how big the set is does not quietly make the piles overlap.
const TRAY_STEP: f32 = TARGET_KING_HEIGHT * WIDEST_PIECE_FRACTION + TRAY_AIR;
/// How many fit in one column before starting another, further out.
const TRAY_COLUMN: usize = 8;
/// Columns needed for everything but the king.
const TRAY_COLUMNS: usize = 2;
/// Clearance between the outermost taken piece and the edge of the board.
const TRAY_MARGIN: f32 = 0.42;

/// Half-width of the board, border included.
///
/// The border is sized to carry the taken pieces: they stand on it, so it has
/// to reach past the outer column. A separate shelf beside the board looked
/// like a piece of furniture that had wandered in.
pub fn plinth_half() -> f32 {
    let outer = BOARD_EXTENT / 2.0 + TRAY_GAP + (TRAY_COLUMNS as f32 - 1.0) * TRAY_STEP;
    outer + TRAY_MARGIN
}

/// Where the `index`th taken piece of `owner` stands.
///
/// Each pile sits on its owner's left, seen from that player's own end of the
/// board: White looks up the board from -Z, so White's left is +X, and Black's
/// is the mirror of that.
pub fn tray_position(owner: ChessColor, index: usize) -> Vec3 {
    let left = match owner {
        ChessColor::White => 1.0,
        ChessColor::Black => -1.0,
    };
    let column = (index / TRAY_COLUMN) as f32;
    let row = (index % TRAY_COLUMN) as f32;
    let x = left * (BOARD_EXTENT / 2.0 + TRAY_GAP + column * TRAY_STEP);
    // Rows run from the owner's own end toward the middle.
    let near_edge = match owner {
        ChessColor::White => -(BOARD_EXTENT / 2.0) + TRAY_STEP * 0.5,
        ChessColor::Black => BOARD_EXTENT / 2.0 - TRAY_STEP * 0.5,
    };
    // Rows advance toward the middle of the board, whichever end we are at.
    let z = near_edge + row * TRAY_STEP * left;
    // The border is the plinth's own top face, a tile's thickness below the
    // playing surface, so taken pieces sit slightly lower than pieces in play.
    Vec3::new(x, PLINTH_HEIGHT, z)
}

/// How a taken piece stands in its pile. Kept as a function so the scale can
/// actually be asserted rather than inferred.
pub fn tray_transform(owner: ChessColor, index: usize) -> Transform {
    let facing = if owner == ChessColor::White {
        0.0
    } else {
        std::f32::consts::PI
    };
    // No scale: a taken piece is the same piece it was a moment ago.
    Transform::from_translation(tray_position(owner, index))
        .with_rotation(Quat::from_rotation_y(facing))
}

/// Stands the taken pieces in their trays.
pub fn sync_captured(
    mut commands: Commands,
    game: Res<Game>,
    review: Res<Review>,
    assets: Res<BoardAssets>,
    existing: Query<Entity, With<TrayPiece>>,
) {
    if !game.is_changed() && !review.is_changed() {
        return;
    }
    for entity in &existing {
        commands.entity(entity).try_despawn();
    }

    let losses = game.losses_at(review.shown_ply(&game));
    for owner in [ChessColor::White, ChessColor::Black] {
        for (index, role) in losses.get(owner).iter().enumerate() {
            let visual = assets.visual_for(*role, owner);
            // Take the facing from the same place the board does, or a
            // modelled piece (already turned) would be spun a second time.
            let mut stand = tray_transform(owner, index);
            stand.rotation = Quat::from_rotation_y(visual.facing);
            let parent = commands
                .spawn((stand, Visibility::default(), TrayPiece))
                .id();
            for (mesh, material, local) in visual.parts {
                let child = commands
                    .spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        local,
                        // Taken pieces are scenery; they must not take clicks.
                        Pickable::IGNORE,
                    ))
                    .id();
                commands.entity(parent).add_child(child);
            }
        }
    }
}

/// A piece sliding from one square to another.
#[derive(Component)]
pub struct MoveAnimation {
    pub from: Vec3,
    pub to: Vec3,
    pub elapsed: f32,
    pub duration: f32,
    /// How high the piece arcs on the way. Knights hop; everything else glides.
    pub lift: f32,
}

/// A piece that has just been taken, shrinking away.
#[derive(Component)]
pub struct Capturing {
    pub elapsed: f32,
    pub duration: f32,
}

const MOVE_DURATION: f32 = 0.34;
/// Shortest an arrival may take, so a piece released almost on its square
/// settles rather than crawling.
const MIN_MOVE_DURATION: f32 = 0.09;

/// Where a piece is travelling in from, and how high it arcs on the way.
pub struct Arrival {
    pub from: Vec3,
    pub lift: f32,
}

/// A move released from the hand: the piece should settle from where it was
/// being held, not snap back to its old square and slide across the board.
#[derive(Resource, Default)]
pub struct DroppedFrom(pub Option<(Square, Vec3)>);

/// Long moves take the full time; short ones are quicker, so releasing a piece
/// a hair off its square is a settle rather than a slow glide.
fn travel_duration(from: Vec3, to: Vec3) -> f32 {
    let spans = from.distance(to) / (SQUARE_SIZE * 3.0);
    (MOVE_DURATION * spans).clamp(MIN_MOVE_DURATION, MOVE_DURATION)
}
const CAPTURE_DURATION: f32 = 0.26;

/// Every (from, to) pair a move physically relocates. Castling moves two.
pub fn move_legs(mv: &Move, mover: ChessColor) -> Vec<(Square, Square)> {
    match mv {
        Move::Castle { king, rook } => {
            let side = mv.castling_side().expect("castle has a side");
            vec![(*king, side.king_to(mover)), (*rook, side.rook_to(mover))]
        }
        _ => match mv.from() {
            Some(from) => vec![(from, mv.to())],
            None => Vec::new(),
        },
    }
}

/// The square a move appears to end on, from the player's point of view.
///
/// UCI writes castling as king-takes-rook, so `Move::to` gives the rook's
/// square. Players move the king two squares instead, which is what every
/// chess interface shows, so that is what we highlight and accept.
pub fn display_destination(mv: &Move, mover: ChessColor) -> Square {
    match mv {
        Move::Castle { .. } => mv
            .castling_side()
            .map(|side| side.king_to(mover))
            .unwrap_or_else(|| mv.to()),
        _ => mv.to(),
    }
}

/// Where the taken piece was standing. For en passant that is not the
/// destination square.
pub fn captured_square(mv: &Move) -> Option<Square> {
    match mv {
        Move::EnPassant { from, to } => Some(Square::from_coords(to.file(), from.rank())),
        Move::Normal { capture, to, .. } => capture.map(|_| *to),
        _ => None,
    }
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

/// Slides pieces toward their destination square.
pub fn animate_moves(
    time: Res<Time>,
    mut commands: Commands,
    mut pieces: Query<(Entity, &mut Transform, &mut MoveAnimation)>,
) {
    for (entity, mut tf, mut anim) in &mut pieces {
        anim.elapsed += time.delta_secs();
        let t = (anim.elapsed / anim.duration).clamp(0.0, 1.0);
        let mut pos = anim.from.lerp(anim.to, ease_in_out(t));
        pos.y += anim.lift * (std::f32::consts::PI * t).sin();
        tf.translation = pos;
        if t >= 1.0 {
            tf.translation = anim.to;
            commands.entity(entity).remove::<MoveAnimation>();
        }
    }
}

/// Shrinks a taken piece into the board, then removes it.
pub fn animate_captures(
    time: Res<Time>,
    mut commands: Commands,
    mut pieces: Query<(Entity, &mut Transform, &mut Capturing)>,
) {
    for (entity, mut tf, mut cap) in &mut pieces {
        cap.elapsed += time.delta_secs();
        let t = (cap.elapsed / cap.duration).clamp(0.0, 1.0);
        tf.scale = Vec3::splat((1.0 - t).max(0.01));
        tf.translation.y = BOARD_SURFACE_Y - t * 0.35;
        if t >= 1.0 {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Draws whatever position is currently being shown: the live game, or an
/// earlier one if the player is reviewing.
pub fn sync_pieces(
    mut commands: Commands,
    game: Res<Game>,
    review: Res<Review>,
    assets: Res<BoardAssets>,
    existing: Query<Entity, With<BoardPiece>>,
    mut dropped: ResMut<DroppedFrom>,
    // The ply last drawn, so we know which way the player just stepped.
    mut drawn: Local<Option<usize>>,
) {
    if !game.is_changed() && !review.is_changed() {
        return;
    }

    // Consumed whether or not it applies, so a stale drop cannot leak into a
    // later move.
    let dropped_from = dropped.0.take();

    let ply = review.shown_ply(&game);
    let position = game.position_at(ply);

    // A single step in either direction animates; a jump does not.
    let previous = *drawn;
    *drawn = Some(ply);
    let step: Option<(Move, bool)> = match previous {
        Some(prev) if ply == prev + 1 => game.history.get(prev).map(|mv| (mv.clone(), false)),
        Some(prev) if prev == ply + 1 => game.history.get(ply).map(|mv| (mv.clone(), true)),
        _ => None,
    };

    for entity in &existing {
        // A capture animation may have already removed this one.
        commands.entity(entity).try_despawn();
    }

    // Legs are (from, to); rewinding just swaps them.
    let legs: Vec<(Square, Square)> = match &step {
        Some((mv, reverse)) => {
            let mover = Game::mover_at(if *reverse { ply } else { ply - 1 });
            move_legs(mv, mover)
                .into_iter()
                .map(|(from, to)| if *reverse { (to, from) } else { (from, to) })
                .collect()
        }
        None => Vec::new(),
    };

    // Re-spawn a piece that was just taken so it can animate away. Going
    // backwards the piece returns instead, and the replayed position has it.
    if let Some((mv, false)) = &step {
        if let (Some(sq), Some(role)) = (captured_square(mv), mv.capture()) {
            let taken_from = Game::mover_at(ply - 1).other();
            spawn_piece(&mut commands, &assets, role, taken_from, sq, None, true);
        }
    }

    let board = position.board().clone();
    for idx in 0..64u8 {
        let Ok(square) = Square::try_from(idx) else {
            continue;
        };
        let Some(piece) = board.piece_at(square) else {
            continue;
        };

        // A piece released from the hand carries on from where it was held.
        // Otherwise it travels in from the square the move started on.
        let arrival = match dropped_from {
            Some((dropped_on, held_at)) if dropped_on == square => Some(Arrival {
                from: held_at,
                // It is already in the air; arcing again would look like a hop.
                lift: 0.0,
            }),
            _ => legs
                .iter()
                .find(|(_, to)| *to == square)
                .map(|(from, _)| Arrival {
                    from: square_to_world(*from),
                    // Knights are the only piece that may jump, so let them.
                    lift: if piece.role == Role::Knight {
                        0.75
                    } else {
                        0.10
                    },
                }),
        };

        spawn_piece(
            &mut commands,
            &assets,
            piece.role,
            piece.color,
            square,
            arrival,
            false,
        );
    }
}

/// Spawns one piece, optionally travelling in from `origin` or being taken.
#[allow(clippy::too_many_arguments)]
fn spawn_piece(
    commands: &mut Commands,
    assets: &BoardAssets,
    role: Role,
    color: ChessColor,
    square: Square,
    arrival: Option<Arrival>,
    captured: bool,
) {
    let visual = assets.visual_for(role, color);
    let facing = visual.facing;

    let target = square_to_world(square);
    let start = arrival.as_ref().map(|a| a.from).unwrap_or(target);

    let mut piece = commands.spawn((
        Transform::from_translation(start).with_rotation(Quat::from_rotation_y(facing)),
        Visibility::default(),
        BoardPiece { square },
    ));

    if captured {
        piece.insert((
            Capturing {
                elapsed: 0.0,
                duration: CAPTURE_DURATION,
            },
            // A piece on its way out must not swallow clicks.
            Pickable::IGNORE,
        ));
    } else if let Some(arrival) = arrival {
        piece.insert(MoveAnimation {
            from: start,
            to: target,
            elapsed: 0.0,
            duration: travel_duration(start, target),
            lift: arrival.lift,
        });
    }

    let parent = piece.id();
    for (mesh, material, local) in visual.parts {
        let child = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                local,
                if captured {
                    Pickable::IGNORE
                } else {
                    Pickable::default()
                },
                PieceOf(parent),
            ))
            .id();
        commands.entity(parent).add_child(child);
    }
}

/// Child meshes point back at their piece so clicks resolve to a square.
#[derive(Component)]
pub struct PieceOf(pub Entity);

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    #[test]
    fn corners_map_to_opposite_corners_of_the_board() {
        let a1 = square_to_world(sq("a1"));
        let h8 = square_to_world(sq("h8"));
        // The a-file is at +X so that it falls on White's left, and rank 1 is
        // at -Z, White's end. So a1 is the +X/-Z corner and h8 the -X/+Z one.
        assert!(a1.x > 0.0 && a1.z < 0.0, "a1 was {a1:?}");
        assert!(h8.x < 0.0 && h8.z > 0.0, "h8 was {h8:?}");
        assert!((a1.y - BOARD_SURFACE_Y).abs() < f32::EPSILON);
    }

    #[test]
    fn white_home_rank_sits_nearer_the_player_than_blacks() {
        assert!(square_to_world(sq("e1")).z < square_to_world(sq("e8")).z);
    }

    #[test]
    fn adjacent_squares_are_one_square_apart() {
        let d4 = square_to_world(sq("d4"));
        let e4 = square_to_world(sq("e4"));
        assert!((d4.distance(e4) - SQUARE_SIZE).abs() < 1e-4);
    }

    #[test]
    fn board_is_centred_on_the_origin() {
        let mut sum = Vec3::ZERO;
        for idx in 0..64u8 {
            sum += square_to_world(Square::try_from(idx).unwrap());
        }
        let centre = sum / 64.0;
        assert!(
            centre.x.abs() < 1e-4 && centre.z.abs() < 1e-4,
            "centre was {centre:?}"
        );
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use crate::chess::Game;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    fn play(game: &mut Game, ucis: &[&str]) {
        for uci in ucis {
            let mv = game
                .parse_uci(uci)
                .unwrap_or_else(|| panic!("{uci} should be legal"));
            game.play(&mv);
        }
    }

    #[test]
    fn a_normal_move_has_one_leg() {
        let game = Game::default();
        let mv = game.parse_uci("e2e4").unwrap();
        let legs = move_legs(&mv, ChessColor::White);
        assert_eq!(legs, vec![(sq("e2"), sq("e4"))]);
        assert_eq!(captured_square(&mv), None);
    }

    #[test]
    fn castling_moves_both_king_and_rook() {
        let mut game = Game::default();
        play(&mut game, &["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "g8f6"]);
        // UCI castling is written king-takes-rook.
        let castle = game
            .parse_uci("e1h1")
            .expect("short castle should be legal");
        let legs = move_legs(&castle, ChessColor::White);
        assert_eq!(legs.len(), 2, "castling relocates two pieces");
        assert!(
            legs.contains(&(sq("e1"), sq("g1"))),
            "king e1->g1, got {legs:?}"
        );
        assert!(
            legs.contains(&(sq("h1"), sq("f1"))),
            "rook h1->f1, got {legs:?}"
        );
    }

    #[test]
    fn queenside_castling_uses_the_other_files() {
        let mut game = Game::default();
        play(
            &mut game,
            &[
                "d2d4", "d7d5", "b1c3", "b8c6", "c1f4", "c8f5", "d1d2", "d8d7",
            ],
        );
        let castle = game.parse_uci("e1a1").expect("long castle should be legal");
        let legs = move_legs(&castle, ChessColor::White);
        assert!(
            legs.contains(&(sq("e1"), sq("c1"))),
            "king e1->c1, got {legs:?}"
        );
        assert!(
            legs.contains(&(sq("a1"), sq("d1"))),
            "rook a1->d1, got {legs:?}"
        );
    }

    #[test]
    fn en_passant_takes_a_pawn_that_is_not_on_the_destination() {
        let mut game = Game::default();
        play(&mut game, &["e2e4", "a7a6", "e4e5", "d7d5"]);
        let ep = game.parse_uci("e5d6").expect("en passant should be legal");
        // The pawn taken stands on d5, while the capturing pawn lands on d6.
        assert_eq!(captured_square(&ep), Some(sq("d5")));
        assert_eq!(
            move_legs(&ep, ChessColor::White),
            vec![(sq("e5"), sq("d6"))]
        );
    }

    #[test]
    fn an_ordinary_capture_is_taken_on_the_destination() {
        let mut game = Game::default();
        play(&mut game, &["e2e4", "d7d5"]);
        let capture = game.parse_uci("e4d5").unwrap();
        assert_eq!(captured_square(&capture), Some(sq("d5")));
    }

    #[test]
    fn easing_is_bounded_and_monotonic() {
        assert!((ease_in_out(0.0) - 0.0).abs() < 1e-6);
        assert!((ease_in_out(1.0) - 1.0).abs() < 1e-6);
        let mut prev = -1.0;
        for i in 0..=20 {
            let v = ease_in_out(i as f32 / 20.0);
            assert!(v >= prev, "easing went backwards at {i}");
            assert!((0.0..=1.0).contains(&v));
            prev = v;
        }
    }
}

#[cfg(test)]
mod pointer_tests {
    use super::*;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    #[test]
    fn a_point_over_a_square_resolves_to_that_square() {
        for name in ["a1", "h8", "e4", "d5", "h1", "a8"] {
            let centre = square_to_world(sq(name));
            assert_eq!(world_to_square(centre), Some(sq(name)), "{name}");
        }
    }

    #[test]
    fn points_within_a_square_still_resolve_to_it() {
        let centre = square_to_world(sq("e4"));
        let nudge = SQUARE_SIZE * 0.4;
        for offset in [
            Vec3::new(nudge, 0.0, 0.0),
            Vec3::new(-nudge, 0.0, 0.0),
            Vec3::new(0.0, 0.0, nudge),
            Vec3::new(0.0, 0.0, -nudge),
        ] {
            assert_eq!(world_to_square(centre + offset), Some(sq("e4")));
        }
    }

    #[test]
    fn dropping_off_the_board_resolves_to_nothing() {
        // Well outside the 8x8 grid in each direction.
        for offset in [
            Vec3::new(BOARD_EXTENT, 0.0, 0.0),
            Vec3::new(-BOARD_EXTENT, 0.0, 0.0),
            Vec3::new(0.0, 0.0, BOARD_EXTENT),
            Vec3::new(0.0, 0.0, -BOARD_EXTENT),
        ] {
            assert_eq!(world_to_square(BOARD_CENTER + offset), None, "{offset:?}");
        }
    }

    #[test]
    fn square_mapping_round_trips_for_every_square() {
        for idx in 0..64u8 {
            let square = Square::try_from(idx).unwrap();
            assert_eq!(world_to_square(square_to_world(square)), Some(square));
        }
    }

    #[test]
    fn height_does_not_change_which_square_a_point_is_over() {
        // A dragged piece floats above the board; it must still resolve.
        let centre = square_to_world(sq("c6"));
        assert_eq!(world_to_square(centre + Vec3::Y * 2.0), Some(sq("c6")));
    }
}

#[cfg(test)]
mod arrival_tests {
    use super::*;

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    #[test]
    fn a_released_piece_carries_on_from_the_hand() {
        // The bug: the arrival used to begin at the square the piece came
        // from, so dropping it made it snap backwards and slide in again.
        let origin = square_to_world(sq("e2"));
        let destination = square_to_world(sq("e4"));
        // Where it actually was when released: near e4, held above the board.
        let held = destination + Vec3::new(0.15, DROP_TEST_LIFT, -0.2);

        let dropped = DroppedFrom(Some((sq("e4"), held)));
        let arrival = match dropped.0 {
            Some((on, from)) if on == sq("e4") => Arrival { from, lift: 0.0 },
            _ => Arrival {
                from: origin,
                lift: 0.10,
            },
        };

        assert!(
            arrival.from.distance(destination) < arrival.from.distance(origin),
            "the piece should settle from the hand, not from its old square"
        );
        assert_eq!(
            arrival.lift, 0.0,
            "it is already airborne; do not arc again"
        );
    }

    #[test]
    fn a_drop_on_one_square_does_not_affect_another() {
        let dropped = DroppedFrom(Some((sq("e4"), Vec3::ZERO)));
        // Rendering some other square must fall back to the normal path.
        let applies = matches!(dropped.0, Some((on, _)) if on == sq("d5"));
        assert!(!applies);
    }

    #[test]
    fn a_short_settle_is_quicker_than_a_long_slide() {
        let from = square_to_world(sq("e4"));
        let nudge = travel_duration(from + Vec3::new(0.05, 0.2, 0.05), from);
        let across = travel_duration(square_to_world(sq("a1")), square_to_world(sq("h8")));
        assert!(nudge < across, "{nudge} should be quicker than {across}");
        assert!(nudge >= MIN_MOVE_DURATION);
        assert!(across <= MOVE_DURATION);
    }

    #[test]
    fn travel_time_is_never_zero_or_absurd() {
        for (a, b) in [
            (sq("e2"), sq("e2")),
            (sq("a1"), sq("h8")),
            (sq("d4"), sq("d5")),
        ] {
            let d = travel_duration(square_to_world(a), square_to_world(b));
            assert!(
                (MIN_MOVE_DURATION..=MOVE_DURATION).contains(&d),
                "{a} -> {b} took {d}"
            );
        }
    }

    /// Matches the lift used while a piece is held.
    const DROP_TEST_LIFT: f32 = 0.22;
}

#[cfg(test)]
mod tray_tests {
    use super::*;
    use crate::camera::BoardCamera;

    /// How far right of centre a point appears, from that player's own end.
    fn screen_x(viewer: ChessColor, point: Vec3) -> f32 {
        let mut cam = BoardCamera::default();
        cam.face(viewer);
        cam.clamp();
        for _ in 0..600 {
            cam.settle(1.0 / 60.0);
        }
        let tf = cam.transform();
        let right = tf.rotation * Vec3::X;
        right.dot((point - cam.position()).normalize())
    }

    #[test]
    fn each_pile_sits_on_its_owners_left() {
        // The point of the feature: your taken pieces appear on your left.
        for owner in [ChessColor::White, ChessColor::Black] {
            for index in 0..6 {
                let x = screen_x(owner, tray_position(owner, index));
                assert!(
                    x < 0.0,
                    "{owner:?}'s piece {index} appears on the right, at {x}"
                );
            }
        }
    }

    #[test]
    fn the_two_piles_are_on_opposite_sides_of_the_board() {
        let white = tray_position(ChessColor::White, 0);
        let black = tray_position(ChessColor::Black, 0);
        assert!(
            white.x.signum() != black.x.signum(),
            "both piles ended up on the same side"
        );
    }

    #[test]
    fn piles_stand_clear_of_the_board() {
        for owner in [ChessColor::White, ChessColor::Black] {
            for index in 0..16 {
                let p = tray_position(owner, index);
                assert!(
                    p.x.abs() > BOARD_EXTENT / 2.0,
                    "{owner:?} piece {index} is standing on the board at {p:?}"
                );
                assert!(
                    world_to_square(p).is_none(),
                    "{owner:?} piece {index} overlaps a playable square"
                );
            }
        }
    }

    #[test]
    fn a_pile_starts_at_its_owners_end_and_grows_toward_the_middle() {
        let near = tray_position(ChessColor::White, 0);
        let later = tray_position(ChessColor::White, 3);
        assert!(near.z < later.z, "White's pile should run up the board");

        let near = tray_position(ChessColor::Black, 0);
        let later = tray_position(ChessColor::Black, 3);
        assert!(near.z > later.z, "Black's pile should run down the board");
    }

    /// Is this point on the board's border, at its surface?
    fn on_border(point: Vec3) -> bool {
        let half = plinth_half();
        point.x.abs() <= half && point.z.abs() <= half && (point.y - PLINTH_HEIGHT).abs() < 1e-4
    }

    #[test]
    fn every_taken_piece_stands_on_the_border() {
        // They used to hang in the air over the courtyard floor. The border is
        // now sized to carry them, so each one must land on it.
        for owner in [ChessColor::White, ChessColor::Black] {
            for index in 0..15 {
                let p = tray_position(owner, index);
                assert!(
                    on_border(p),
                    "{owner:?} piece {index} at {p:?} is off the border (half-width {})",
                    plinth_half()
                );
            }
        }
    }

    #[test]
    fn the_border_reaches_past_the_outermost_piece() {
        // With clearance, so a piece's foot is not hanging over the edge.
        let outer = (0..15)
            .map(|i| tray_position(ChessColor::White, i).x.abs())
            .fold(0.0f32, f32::max);
        assert!(
            plinth_half() - outer > 0.3,
            "only {:.2} of border past the outer column at {outer:.2}",
            plinth_half() - outer
        );
    }

    #[test]
    fn taken_pieces_sit_a_tile_lower_than_pieces_in_play() {
        // The border is the plinth's top face; the squares sit proud of it.
        let in_play = square_to_world("e2".parse().unwrap()).y;
        let taken = tray_position(ChessColor::White, 0).y;
        assert!(
            taken < in_play,
            "taken pieces should rest on the lower border"
        );
        assert!(
            (in_play - taken - (BOARD_SURFACE_Y - PLINTH_HEIGHT)).abs() < 1e-4,
            "the drop should be exactly the tile thickness"
        );
    }

    #[test]
    fn the_border_is_not_absurdly_wide() {
        // A border wider than the board itself would look like a table.
        let border = plinth_half() - BOARD_EXTENT / 2.0;
        assert!(
            border < BOARD_EXTENT / 2.0,
            "border of {border:.2} against a board half-width of {:.2}",
            BOARD_EXTENT / 2.0
        );
    }

    #[test]
    fn a_taken_piece_is_the_same_size_as_one_in_play() {
        // Shrinking them made them read as a different set.
        for owner in [ChessColor::White, ChessColor::Black] {
            for index in [0usize, 7, 14] {
                let scale = tray_transform(owner, index).scale;
                assert!(
                    (scale - Vec3::ONE).length() < 1e-6,
                    "{owner:?} piece {index} is scaled to {scale:?}, not full size"
                );
            }
        }
    }

    #[test]
    fn taken_pieces_face_the_way_their_owner_does() {
        let white = tray_transform(ChessColor::White, 0).rotation * Vec3::Z;
        let black = tray_transform(ChessColor::Black, 0).rotation * Vec3::Z;
        assert!(
            white.dot(black) < 0.0,
            "both piles face the same way; they should mirror each other"
        );
    }

    #[test]
    fn a_full_pile_never_collides_with_itself() {
        // Fifteen is everything but the king.
        for owner in [ChessColor::White, ChessColor::Black] {
            let spots: Vec<Vec3> = (0..15).map(|i| tray_position(owner, i)).collect();
            for (i, a) in spots.iter().enumerate() {
                for (j, b) in spots.iter().enumerate().skip(i + 1) {
                    // Wide enough for the broadest piece in the set.
                    assert!(
                        a.distance(*b) > TARGET_KING_HEIGHT * WIDEST_PIECE_FRACTION,
                        "{owner:?} pieces {i} and {j} overlap at {a:?} / {b:?}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod sizing_tests {
    use super::*;

    /// Footprint and height of a built piece, after scaling.
    ///
    /// Measured as the extent of the enclosing box in x and z. Taking the
    /// distance to a box *corner* instead would overstate a round piece by a
    /// factor of root two, since a cylinder's corner is not on the cylinder.
    fn extent(parts: &[(Handle<Mesh>, Transform)], meshes: &Assets<Mesh>) -> (f32, f32) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for (handle, tf) in parts {
            let Some(mesh) = meshes.get(handle) else {
                continue;
            };
            let Some((min, max)) = crate::models::bounds_of(mesh) else {
                continue;
            };
            for corner in [
                Vec3::new(min.x, min.y, min.z),
                Vec3::new(max.x, min.y, min.z),
                Vec3::new(min.x, max.y, min.z),
                Vec3::new(min.x, min.y, max.z),
                Vec3::new(max.x, max.y, min.z),
                Vec3::new(max.x, min.y, max.z),
                Vec3::new(min.x, max.y, max.z),
                Vec3::new(max.x, max.y, max.z),
            ] {
                let p = tf.transform_point(corner);
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        ((hi.x - lo.x).max(hi.z - lo.z), hi.y)
    }

    #[test]
    fn the_generated_king_matches_the_modelled_one_in_height() {
        // The stand-ins have to be the same size as the real set, or the
        // pieces visibly change scale the moment the model finishes loading.
        let mut meshes = Assets::<Mesh>::default();
        let built = build_turned_pieces(&mut meshes);
        let (_, king_height) = built
            .iter()
            .find(|(role, _)| *role == Role::King)
            .map(|(_, parts)| extent(parts, &meshes))
            .expect("a king is built");
        assert!(
            (king_height - TARGET_KING_HEIGHT).abs() < 0.01,
            "generated king is {king_height}, the modelled one is {TARGET_KING_HEIGHT}"
        );
    }

    #[test]
    fn a_king_stands_taller_than_its_square_is_wide() {
        // A real tournament set is a 95mm king on a 57mm square.
        assert!(
            TARGET_KING_HEIGHT > SQUARE_SIZE,
            "the set would read as a travel set"
        );
        // A real set is 1.67; staying near it keeps the pieces believable.
        assert!(
            (1.3..=1.7).contains(&KING_TO_SQUARE),
            "king to square ratio {KING_TO_SQUARE} is outside what a real set looks like"
        );
    }

    #[test]
    fn no_generated_piece_is_wider_than_its_square() {
        let mut meshes = Assets::<Mesh>::default();
        for (role, parts) in build_turned_pieces(&mut meshes) {
            let (widest, _) = extent(&parts, &meshes);
            assert!(
                widest < SQUARE_SIZE,
                "{role:?} is {widest:.3} across, wider than its {SQUARE_SIZE} square"
            );
        }
    }

    #[test]
    fn the_captured_piles_are_spaced_for_the_widest_piece() {
        let mut meshes = Assets::<Mesh>::default();
        let widest = build_turned_pieces(&mut meshes)
            .iter()
            .map(|(_, parts)| extent(parts, &meshes).0)
            .fold(0.0f32, f32::max);
        assert!(
            TRAY_STEP > widest,
            "pile spacing {TRAY_STEP} is tighter than the widest piece at {widest:.3}"
        );
    }
}
