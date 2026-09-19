//! The viewpoint: a camera that orbits the board and looks around.
//!
//! There is no character to walk: the courtyard is scenery, seen from a camera
//! you steer. That also means nothing moves unless you move it, which is what
//! lets the app stop drawing when it is left alone.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

use crate::board::{BOARD_CENTER, BOARD_SURFACE_Y};
use crate::interaction::PendingPromotion;
use crate::ui::RatingEntry;
use shakmaty::Color as ChessColor;

/// How far one wheel notch moves the camera, as a fraction of the distance.
const ZOOM_PER_NOTCH: f32 = 0.11;
/// Trackpad pixels that count as one notch.
const PIXELS_PER_NOTCH: f32 = 45.0;
/// Higher settles the zoom faster.
const ZOOM_SMOOTHING: f32 = 14.0;
/// Higher swings the camera to a new angle faster.
const ORBIT_SMOOTHING: f32 = 12.0;
/// Keys that swing the view, and how fast, in radians per second.
///
/// These are read while *held*, so none of them may share a letter with a
/// promotion choice: ordering stops the picker's own keypress leaking, but a
/// held key keeps firing for every frame the finger stays down afterwards.
pub const ORBIT_KEYS: [(KeyCode, f32, f32); 4] = [
    (KeyCode::KeyA, 1.1, 0.0),
    (KeyCode::KeyD, -1.1, 0.0),
    (KeyCode::KeyW, 0.0, 0.9),
    (KeyCode::KeyS, 0.0, -0.9),
];

/// Keys that zoom, and in which direction. Bracket keys rather than Q and E,
/// which belong to the promotion picker.
pub const ZOOM_KEYS: [(KeyCode, f32); 2] =
    [(KeyCode::BracketLeft, -4.0), (KeyCode::BracketRight, 4.0)];

/// Below this the camera counts as having arrived, and drawing can stop.
const SETTLED: f32 = 0.002;
/// Vertical field of view. Narrow enough to keep the board from distorting.
pub const CAMERA_FOV_DEGREES: f32 = 52.0;

/// Marks the one camera.
#[derive(Component)]
pub struct BoardView;

/// Where the camera is looking from.
///
/// The orbit is stored as angles and a distance, and it is those that ease
/// toward their targets — not the camera's position. Interpolating position
/// instead sends the camera on a straight line between two points, which for
/// a half-turn runs directly through the middle of the board.
#[derive(Resource)]
pub struct BoardCamera {
    /// Rotation around the board. 0 looks down the board from White's side.
    pub yaw: f32,
    /// Elevation. Larger is more top-down.
    pub pitch: f32,
    pub distance: f32,
    pub target_yaw: f32,
    pub target_pitch: f32,
    pub target_distance: f32,
}

impl Default for BoardCamera {
    fn default() -> Self {
        // Far enough back that the whole board is in shot and both back ranks
        // are seen at a similar angle.
        Self::at(0.0, 0.92, 25.5)
    }
}

/// Eases an angle toward another the short way round, so a half-turn never
/// unwinds the long way.
fn ease_angle(current: f32, target: f32, t: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let delta = (target - current + PI).rem_euclid(TAU) - PI;
    current + delta * t
}

impl BoardCamera {
    /// A camera already settled at the given orbit.
    pub fn at(yaw: f32, pitch: f32, distance: f32) -> Self {
        Self {
            yaw,
            pitch,
            distance,
            target_yaw: yaw,
            target_pitch: pitch,
            target_distance: distance,
        }
    }

    /// You may now look all the way round; the default still sits behind White.
    pub const PITCH_MIN: f32 = 0.14;
    pub const PITCH_MAX: f32 = 1.45;
    pub const DISTANCE_MIN: f32 = 7.0;
    pub const DISTANCE_MAX: f32 = 42.0;

    pub fn clamp(&mut self) {
        self.target_pitch = self.target_pitch.clamp(Self::PITCH_MIN, Self::PITCH_MAX);
        self.pitch = self.pitch.clamp(Self::PITCH_MIN, Self::PITCH_MAX);
        self.target_yaw = self.target_yaw.rem_euclid(std::f32::consts::TAU);
        self.yaw = self.yaw.rem_euclid(std::f32::consts::TAU);
        self.target_distance = self
            .target_distance
            .clamp(Self::DISTANCE_MIN, Self::DISTANCE_MAX);
        self.distance = self.distance.clamp(Self::DISTANCE_MIN, Self::DISTANCE_MAX);
    }

    /// One "notch" of zoom. Proportional, so it feels the same near and far.
    pub fn zoom(&mut self, notches: f32) {
        self.target_distance *= (1.0 - notches * ZOOM_PER_NOTCH).clamp(0.4, 2.5);
        self.target_distance = self
            .target_distance
            .clamp(Self::DISTANCE_MIN, Self::DISTANCE_MAX);
    }

    /// Eases the whole orbit toward its target, so every move glides.
    pub fn settle(&mut self, dt: f32) {
        let zoom_t = 1.0 - (-ZOOM_SMOOTHING * dt).exp();
        self.distance += (self.target_distance - self.distance) * zoom_t;
        let orbit_t = 1.0 - (-ORBIT_SMOOTHING * dt).exp();
        self.yaw = ease_angle(self.yaw, self.target_yaw, orbit_t);
        self.pitch += (self.target_pitch - self.pitch) * orbit_t;
    }

    /// True once the orbit has arrived and there is nothing left to animate.
    pub fn is_settled(&self) -> bool {
        use std::f32::consts::{PI, TAU};
        let yaw_gap = ((self.target_yaw - self.yaw + PI).rem_euclid(TAU) - PI).abs();
        (self.target_distance - self.distance).abs() < SETTLED
            && (self.target_pitch - self.pitch).abs() < SETTLED
            && yaw_gap < SETTLED
    }

    /// Swing round to sit behind the given side, so you look at the board
    /// from your own end of it.
    pub fn face(&mut self, side: ChessColor) {
        self.target_yaw = match side {
            ChessColor::White => 0.0,
            ChessColor::Black => std::f32::consts::PI,
        };
    }

    /// Point the camera orbits around: the middle of the board.
    pub fn focus() -> Vec3 {
        BOARD_CENTER + Vec3::Y * BOARD_SURFACE_Y
    }

    /// Camera position for the current orbit. At yaw 0 this is White's side.
    pub fn position(&self) -> Vec3 {
        let dir = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        );
        Self::focus() + dir * self.distance
    }

    /// Where the camera should be pointing.
    pub fn transform(&self) -> Transform {
        Transform::from_translation(self.position()).looking_at(Self::focus(), Vec3::Y)
    }
}

pub fn spawn_camera(mut commands: Commands, camera: Res<BoardCamera>) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: CAMERA_FOV_DEGREES.to_radians(),
            ..default()
        }),
        camera.transform(),
        DistanceFog {
            // Matches the horizon, so the ground fades into the sky.
            color: Color::srgb(0.70, 0.82, 0.94),
            falloff: FogFalloff::Linear {
                start: 45.0,
                end: 120.0,
            },
            ..default()
        },
        BoardView,
    ));
}

/// Right-drag to look around, scroll to zoom, WASD to nudge.
pub fn orbit_camera(
    entry: Res<RatingEntry>,
    promotion: Res<PendingPromotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut camera: ResMut<BoardCamera>,
) {
    let dt = time.delta_secs();
    if entry.active || promotion.is_open() {
        camera.settle(dt);
        return;
    }

    // Right button keeps the left one free for moving pieces.
    if buttons.pressed(MouseButton::Right) && motion.delta != Vec2::ZERO {
        camera.target_yaw -= motion.delta.x * 0.005;
        camera.target_pitch += motion.delta.y * 0.005;
    }

    // WASD nudges the view; the arrows belong to the move history.
    for (key, yaw, pitch) in ORBIT_KEYS {
        if keys.pressed(key) {
            camera.target_yaw += dt * yaw;
            camera.target_pitch += dt * pitch;
        }
    }

    // Wheels report whole lines; trackpads report pixels, tens at a time.
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_NOTCH,
    };
    if notches != 0.0 {
        camera.zoom(notches);
    }
    for (key, rate) in ZOOM_KEYS {
        if keys.pressed(key) {
            camera.zoom(dt * rate);
        }
    }

    camera.clamp();
    camera.settle(dt);
}

/// Moves the camera to wherever the orbit now says it should be.
pub fn drive_camera(camera: Res<BoardCamera>, mut views: Query<&mut Transform, With<BoardView>>) {
    let Ok(mut tf) = views.single_mut() else {
        return;
    };
    // Smoothing already happened on the orbit itself, so this just follows it
    // round the board rather than cutting across the middle.
    *tf = camera.transform();
}

/// True while the camera still has somewhere to go, so drawing must continue.
pub fn camera_is_moving(camera: &BoardCamera, tf: &Transform) -> bool {
    !camera.is_settled() || tf.translation.distance(camera.position()) > SETTLED
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::square_to_world;

    fn sq(name: &str) -> shakmaty::Square {
        name.parse().unwrap()
    }

    fn cam_at(distance: f32) -> BoardCamera {
        BoardCamera {
            distance,
            target_distance: distance,
            ..default()
        }
    }

    fn notches(unit: MouseScrollUnit, delta_y: f32) -> f32 {
        match unit {
            MouseScrollUnit::Line => delta_y,
            MouseScrollUnit::Pixel => delta_y / PIXELS_PER_NOTCH,
        }
    }

    #[test]
    fn the_default_view_looks_from_behind_white() {
        let pos = BoardCamera::default().position();
        let white = square_to_world(sq("e1")).z;
        let black = square_to_world(sq("e8")).z;
        assert!(
            pos.z.signum() == white.signum(),
            "camera at z={} is not on White's side (White {white}, Black {black})",
            pos.z
        );
        assert!(pos.y > BoardCamera::focus().y, "camera is below the board");
    }

    #[test]
    fn the_view_can_now_swing_all_the_way_round() {
        // No walking any more, so the camera is free to orbit fully.
        let mut cam = BoardCamera::default();
        let mut seen_behind_black = false;
        for _ in 0..24 {
            cam.yaw += std::f32::consts::TAU / 24.0;
            cam.clamp();
            if cam.position().z > BoardCamera::focus().z {
                seen_behind_black = true;
            }
        }
        assert!(seen_behind_black, "never reached the far side");
    }

    #[test]
    fn yaw_wraps_instead_of_growing_without_bound() {
        let mut cam = BoardCamera::default();
        cam.yaw = std::f32::consts::TAU * 5.0 + 1.0;
        cam.clamp();
        assert!(
            (0.0..std::f32::consts::TAU).contains(&cam.yaw),
            "yaw was {}",
            cam.yaw
        );
    }

    #[test]
    fn pitch_stays_above_the_board_and_below_straight_down() {
        let mut cam = BoardCamera::default();
        cam.pitch = -10.0;
        cam.clamp();
        assert_eq!(cam.pitch, BoardCamera::PITCH_MIN);
        cam.pitch = 10.0;
        cam.clamp();
        assert_eq!(cam.pitch, BoardCamera::PITCH_MAX);
    }

    #[test]
    fn a_trackpad_and_a_wheel_zoom_by_the_same_amount() {
        let mut wheel = cam_at(18.0);
        wheel.zoom(notches(MouseScrollUnit::Line, 1.0));
        let mut trackpad = cam_at(18.0);
        trackpad.zoom(notches(MouseScrollUnit::Pixel, PIXELS_PER_NOTCH));
        assert!((wheel.target_distance - trackpad.target_distance).abs() < 1e-3);
    }

    #[test]
    fn one_trackpad_flick_does_not_cross_the_whole_range() {
        let mut cam = cam_at(18.0);
        cam.zoom(notches(MouseScrollUnit::Pixel, 120.0));
        assert!(cam.target_distance > BoardCamera::DISTANCE_MIN + 1.0);
    }

    #[test]
    fn zoom_is_proportional_so_it_feels_the_same_near_and_far() {
        let mut near = cam_at(15.0);
        let mut far = cam_at(30.0);
        near.zoom(1.0);
        far.zoom(1.0);
        assert!(((near.target_distance / 15.0) - (far.target_distance / 30.0)).abs() < 1e-4);
    }

    #[test]
    fn zoom_never_escapes_its_limits() {
        let mut cam = cam_at(18.0);
        for _ in 0..200 {
            cam.zoom(5.0);
        }
        assert!(cam.target_distance >= BoardCamera::DISTANCE_MIN);
        for _ in 0..200 {
            cam.zoom(-5.0);
        }
        assert!(cam.target_distance <= BoardCamera::DISTANCE_MAX);
    }

    #[test]
    fn distance_eases_toward_the_target_rather_than_jumping() {
        let mut cam = cam_at(18.0);
        cam.zoom(3.0);
        let target = cam.target_distance;
        cam.settle(1.0 / 60.0);
        assert!(cam.distance < 18.0 && cam.distance > target);
        for _ in 0..240 {
            cam.settle(1.0 / 60.0);
        }
        assert!((cam.distance - target).abs() < 0.01);
    }

    // --- the checks the idle logic depends on -----------------------------

    #[test]
    fn a_still_camera_reports_settled_so_drawing_can_stop() {
        let cam = BoardCamera::default();
        let tf = cam.transform();
        assert!(cam.is_settled());
        assert!(
            !camera_is_moving(&cam, &tf),
            "a camera that has arrived must not keep the app awake"
        );
    }

    #[test]
    fn a_zooming_camera_reports_moving_so_drawing_continues() {
        let mut cam = BoardCamera::default();
        let tf = cam.transform();
        cam.zoom(2.0);
        assert!(!cam.is_settled());
        assert!(camera_is_moving(&cam, &tf), "drawing would stop mid-zoom");
    }

    #[test]
    fn a_camera_mid_swing_reports_moving() {
        let mut cam = BoardCamera::default();
        let tf = cam.transform();
        // Orbit the target without moving the transform yet.
        cam.yaw += 0.6;
        assert!(
            camera_is_moving(&cam, &tf),
            "drawing would stop before the swing finished"
        );
    }

    #[test]
    fn the_camera_settles_and_then_lets_the_app_sleep() {
        let mut cam = BoardCamera::default();
        cam.zoom(2.0);
        for _ in 0..600 {
            cam.settle(1.0 / 60.0);
        }
        let tf = cam.transform();
        assert!(!camera_is_moving(&cam, &tf), "never became still");
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    use crate::board::square_to_world;

    fn sq(name: &str) -> shakmaty::Square {
        name.parse().unwrap()
    }

    #[test]
    fn both_back_ranks_are_seen_from_a_similar_distance() {
        let cam = BoardCamera::default().position();
        let near = cam.distance(square_to_world(sq("e1")));
        let far = cam.distance(square_to_world(sq("e8")));
        assert!(
            far / near < 1.35,
            "far rank is {:.2}x the near one; camera is too close",
            far / near
        );
    }

    #[test]
    fn the_whole_board_fits_in_the_default_view() {
        // Checking the angle to the view axis is too lenient: it mixes the
        // vertical and horizontal offsets together, and the vertical field of
        // view is the tighter of the two. Resolve each axis separately, or a
        // back rank gets clipped by the bottom edge while the test passes.
        for side in [ChessColor::White, ChessColor::Black] {
            let mut cam = BoardCamera::default();
            cam.face(side);
            cam.clamp();

            let view = cam.transform().to_matrix().inverse();
            let half_v = CAMERA_FOV_DEGREES.to_radians() / 2.0;
            // The window ships at 1440x900.
            let half_h = (half_v.tan() * (1440.0 / 900.0)).atan();

            for name in ["a1", "h1", "a8", "h8"] {
                let local = view.transform_point3(square_to_world(sq(name)));
                // Camera space looks down -Z.
                let depth = -local.z;
                assert!(depth > 0.0, "{name} is behind the camera");
                let vertical = (local.y / depth).atan().abs();
                let horizontal = (local.x / depth).atan().abs();
                assert!(
                    vertical < half_v * 0.78,
                    "{side:?}: {name} is {:.1}deg above centre, past the {:.1}deg edge",
                    vertical.to_degrees(),
                    half_v.to_degrees()
                );
                assert!(
                    horizontal < half_h * 0.78,
                    "{side:?}: {name} is {:.1}deg off centre, past the {:.1}deg edge",
                    horizontal.to_degrees(),
                    half_h.to_degrees()
                );
            }
        }
    }
}

#[cfg(test)]
mod side_tests {
    use super::*;
    use crate::board::square_to_world;

    fn sq(name: &str) -> shakmaty::Square {
        name.parse().unwrap()
    }

    /// Runs the easing to completion, as the app does over a few frames.
    fn settle_fully(cam: &mut BoardCamera) {
        for _ in 0..600 {
            cam.settle(1.0 / 60.0);
        }
    }

    #[test]
    fn facing_white_puts_the_camera_behind_whites_rank() {
        let mut cam = BoardCamera::default();
        cam.face(ChessColor::White);
        cam.clamp();
        settle_fully(&mut cam);
        let z = cam.position().z;
        assert!(
            z.signum() == square_to_world(sq("e1")).z.signum(),
            "camera at z={z} is not behind White"
        );
    }

    #[test]
    fn facing_black_puts_the_camera_behind_blacks_rank() {
        let mut cam = BoardCamera::default();
        cam.face(ChessColor::Black);
        cam.clamp();
        settle_fully(&mut cam);
        let z = cam.position().z;
        assert!(
            z.signum() == square_to_world(sq("e8")).z.signum(),
            "camera at z={z} is not behind Black"
        );
    }

    #[test]
    fn swapping_sides_moves_the_camera_to_the_other_end() {
        let mut cam = BoardCamera::default();
        cam.face(ChessColor::White);
        cam.clamp();
        settle_fully(&mut cam);
        let white_z = cam.position().z;
        cam.face(ChessColor::Black);
        cam.clamp();
        settle_fully(&mut cam);
        let black_z = cam.position().z;
        assert!(
            white_z.signum() != black_z.signum(),
            "both sides ended up at the same end ({white_z} and {black_z})"
        );
    }

    #[test]
    fn swapping_sides_orbits_around_the_board_not_through_it() {
        // Interpolating the camera's position instead of its orbit sends it on
        // a straight line between the two ends, which passes through the
        // middle of the board and flies the view between the pieces.
        let mut cam = BoardCamera::default();
        cam.face(ChessColor::White);
        cam.clamp();
        settle_fully(&mut cam);

        let radius = cam.position().distance(BoardCamera::focus());
        cam.face(ChessColor::Black);
        cam.clamp();

        let mut closest = f32::MAX;
        for _ in 0..600 {
            cam.settle(1.0 / 60.0);
            closest = closest.min(cam.position().distance(BoardCamera::focus()));
        }

        assert!(
            closest > radius * 0.9,
            "camera closed to {closest:.1} of the board centre mid-swing, \
             against a {radius:.1} orbit: it cut through the board"
        );
    }

    #[test]
    fn facing_a_side_keeps_the_height_and_distance() {
        let mut cam = BoardCamera::default();
        let before = cam.position().distance(BoardCamera::focus());
        cam.face(ChessColor::Black);
        cam.clamp();
        settle_fully(&mut cam);
        let after = cam.position().distance(BoardCamera::focus());
        assert!((before - after).abs() < 1e-4, "distance changed on flip");
        assert!(cam.position().y > BoardCamera::focus().y);
    }
}

#[cfg(test)]
mod orientation_tests {
    use super::*;
    use crate::board::square_to_world;

    fn sq(name: &str) -> shakmaty::Square {
        name.parse().unwrap()
    }

    /// How far right of centre a square appears, from a camera facing `side`.
    fn screen_x(side: ChessColor, name: &str) -> f32 {
        let mut cam = BoardCamera::default();
        cam.face(side);
        cam.clamp();
        for _ in 0..600 {
            cam.settle(1.0 / 60.0);
        }
        let tf = cam.transform();
        // Bevy cameras look down local -Z, so local +X is screen right.
        let screen_right = tf.rotation * Vec3::X;
        screen_right.dot((square_to_world(sq(name)) - cam.position()).normalize())
    }

    #[test]
    fn white_sees_the_a_file_on_the_left() {
        assert!(
            screen_x(ChessColor::White, "a1") < 0.0,
            "the board is mirrored: a1 is on White's right"
        );
        assert!(screen_x(ChessColor::White, "h1") > 0.0);
    }

    #[test]
    fn white_sees_the_queen_to_the_left_of_the_king() {
        // The decisive check: in the opening position White's queen stands on
        // d1 and the king on e1, queen to the left.
        let queen = screen_x(ChessColor::White, "d1");
        let king = screen_x(ChessColor::White, "e1");
        assert!(
            queen < king,
            "queen at {queen} is not left of the king at {king}"
        );
    }

    #[test]
    fn black_sees_its_own_board_the_right_way_round() {
        // Everything mirrors from the other end. The queens face each other
        // down the d-file, so the file that is on White's left is on Black's
        // right: Black's queen stands to the right of Black's king.
        assert!(
            screen_x(ChessColor::Black, "a8") > 0.0,
            "Black's a-file should be on Black's right"
        );
        let queen = screen_x(ChessColor::Black, "d8");
        let king = screen_x(ChessColor::Black, "e8");
        assert!(
            queen > king,
            "queen at {queen} should be right of the king at {king}, as Black sees it"
        );
    }
}

#[cfg(test)]
mod tray_framing_tests {
    use super::*;
    use crate::board::tray_position;

    /// Angles from the view axis, resolved per screen axis.
    fn offsets(cam: &BoardCamera, point: Vec3) -> (f32, f32) {
        let view = cam.transform().to_matrix().inverse();
        let local = view.transform_point3(point);
        let depth = -local.z;
        (
            (local.x / depth).atan().abs(),
            (local.y / depth).atan().abs(),
        )
    }

    #[test]
    fn the_taken_pieces_are_actually_in_shot() {
        // A pile you cannot see is no use. Both ends of both piles must sit
        // inside the frame from the side that owns them.
        let half_v = CAMERA_FOV_DEGREES.to_radians() / 2.0;
        let half_h = (half_v.tan() * (1440.0 / 900.0)).atan();

        for side in [ChessColor::White, ChessColor::Black] {
            let mut cam = BoardCamera::default();
            cam.face(side);
            cam.clamp();
            for _ in 0..600 {
                cam.settle(1.0 / 60.0);
            }
            for owner in [ChessColor::White, ChessColor::Black] {
                for index in [0usize, 7, 14] {
                    let (h, v) = offsets(&cam, tray_position(owner, index));
                    assert!(
                        h < half_h,
                        "viewed by {side:?}, {owner:?}'s piece {index} is {:.1}deg off centre, \
                         past the {:.1}deg edge",
                        h.to_degrees(),
                        half_h.to_degrees()
                    );
                    assert!(
                        v < half_v,
                        "{owner:?} piece {index} is off the top or bottom"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use crate::interaction::PROMOTION_KEYS;

    #[test]
    fn no_held_camera_key_shares_a_letter_with_a_promotion() {
        // Ordering keeps the picker's own keypress from leaking into the
        // camera, but only for the frame the picker closes on. A key read
        // while held keeps firing for as long as the finger is down, so a
        // normal tap of Q would promote and then zoom for ten more frames.
        let held: Vec<KeyCode> = ORBIT_KEYS
            .iter()
            .map(|(key, _, _)| *key)
            .chain(ZOOM_KEYS.iter().map(|(key, _)| *key))
            .collect();

        for (key, role) in PROMOTION_KEYS {
            assert!(
                !held.contains(&key),
                "{role:?} is chosen with a key the camera also reads while held"
            );
        }
    }

    #[test]
    fn the_camera_keys_do_not_collide_with_each_other() {
        let mut keys: Vec<KeyCode> = ORBIT_KEYS
            .iter()
            .map(|(k, _, _)| *k)
            .chain(ZOOM_KEYS.iter().map(|(k, _)| *k))
            .collect();
        let before = keys.len();
        keys.sort_by_key(|k| format!("{k:?}"));
        keys.dedup();
        assert_eq!(keys.len(), before, "two camera actions share a key");
    }

    #[test]
    fn the_zoom_keys_pull_in_opposite_directions() {
        let rates: Vec<f32> = ZOOM_KEYS.iter().map(|(_, r)| *r).collect();
        assert!(
            rates.iter().any(|r| *r > 0.0) && rates.iter().any(|r| *r < 0.0),
            "zoom only works one way"
        );
    }

    #[test]
    fn orbiting_covers_both_axes_in_both_directions() {
        assert!(ORBIT_KEYS.iter().any(|(_, y, _)| *y > 0.0));
        assert!(ORBIT_KEYS.iter().any(|(_, y, _)| *y < 0.0));
        assert!(ORBIT_KEYS.iter().any(|(_, _, p)| *p > 0.0));
        assert!(ORBIT_KEYS.iter().any(|(_, _, p)| *p < 0.0));
    }
}
