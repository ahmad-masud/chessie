//! Rating control below Stockfish's floor.
//!
//! Stockfish's `UCI_Elo` bottoms out around 1320, which is still a solid club
//! player. To offer genuine beginner opponents we keep the engine at its floor
//! and, with a rating-dependent probability, substitute a deliberately weaker
//! legal move for the engine's choice.

use crate::engine::{ENGINE_MIN_ELO, UI_MIN_ELO};
use bevy::prelude::*;
use shakmaty::{Move, Position, Role};

/// Small deterministic PRNG, so we do not pull in a dependency just for this.
#[derive(Resource)]
pub struct Rng(u64);

impl Default for Rng {
    fn default() -> Self {
        // Seed from the clock so successive games differ.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x2545F4914F6CDD1D);
        Self(seed | 1)
    }
}

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let idx = (self.next_u64() % items.len() as u64) as usize;
        items.get(idx)
    }
}

/// The opponent's configured rating.
#[derive(Resource, Clone, Copy)]
pub struct Rating(pub u32);

impl Default for Rating {
    fn default() -> Self {
        Self(1200)
    }
}

impl Rating {
    /// Probability that we discard the engine's move for a weak one.
    pub fn blunder_chance(&self) -> f32 {
        if self.0 >= ENGINE_MIN_ELO {
            return 0.0;
        }
        let span = (ENGINE_MIN_ELO - UI_MIN_ELO) as f32;
        let below = (ENGINE_MIN_ELO - self.0.max(UI_MIN_ELO)) as f32;
        // Ramps to ~0.6 at the very bottom, which plays like a true novice.
        (below / span) * 0.6
    }

    /// Thinking time; weak opponents should also answer quickly.
    pub fn movetime_ms(&self) -> u64 {
        match self.0 {
            0..=800 => 120,
            801..=1400 => 250,
            1401..=2000 => 500,
            2001..=2600 => 900,
            _ => 1500,
        }
    }
}

/// Rough material value, used to rank how bad a candidate move is.
fn role_value(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
    }
}

/// Given the engine's move, decide what the weakened opponent actually plays.
pub fn apply_weakening<P: Position + Clone>(
    position: &P,
    engine_move: Move,
    rating: Rating,
    rng: &mut Rng,
) -> Move {
    let chance = rating.blunder_chance();
    if chance <= 0.0 || rng.next_f32() > chance {
        return engine_move;
    }

    let legal: Vec<Move> = position.legal_moves().into_iter().collect();
    if legal.len() <= 1 {
        return engine_move;
    }

    // Prefer quiet moves that hang material or decline a capture: these are the
    // mistakes real beginners make, rather than uniformly random noise.
    let mut weak: Vec<Move> = legal
        .iter()
        .filter(|m| **m != engine_move)
        .filter(|m| {
            // Passing up a capture the engine wanted is a classic beginner error.
            m.capture().map(role_value).unwrap_or(0)
                <= engine_move.capture().map(role_value).unwrap_or(0)
        })
        .cloned()
        .collect();

    if weak.is_empty() {
        weak = legal.into_iter().filter(|m| *m != engine_move).collect();
    }

    rng.pick(&weak).cloned().unwrap_or(engine_move)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Chess;

    #[test]
    fn blunder_chance_is_zero_at_and_above_the_engine_floor() {
        assert_eq!(Rating(ENGINE_MIN_ELO).blunder_chance(), 0.0);
        assert_eq!(Rating(2500).blunder_chance(), 0.0);
    }

    #[test]
    fn blunder_chance_rises_as_rating_falls() {
        let low = Rating(UI_MIN_ELO).blunder_chance();
        let mid = Rating(800).blunder_chance();
        assert!(low > mid, "{low} should exceed {mid}");
        assert!(mid > 0.0);
        assert!(
            low <= 0.6001,
            "blunder chance should stay bounded, got {low}"
        );
    }

    #[test]
    fn weakened_move_is_always_legal() {
        let position = Chess::default();
        let legal: Vec<_> = position.legal_moves().into_iter().collect();
        let engine_move = legal[0].clone();
        let mut rng = Rng::default();

        // At the bottom of the range this substitutes constantly; every result
        // must still be a move we are allowed to play.
        for _ in 0..200 {
            let played =
                apply_weakening(&position, engine_move.clone(), Rating(UI_MIN_ELO), &mut rng);
            assert!(legal.contains(&played), "{played:?} is not legal");
        }
    }

    #[test]
    fn full_strength_never_substitutes() {
        let position = Chess::default();
        let engine_move = position.legal_moves().into_iter().next().unwrap();
        let mut rng = Rng::default();
        for _ in 0..50 {
            let played = apply_weakening(&position, engine_move.clone(), Rating(2800), &mut rng);
            assert_eq!(played, engine_move);
        }
    }

    #[test]
    fn weakening_actually_changes_moves_at_low_ratings() {
        let position = Chess::default();
        let engine_move = position.legal_moves().into_iter().next().unwrap();
        let mut rng = Rng::default();
        let substituted = (0..200)
            .filter(|_| {
                apply_weakening(&position, engine_move.clone(), Rating(300), &mut rng)
                    != engine_move
            })
            .count();
        assert!(
            substituted > 40,
            "expected frequent blunders, saw {substituted}/200"
        );
    }

    #[test]
    fn movetime_increases_with_rating() {
        assert!(Rating(400).movetime_ms() < Rating(3000).movetime_ms());
    }
}
