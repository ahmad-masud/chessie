//! Loading the chess pieces from a glTF model.
//!
//! The set is Poly Haven's "Chess Set" by Riley Queen, CC0. Only the pieces
//! are used: the board here does real work (click targets, move highlights,
//! the border the captured pieces stand on), so it stays procedural.
//!
//! Loading is asynchronous, and the turned shapes in `pieces.rs` are drawn
//! until it finishes — so the game is playable immediately, and still works
//! if the files are missing.

use bevy::gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode};
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use shakmaty::{Color as ChessColor, Role};

use crate::board::{BoardAssets, TARGET_KING_HEIGHT};
use crate::chess::Game;

/// Where the set lives, relative to `assets/`.
const MODEL_PATH: &str = "models/chess_set/chess_set.gltf";

/// The node in the file to use for each piece. The model lays a full set out
/// in its starting position, so most pieces appear several times; one of each
/// is enough, and the black pieces are separate meshes already turned to face
/// up the board.
const PIECE_NODES: [(Role, ChessColor, &str); 12] = [
    (Role::Pawn, ChessColor::White, "piece_pawn_white_01"),
    (Role::Knight, ChessColor::White, "piece_knight_white_01"),
    (Role::Bishop, ChessColor::White, "piece_bishop_white_01"),
    (Role::Rook, ChessColor::White, "piece_rook_white_01"),
    (Role::Queen, ChessColor::White, "piece_queen_white"),
    (Role::King, ChessColor::White, "piece_king_white"),
    (Role::Pawn, ChessColor::Black, "piece_pawn_black_01"),
    (Role::Knight, ChessColor::Black, "piece_knight_black_01"),
    (Role::Bishop, ChessColor::Black, "piece_bishop_black_01"),
    (Role::Rook, ChessColor::Black, "piece_rook_black_01"),
    (Role::Queen, ChessColor::Black, "piece_queen_black"),
    (Role::King, ChessColor::Black, "piece_king_black"),
];

/// The node whose height sets the scale for the whole set.
const SCALE_REFERENCE: &str = "piece_king_white";

#[derive(Resource)]
pub struct PieceModels {
    handle: Handle<Gltf>,
    /// Set once the pieces are in, or once we have given up on them.
    pub settled: bool,
}

pub fn load_piece_models(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(PieceModels {
        handle: asset_server.load(MODEL_PATH),
        settled: false,
    });
}

/// Turns a glTF material into one the renderer can use. Only the channels
/// this set actually carries are copied across.
fn to_standard(source: &GltfMaterial) -> StandardMaterial {
    StandardMaterial {
        base_color: source.base_color,
        base_color_texture: source.base_color_texture.clone(),
        metallic_roughness_texture: source.metallic_roughness_texture.clone(),
        normal_map_texture: source.normal_map_texture.clone(),
        occlusion_texture: source.occlusion_texture.clone(),
        perceptual_roughness: source.perceptual_roughness,
        metallic: source.metallic,
        ..default()
    }
}

/// Stands a piece on the origin at the right size: its feet on y = 0, centred
/// on x and z, and scaled so the set matches the board.
fn seat(bounds: (Vec3, Vec3), scale: f32) -> Transform {
    let (min, max) = bounds;
    let centre = (min + max) / 2.0;
    Transform::from_scale(Vec3::splat(scale)).with_translation(Vec3::new(
        -centre.x * scale,
        -min.y * scale,
        -centre.z * scale,
    ))
}

/// Reads a mesh's extent straight from its vertices.
pub fn bounds_of(mesh: &Mesh) -> Option<(Vec3, Vec3)> {
    let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?
    else {
        return None;
    };
    if positions.is_empty() {
        return None;
    }
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for p in positions {
        let p = Vec3::from_array(*p);
        min = min.min(p);
        max = max.max(p);
    }
    Some((min, max))
}

/// Installs the modelled pieces once the file has finished loading.
#[allow(clippy::too_many_arguments)]
pub fn install_piece_models(
    mut models: ResMut<PieceModels>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    gltf_materials: Res<Assets<GltfMaterial>>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut board: ResMut<BoardAssets>,
    mut game: ResMut<Game>,
) {
    if models.settled {
        return;
    }
    let Some(gltf) = gltfs.get(&models.handle) else {
        return;
    };

    // Resolves a node name to its first primitive's mesh and material.
    let primitive_of = |name: &str| -> Option<(Handle<Mesh>, Option<Handle<GltfMaterial>>)> {
        let node = nodes.get(gltf.named_nodes.get(name)?)?;
        let mesh = gltf_meshes.get(node.mesh.as_ref()?)?;
        let primitive = mesh.primitives.first()?;
        Some((primitive.mesh.clone(), primitive.material.clone()))
    };

    // One scale for the whole set, taken from the king.
    let reference =
        primitive_of(SCALE_REFERENCE).and_then(|(mesh, _)| meshes.get(&mesh).and_then(bounds_of));
    let Some((king_min, king_max)) = reference else {
        warn!("chess set model has no usable '{SCALE_REFERENCE}'; keeping the turned pieces");
        models.settled = true;
        return;
    };
    let scale = TARGET_KING_HEIGHT / (king_max.y - king_min.y);

    let mut installed = Vec::new();
    for (role, color, name) in PIECE_NODES {
        let Some((mesh_handle, material_handle)) = primitive_of(name) else {
            warn!("chess set model is missing '{name}'; keeping the turned pieces");
            models.settled = true;
            return;
        };
        let Some(bounds) = meshes.get(&mesh_handle).and_then(bounds_of) else {
            // The mesh is referenced but not loaded yet; try again next frame.
            return;
        };
        let material = material_handle
            .and_then(|h| gltf_materials.get(&h).map(to_standard))
            .unwrap_or_default();
        installed.push((
            (role, color),
            vec![(mesh_handle, materials.add(material), seat(bounds, scale))],
        ));
    }

    board.modelled = installed;
    models.settled = true;
    // Redraw the board with the new pieces.
    game.set_changed();
    info!("chess set model installed, scaled {scale:.2}x");
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    /// A box spanning the given corners, to stand in for a piece.
    fn block(min: Vec3, max: Vec3) -> Mesh {
        let corners: Vec<[f32; 3]> = [
            [min.x, min.y, min.z],
            [max.x, min.y, min.z],
            [min.x, max.y, min.z],
            [max.x, max.y, max.z],
        ]
        .to_vec();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, corners)
    }

    #[test]
    fn every_piece_and_colour_is_named_exactly_once() {
        assert_eq!(PIECE_NODES.len(), 12);
        let mut seen: Vec<(Role, ChessColor)> = Vec::new();
        for (role, color, _) in PIECE_NODES {
            assert!(
                !seen.contains(&(role, color)),
                "{role:?} {color:?} listed twice"
            );
            seen.push((role, color));
        }
        for role in [
            Role::Pawn,
            Role::Knight,
            Role::Bishop,
            Role::Rook,
            Role::Queen,
            Role::King,
        ] {
            for color in [ChessColor::White, ChessColor::Black] {
                assert!(
                    seen.contains(&(role, color)),
                    "{role:?} {color:?} is missing"
                );
            }
        }
    }

    #[test]
    fn the_scale_reference_is_one_of_the_pieces_we_load() {
        assert!(
            PIECE_NODES
                .iter()
                .any(|(_, _, name)| *name == SCALE_REFERENCE),
            "the piece the scale is taken from is not itself loaded"
        );
    }

    #[test]
    fn bounds_come_from_the_vertices() {
        let mesh = block(Vec3::new(-1.0, 0.5, -2.0), Vec3::new(3.0, 4.0, 2.0));
        let (min, max) = bounds_of(&mesh).expect("a mesh with positions has bounds");
        assert_eq!(min, Vec3::new(-1.0, 0.5, -2.0));
        assert_eq!(max, Vec3::new(3.0, 4.0, 2.0));
    }

    #[test]
    fn a_mesh_without_positions_has_no_bounds() {
        let empty = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        assert!(bounds_of(&empty).is_none());
    }

    #[test]
    fn seating_stands_a_piece_on_the_board_not_through_it() {
        // The model's pieces sit wherever they were on the artist's board, at
        // a fraction of our scale. Seating recentres and resizes them.
        let bounds = (Vec3::new(0.18, 0.017, -0.21), Vec3::new(0.22, 0.112, -0.17));
        let height = bounds.1.y - bounds.0.y;
        let scale = TARGET_KING_HEIGHT / height;
        let tf = seat(bounds, scale);

        // Its lowest point must land exactly on y = 0.
        let foot = tf.transform_point(Vec3::new(0.0, bounds.0.y, 0.0));
        assert!(
            foot.y.abs() < 1e-5,
            "the piece sits at {}, not on the board",
            foot.y
        );

        // And it must be centred over its square.
        let centre = (bounds.0 + bounds.1) / 2.0;
        let middle = tf.transform_point(centre);
        assert!(
            middle.x.abs() < 1e-5 && middle.z.abs() < 1e-5,
            "off centre at {middle:?}"
        );
    }

    #[test]
    fn seating_scales_the_king_to_the_height_we_want() {
        let bounds = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.04, 0.095, 0.04));
        let scale = TARGET_KING_HEIGHT / (bounds.1.y - bounds.0.y);
        let tf = seat(bounds, scale);
        let top = tf.transform_point(Vec3::new(0.0, bounds.1.y, 0.0));
        assert!(
            (top.y - TARGET_KING_HEIGHT).abs() < 1e-4,
            "king stands {} tall, wanted {TARGET_KING_HEIGHT}",
            top.y
        );
    }

    #[test]
    fn the_whole_set_keeps_its_proportions() {
        // One scale for every piece, so a pawn stays shorter than a king.
        let scale = 10.0;
        let pawn = seat((Vec3::ZERO, Vec3::new(0.03, 0.054, 0.03)), scale);
        let king = seat((Vec3::ZERO, Vec3::new(0.04, 0.095, 0.04)), scale);
        assert_eq!(pawn.scale, king.scale);
        let pawn_top = pawn.transform_point(Vec3::new(0.0, 0.054, 0.0)).y;
        let king_top = king.transform_point(Vec3::new(0.0, 0.095, 0.0)).y;
        assert!(pawn_top < king_top);
    }
}
