//! Piece geometry.
//!
//! Real chess pieces are turned on a lathe, so the shapes here are built the
//! same way: a 2D profile revolved around the Y axis. That gives proper
//! curved silhouettes instead of a stack of cylinders. The knight is the one
//! piece that is not a surface of revolution, so it is extruded from a
//! side-on outline instead.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use shakmaty::Role;

/// How many segments go around a revolved piece.
const SEGMENTS: usize = 40;

/// Revolves `profile` (x = radius, y = height) around the Y axis.
///
/// A profile that starts and ends at radius 0 produces a closed solid, so no
/// separate end caps are needed.
pub fn lathe(profile: &[Vec2]) -> Mesh {
    let rings = profile.len();
    assert!(rings >= 2, "a profile needs at least two points");

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(rings * (SEGMENTS + 1));
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(rings * (SEGMENTS + 1));
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(rings * (SEGMENTS + 1));

    let y_min = profile.first().map(|p| p.y).unwrap_or(0.0);
    let y_max = profile.last().map(|p| p.y).unwrap_or(1.0);
    let y_span = (y_max - y_min).max(1e-5);

    for (i, point) in profile.iter().enumerate() {
        // Tangent along the profile, from neighbouring points, so shading is
        // smooth across the curve.
        let prev = profile[i.saturating_sub(1)];
        let next = profile[(i + 1).min(rings - 1)];
        let tangent = (next - prev).normalize_or(Vec2::Y);
        // Outward normal in the profile plane.
        let n2 = Vec2::new(tangent.y, -tangent.x);

        for s in 0..=SEGMENTS {
            let theta = s as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let (sin, cos) = theta.sin_cos();
            positions.push([point.x * cos, point.y, point.x * sin]);
            let normal = Vec3::new(n2.x * cos, n2.y, n2.x * sin).normalize_or(Vec3::Y);
            normals.push(normal.to_array());
            uvs.push([s as f32 / SEGMENTS as f32, (point.y - y_min) / y_span]);
        }
    }

    let mut indices: Vec<u32> = Vec::with_capacity(rings * SEGMENTS * 6);
    let stride = SEGMENTS + 1;
    for i in 0..rings - 1 {
        for s in 0..SEGMENTS {
            let a = (i * stride + s) as u32;
            let b = (i * stride + s + 1) as u32;
            let c = ((i + 1) * stride + s) as u32;
            let d = ((i + 1) * stride + s + 1) as u32;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Twice the signed area. Positive means the outline runs counter-clockwise.
fn signed_area_2(poly: &[Vec2]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            a.x * b.y - b.x * a.y
        })
        .sum()
}

/// Which side of the line `a`->`b` the point `c` lies on.
fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

/// Triangulates a simple polygon by ear clipping, assuming it runs
/// counter-clockwise.
///
/// A fan from the centroid is not good enough here: the knight's outline is
/// concave around the ears and the jaw, and a fan lays triangles across those
/// notches, outside the silhouette.
fn triangulate(poly: &[Vec2]) -> Vec<[usize; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let mut remaining: Vec<usize> = (0..n).collect();
    let mut tris = Vec::with_capacity(n.saturating_sub(2));

    // Each pass must remove one ear; the bound stops a malformed outline
    // spinning forever.
    let mut guard = n * n;
    while remaining.len() > 3 && guard > 0 {
        guard -= 1;
        let m = remaining.len();
        let mut clipped = None;
        for k in 0..m {
            let (i0, i1, i2) = (
                remaining[(k + m - 1) % m],
                remaining[k],
                remaining[(k + 1) % m],
            );
            let (a, b, c) = (poly[i0], poly[i1], poly[i2]);
            // Reflex corners are not ears.
            if cross(a, b, c) <= 0.0 {
                continue;
            }
            // Nor is a corner with another vertex inside it.
            if remaining
                .iter()
                .any(|&j| j != i0 && j != i1 && j != i2 && point_in_triangle(poly[j], a, b, c))
            {
                continue;
            }
            tris.push([i0, i1, i2]);
            clipped = Some(k);
            break;
        }
        match clipped {
            Some(k) => {
                remaining.remove(k);
            }
            None => break,
        }
    }
    if remaining.len() == 3 {
        tris.push([remaining[0], remaining[1], remaining[2]]);
    }
    tris
}

/// Extrudes a closed 2D outline along Z, giving it flat front and back faces.
/// Used for the knight, which has no axis of revolution.
///
/// The winding is normalised here so callers cannot get it wrong: an outline
/// given clockwise would otherwise build its faces inside-out, and backface
/// culling would show straight through into the piece.
pub fn extrude(outline: &[Vec2], thickness: f32) -> Mesh {
    assert!(outline.len() >= 3, "an outline needs at least three points");
    let mut poly = outline.to_vec();
    if signed_area_2(&poly) < 0.0 {
        poly.reverse();
    }
    let n = poly.len();
    let half = thickness / 2.0;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let tris = triangulate(&poly);

    // Front and back caps, each a copy of the triangulated outline.
    for (sign, normal) in [(1.0f32, [0.0, 0.0, 1.0f32]), (-1.0, [0.0, 0.0, -1.0])] {
        let base = positions.len() as u32;
        for p in &poly {
            positions.push([p.x, p.y, half * sign]);
            normals.push(normal);
            uvs.push([p.x, p.y]);
        }
        for t in &tris {
            let (a, b, c) = (t[0] as u32, t[1] as u32, t[2] as u32);
            // The back face is the same triangles wound the other way.
            if sign > 0.0 {
                indices.extend_from_slice(&[base + a, base + b, base + c]);
            } else {
                indices.extend_from_slice(&[base + a, base + c, base + b]);
            }
        }
    }

    // Side wall. With the outline counter-clockwise, (dy, -dx) points outward.
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let edge = (b - a).normalize_or(Vec2::X);
        let nrm = Vec3::new(edge.y, -edge.x, 0.0).normalize_or(Vec3::X);
        let base = positions.len() as u32;
        for (p, z) in [(a, half), (b, half), (a, -half), (b, -half)] {
            positions.push([p.x, p.y, z]);
            normals.push(nrm.to_array());
            uvs.push([0.0, 0.0]);
        }
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// The turned profile for each piece, in world units.
///
/// Every profile shares the same base flare and collar so the set looks like
/// one set, then diverges above the stem.
fn profile(role: Role) -> Vec<Vec2> {
    let v = |x: f32, y: f32| Vec2::new(x, y);
    // Shared foot: a flared base rolling into the stem.
    let mut p = vec![
        v(0.00, 0.000),
        v(0.300, 0.000),
        v(0.305, 0.022),
        v(0.292, 0.050),
        v(0.250, 0.072),
        v(0.196, 0.098),
        v(0.168, 0.130),
    ];
    match role {
        Role::Pawn => p.extend([
            v(0.140, 0.175),
            v(0.120, 0.250),
            v(0.112, 0.330),
            v(0.126, 0.380),
            v(0.165, 0.410),
            v(0.150, 0.436),
            v(0.120, 0.455),
            v(0.150, 0.495),
            v(0.178, 0.560),
            v(0.170, 0.625),
            v(0.130, 0.685),
            v(0.070, 0.725),
            v(0.00, 0.745),
        ]),
        Role::Rook => p.extend([
            v(0.170, 0.200),
            v(0.163, 0.330),
            v(0.168, 0.470),
            v(0.190, 0.545),
            v(0.245, 0.585),
            v(0.268, 0.615),
            v(0.272, 0.660),
            v(0.300, 0.672),
            v(0.302, 0.740),
            v(0.250, 0.752),
            v(0.245, 0.700),
            v(0.00, 0.700),
        ]),
        Role::Knight => p.extend([
            // Only the foot is turned; the head is extruded separately.
            v(0.165, 0.190),
            v(0.160, 0.260),
            v(0.170, 0.300),
            v(0.185, 0.330),
            v(0.00, 0.345),
        ]),
        Role::Bishop => p.extend([
            v(0.145, 0.190),
            v(0.126, 0.280),
            v(0.118, 0.370),
            v(0.140, 0.420),
            v(0.185, 0.450),
            v(0.168, 0.478),
            v(0.130, 0.500),
            v(0.150, 0.545),
            v(0.186, 0.620),
            v(0.182, 0.700),
            v(0.140, 0.775),
            v(0.086, 0.822),
            v(0.062, 0.856),
            v(0.078, 0.882),
            v(0.070, 0.910),
            v(0.036, 0.930),
            v(0.00, 0.940),
        ]),
        Role::Queen => p.extend([
            v(0.158, 0.200),
            v(0.134, 0.300),
            v(0.120, 0.420),
            v(0.128, 0.520),
            v(0.150, 0.585),
            v(0.196, 0.625),
            v(0.176, 0.655),
            v(0.140, 0.680),
            v(0.168, 0.720),
            v(0.216, 0.800),
            v(0.232, 0.870),
            v(0.196, 0.900),
            v(0.150, 0.912),
            v(0.00, 0.918),
        ]),
        Role::King => p.extend([
            v(0.162, 0.205),
            v(0.138, 0.310),
            v(0.124, 0.440),
            v(0.132, 0.550),
            v(0.156, 0.620),
            v(0.202, 0.662),
            v(0.182, 0.694),
            v(0.144, 0.720),
            v(0.172, 0.762),
            v(0.220, 0.845),
            v(0.236, 0.915),
            v(0.206, 0.948),
            v(0.168, 0.962),
            v(0.00, 0.968),
        ]),
    }
    p
}

/// Side-on outline of the knight's head, drawn in the XY plane with the nose
/// toward +X. Traced as a horse's profile: up the crest of the neck, over the
/// poll between the ears, down the face to the muzzle, then back under the jaw
/// and along the throat.
fn knight_outline() -> Vec<Vec2> {
    let v = |x: f32, y: f32| Vec2::new(x, y);
    vec![
        // Bottom of the neck, sunk into the turned foot below.
        v(-0.132, 0.280),
        // Crest of the neck, rising to the poll.
        v(-0.170, 0.352),
        v(-0.196, 0.432),
        v(-0.208, 0.512),
        v(-0.204, 0.586),
        v(-0.186, 0.652),
        v(-0.154, 0.706),
        v(-0.112, 0.748),
        // Ears: a spike, a notch, another spike.
        v(-0.094, 0.822),
        v(-0.058, 0.758),
        v(-0.022, 0.830),
        v(0.006, 0.756),
        // Brow and the long slope of the face.
        v(0.056, 0.732),
        v(0.106, 0.702),
        v(0.152, 0.664),
        v(0.192, 0.616),
        v(0.216, 0.568),
        // Muzzle and nose.
        v(0.226, 0.522),
        v(0.212, 0.490),
        v(0.180, 0.474),
        v(0.146, 0.484),
        // Under the jaw.
        v(0.116, 0.468),
        v(0.086, 0.436),
        v(0.062, 0.396),
        v(0.048, 0.356),
        // Throat down to the chest.
        v(0.060, 0.322),
        v(0.082, 0.292),
        v(0.010, 0.280),
    ]
}

/// The head is modelled facing +X, but it has to look up the board at the
/// opponent. Turning it the other way points White's knights back at White.
const KNIGHT_FACING: f32 = -std::f32::consts::FRAC_PI_2;
/// How thick the head is. Chunky enough not to read as a cut-out.
const KNIGHT_THICKNESS: f32 = 0.205;

/// Mesh parts for a piece, each with its local transform.
pub fn build(role: Role, meshes: &mut Assets<Mesh>) -> Vec<(Handle<Mesh>, Transform)> {
    let mut parts = vec![(meshes.add(lathe(&profile(role))), Transform::IDENTITY)];

    match role {
        Role::Rook => {
            // Four merlons around the rim.
            let merlon = meshes.add(Cuboid::new(0.085, 0.075, 0.085));
            for i in 0..4 {
                let angle = std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4;
                let r = 0.185;
                parts.push((
                    merlon.clone(),
                    Transform::from_xyz(angle.cos() * r, 0.735, angle.sin() * r),
                ));
            }
        }
        Role::Knight => {
            parts.push((
                meshes.add(extrude(&knight_outline(), KNIGHT_THICKNESS)),
                // Turned to look up the board at the opponent.
                Transform::from_rotation(Quat::from_rotation_y(KNIGHT_FACING)),
            ));
        }
        Role::Bishop => {
            // The mitre's slit.
            parts.push((
                meshes.add(Cuboid::new(0.030, 0.140, 0.190)),
                Transform::from_xyz(0.0, 0.790, 0.030),
            ));
        }
        Role::Queen => {
            // A ring of points around the coronet.
            let point = meshes.add(Sphere::new(0.044));
            for i in 0..7 {
                let angle = std::f32::consts::TAU * i as f32 / 7.0;
                let r = 0.196;
                parts.push((
                    point.clone(),
                    Transform::from_xyz(angle.cos() * r, 0.898, angle.sin() * r),
                ));
            }
            parts.push((
                meshes.add(Sphere::new(0.052)),
                Transform::from_xyz(0.0, 0.952, 0.0),
            ));
        }
        Role::King => {
            // Surmounted by a cross.
            parts.push((
                meshes.add(Cuboid::new(0.052, 0.185, 0.052)),
                Transform::from_xyz(0.0, 1.045, 0.0),
            ));
            parts.push((
                meshes.add(Cuboid::new(0.145, 0.052, 0.052)),
                Transform::from_xyz(0.0, 1.072, 0.0),
            ));
        }
        Role::Pawn => {}
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    const ROLES: [Role; 6] = [
        Role::Pawn,
        Role::Knight,
        Role::Bishop,
        Role::Rook,
        Role::Queen,
        Role::King,
    ];

    fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => panic!("mesh has no float3 positions"),
        }
    }

    fn normals(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => panic!("mesh has no float3 normals"),
        }
    }

    fn index_count(mesh: &Mesh) -> usize {
        mesh.indices().map(|i| i.len()).unwrap_or(0)
    }

    #[test]
    fn every_lathed_piece_produces_sane_geometry() {
        for role in ROLES {
            let mesh = lathe(&profile(role));
            let pos = positions(&mesh);
            let nrm = normals(&mesh);
            assert_eq!(pos.len(), nrm.len(), "{role:?}: attribute lengths differ");
            assert!(!pos.is_empty(), "{role:?}: no vertices");
            assert!(
                index_count(&mesh) % 3 == 0,
                "{role:?}: indices are not triangles"
            );

            for p in &pos {
                assert!(
                    p.iter().all(|c| c.is_finite()),
                    "{role:?}: non-finite vertex {p:?}"
                );
            }
            for n in &nrm {
                let len = Vec3::from_array(*n).length();
                assert!(
                    (len - 1.0).abs() < 1e-3,
                    "{role:?}: normal not unit length ({len})"
                );
            }
        }
    }

    #[test]
    fn indices_stay_inside_the_vertex_buffer() {
        for role in ROLES {
            let mesh = lathe(&profile(role));
            let count = positions(&mesh).len() as u32;
            if let Some(bevy::mesh::Indices::U32(idx)) = mesh.indices() {
                assert!(
                    idx.iter().all(|i| *i < count),
                    "{role:?}: index out of range"
                );
            }
        }
    }

    #[test]
    fn pieces_stand_on_the_board_and_have_believable_heights() {
        for role in ROLES {
            let pos = positions(&lathe(&profile(role)));
            let min_y = pos.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            let max_y = pos.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            assert!(
                min_y.abs() < 1e-4,
                "{role:?} floats or sinks: base at {min_y}"
            );
            assert!(max_y > 0.3, "{role:?} is too short at {max_y}");
            assert!(max_y < 1.2, "{role:?} is too tall at {max_y}");
        }
    }

    #[test]
    fn the_king_is_the_tallest_and_the_pawn_the_shortest() {
        let height = |role| {
            positions(&lathe(&profile(role)))
                .iter()
                .map(|p| p[1])
                .fold(f32::MIN, f32::max)
        };
        let pawn = height(Role::Pawn);
        let king = height(Role::King);
        let queen = height(Role::Queen);
        assert!(king > queen, "king {king} should top the queen {queen}");
        assert!(queen > pawn, "queen {queen} should top the pawn {pawn}");
    }

    #[test]
    fn pieces_fit_within_their_square() {
        // Half a square, with a little clearance so neighbours never intersect.
        let limit = 1.2 / 2.0;
        for role in ROLES {
            let pos = positions(&lathe(&profile(role)));
            let widest = pos
                .iter()
                .map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt())
                .fold(0.0f32, f32::max);
            assert!(
                widest < limit,
                "{role:?} is {widest} wide, wider than its square"
            );
        }
    }

    #[test]
    fn the_knight_outline_is_extruded_into_a_solid() {
        let mesh = extrude(&knight_outline(), KNIGHT_THICKNESS);
        let pos = positions(&mesh);
        assert!(!pos.is_empty());
        assert!(index_count(&mesh) % 3 == 0);
        let depth = pos.iter().map(|p| p[2]).fold(f32::MIN, f32::max)
            - pos.iter().map(|p| p[2]).fold(f32::MAX, f32::min);
        assert!(
            (depth - KNIGHT_THICKNESS).abs() < 1e-4,
            "knight depth was {depth}"
        );
        for p in &pos {
            assert!(p.iter().all(|c| c.is_finite()));
        }
    }
}

#[cfg(test)]
mod extrude_tests {
    use super::*;
    use bevy::mesh::{Indices, VertexAttributeValues};

    fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => panic!("no positions"),
        }
    }

    fn indices(mesh: &Mesh) -> Vec<u32> {
        match mesh.indices() {
            Some(Indices::U32(v)) => v.clone(),
            _ => panic!("no u32 indices"),
        }
    }

    fn polygon_area(poly: &[Vec2]) -> f32 {
        signed_area_2(poly).abs() / 2.0
    }

    fn ccw_knight() -> Vec<Vec2> {
        let mut p = knight_outline();
        if signed_area_2(&p) < 0.0 {
            p.reverse();
        }
        p
    }

    #[test]
    fn the_knight_outline_really_is_concave() {
        // If this ever became convex, a cheaper triangulation would do; while
        // it is concave, a centroid fan lays triangles across the notches.
        let poly = ccw_knight();
        let n = poly.len();
        let reflex = (0..n)
            .filter(|&i| cross(poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]) <= 0.0)
            .count();
        assert!(reflex > 0, "expected a concave outline, found none");
    }

    #[test]
    fn triangulation_covers_the_outline_exactly() {
        // Catches both a fan spilling outside a concave outline and any gap.
        let poly = ccw_knight();
        let tris = triangulate(&poly);
        assert_eq!(
            tris.len(),
            poly.len() - 2,
            "a simple polygon has n-2 triangles"
        );

        let total: f32 = tris
            .iter()
            .map(|t| cross(poly[t[0]], poly[t[1]], poly[t[2]]).abs() / 2.0)
            .sum();
        let expected = polygon_area(&poly);
        assert!(
            (total - expected).abs() < 1e-5,
            "triangles cover {total:.5} but the outline is {expected:.5}"
        );
    }

    #[test]
    fn every_cap_triangle_is_wound_the_same_way() {
        let poly = ccw_knight();
        for t in triangulate(&poly) {
            assert!(
                cross(poly[t[0]], poly[t[1]], poly[t[2]]) > 0.0,
                "a triangle came out inside-out"
            );
        }
    }

    #[test]
    fn the_front_cap_faces_the_camera_rather_than_away() {
        // This is the bug you could see through: a clockwise outline built the
        // front face backwards, so culling showed the inside of the piece.
        let mesh = extrude(&knight_outline(), KNIGHT_THICKNESS);
        let pos = positions(&mesh);
        let idx = indices(&mesh);

        let mut checked = 0;
        for tri in idx.chunks(3) {
            let (a, b, c) = (
                Vec3::from_array(pos[tri[0] as usize]),
                Vec3::from_array(pos[tri[1] as usize]),
                Vec3::from_array(pos[tri[2] as usize]),
            );
            // Only the front cap: all three vertices at +half.
            if a.z > 0.0 && b.z > 0.0 && c.z > 0.0 {
                let normal = (b - a).cross(c - a);
                assert!(
                    normal.z > 0.0,
                    "a front-cap triangle faces away from the viewer"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "found no front-cap triangles to check");
    }

    #[test]
    fn a_clockwise_outline_builds_the_same_solid_as_a_counter_clockwise_one() {
        // The knight outline is written clockwise; callers should not have to
        // know or care.
        let cw = {
            let mut p = knight_outline();
            if signed_area_2(&p) > 0.0 {
                p.reverse();
            }
            p
        };
        let ccw = {
            let mut p = cw.clone();
            p.reverse();
            p
        };
        assert!(signed_area_2(&cw) < 0.0 && signed_area_2(&ccw) > 0.0);

        let a = extrude(&cw, KNIGHT_THICKNESS);
        let b = extrude(&ccw, KNIGHT_THICKNESS);
        assert_eq!(positions(&a).len(), positions(&b).len());
        assert_eq!(indices(&a).len(), indices(&b).len());

        // Both must present outward-facing front caps.
        for mesh in [&a, &b] {
            let pos = positions(mesh);
            for tri in indices(mesh).chunks(3) {
                let (p0, p1, p2) = (
                    Vec3::from_array(pos[tri[0] as usize]),
                    Vec3::from_array(pos[tri[1] as usize]),
                    Vec3::from_array(pos[tri[2] as usize]),
                );
                if p0.z > 0.0 && p1.z > 0.0 && p2.z > 0.0 {
                    assert!((p1 - p0).cross(p2 - p0).z > 0.0);
                }
            }
        }
    }

    /// Ray-casting point-in-polygon, valid for concave outlines.
    fn inside(poly: &[Vec2], p: Vec2) -> bool {
        let n = poly.len();
        let mut hit = false;
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (poly[i], poly[j]);
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                hit = !hit;
            }
            j = i;
        }
        hit
    }

    #[test]
    fn the_side_wall_normals_point_outward() {
        // Comparing against the centroid would be wrong here: on a concave
        // outline an outward normal can point back toward the middle. Stepping
        // along the normal must leave the shape, and stepping against it must
        // stay inside.
        let poly = ccw_knight();
        let eps = 1e-3;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            let edge = (b - a).normalize_or(Vec2::X);
            let nrm = Vec2::new(edge.y, -edge.x);
            let mid = (a + b) / 2.0;
            assert!(
                !inside(&poly, mid + nrm * eps),
                "edge {i}: stepping along the normal stayed inside the piece"
            );
            assert!(
                inside(&poly, mid - nrm * eps),
                "edge {i}: stepping against the normal left the piece"
            );
        }
    }
}

#[cfg(test)]
mod knight_tests {
    use super::*;

    #[test]
    fn the_knight_looks_up_the_board_at_the_opponent() {
        // Modelled with the nose toward +X; White's home rank is at -Z, so the
        // nose has to end up pointing toward +Z. Turning it the other way
        // makes White's knights face their own player.
        let turned = Quat::from_rotation_y(KNIGHT_FACING) * Vec3::X;
        assert!(
            turned.z > 0.9,
            "the knight's nose points {turned:?}, not up the board"
        );
    }

    #[test]
    fn a_black_knight_faces_the_other_way() {
        // Black pieces are turned a further half-turn when spawned.
        let black = Quat::from_rotation_y(std::f32::consts::PI)
            * (Quat::from_rotation_y(KNIGHT_FACING) * Vec3::X);
        assert!(
            black.z < -0.9,
            "Black's knight should look back down the board"
        );
    }

    #[test]
    fn the_head_sits_on_the_foot_with_no_gap() {
        // The turned foot ends at y = 0.345; the head must reach below that or
        // it floats.
        let lowest = knight_outline()
            .iter()
            .map(|p| p.y)
            .fold(f32::MAX, f32::min);
        assert!(
            lowest < 0.345,
            "the head starts at {lowest}, above the foot"
        );
    }

    #[test]
    fn the_whole_knight_fits_within_its_square() {
        // The extruded head sticks out along the board, so it needs checking
        // as well as the turned profile.
        let half = 1.2 / 2.0;
        let rotation = Quat::from_rotation_y(KNIGHT_FACING);
        for p in knight_outline() {
            for z in [-KNIGHT_THICKNESS / 2.0, KNIGHT_THICKNESS / 2.0] {
                let corner = rotation * Vec3::new(p.x, p.y, z);
                let reach = (corner.x * corner.x + corner.z * corner.z).sqrt();
                assert!(reach < half, "the head reaches {reach}, past its square");
            }
        }
    }

    #[test]
    fn the_ears_survive_triangulation() {
        // The notch between the ears is the concave detail most likely to be
        // swallowed; the cap must still cover the outline exactly.
        let mut poly = knight_outline();
        if signed_area_2(&poly) < 0.0 {
            poly.reverse();
        }
        let tris = triangulate(&poly);
        let covered: f32 = tris
            .iter()
            .map(|t| cross(poly[t[0]], poly[t[1]], poly[t[2]]).abs() / 2.0)
            .sum();
        let expected = signed_area_2(&poly).abs() / 2.0;
        assert!((covered - expected).abs() < 1e-5, "{covered} vs {expected}");
    }
}
