//! The scene around the board: a ground plane, a sky, and the light.
//!
//! Deliberately bare. The board is the subject; anything else in shot competes
//! with it.

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::board::spawn_board;

/// A clear day: a bright sun, with the sky itself doing the filling in. Values
/// are set against Bevy's reference scale, where `OVERCAST_DAY` is 1 000 lux
/// and `FULL_DAYLIGHT` is 20 000.
/// Sky colours, horizon first.
const SKY_HORIZON: Vec3 = Vec3::new(0.66, 0.85, 1.00);
const SKY_LOW: Vec3 = Vec3::new(0.40, 0.72, 0.99);
const SKY_MID: Vec3 = Vec3::new(0.17, 0.53, 0.94);
const SKY_ZENITH: Vec3 = Vec3::new(0.05, 0.33, 0.85);
/// How far away the sky sits.
const SKY_RADIUS: f32 = 200.0;

/// Colour of the sky at a given height, 0 at the horizon and 1 overhead.
fn sky_gradient(t: f32) -> Vec3 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.12 {
        SKY_HORIZON.lerp(SKY_LOW, t / 0.12)
    } else if t < 0.35 {
        SKY_LOW.lerp(SKY_MID, (t - 0.12) / 0.23)
    } else {
        SKY_MID.lerp(SKY_ZENITH, (t - 0.35) / 0.65)
    }
}

/// A dome painted with the gradient above, drawn unlit from the inside. A flat
/// clear colour reads as a void rather than a sky.
fn sky_dome_mesh() -> Mesh {
    const RINGS: usize = 32;
    const SEGMENTS: usize = 48;
    // Start below the horizon so the dome meets the ground with no gap.
    const LAT_MIN: f32 = -0.22;
    const LAT_MAX: f32 = std::f32::consts::FRAC_PI_2;

    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();

    for r in 0..=RINGS {
        let lat = LAT_MIN + (LAT_MAX - LAT_MIN) * (r as f32 / RINGS as f32);
        let (sin_lat, cos_lat) = lat.sin_cos();
        let t = (sin_lat / LAT_MAX.sin()).max(0.0);
        let c = sky_gradient(t);
        for seg in 0..=SEGMENTS {
            let lon = seg as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let (sin_lon, cos_lon) = lon.sin_cos();
            let p = Vec3::new(cos_lat * cos_lon, sin_lat, cos_lat * sin_lon) * SKY_RADIUS;
            positions.push(p.to_array());
            normals.push((-p.normalize()).to_array());
            colors.push([c.x, c.y, c.z, 1.0]);
            uvs.push([seg as f32 / SEGMENTS as f32, t]);
        }
    }

    let mut indices = Vec::new();
    let stride = SEGMENTS + 1;
    for r in 0..RINGS {
        for seg in 0..SEGMENTS {
            let a = (r * stride + seg) as u32;
            let b = (r * stride + seg + 1) as u32;
            let c = ((r + 1) * stride + seg) as u32;
            let d = ((r + 1) * stride + seg + 1) as u32;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

pub fn setup_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // --- The board --------------------------------------------------------
    spawn_board(&mut commands, &mut meshes, &mut materials);

    // --- Sky --------------------------------------------------------------
    commands.spawn((
        Mesh3d(meshes.add(sky_dome_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            // Seen from the inside, so do not cull the faces away.
            cull_mode: None,
            // Unlit skips the lighting, not the fog. The dome sits further
            // away than any sensible fog distance, so leaving this on paints
            // the whole sky a flat fog colour and the gradient is lost.
            fog_enabled: false,
            ..default()
        })),
        Transform::default(),
        // A dome this size would wreck the shadow cascades.
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(ClearColor(Color::srgb(
        SKY_HORIZON.x,
        SKY_HORIZON.y,
        SKY_HORIZON.z,
    )));

    // --- Light ------------------------------------------------------------
    // On a clear day the sky is a large, soft, blue source in its own right.
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.52, 0.68, 0.96),
        brightness: 130.0,
        ..default()
    });

    // A high sun, so the board is lit evenly and the pieces cast short shadows
    // rather than long bars across the squares.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.96, 0.89),
            illuminance: 13_000.0,
            shadow_maps_enabled: true,
            shadow_depth_bias: 0.03,
            ..default()
        },
        Transform::from_xyz(-24.0, 42.0, 16.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
