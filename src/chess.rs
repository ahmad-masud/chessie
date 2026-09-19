//! Chess rules and game state, wrapping `shakmaty`.

use bevy::prelude::*;
use shakmaty::ByColor;
use shakmaty::{fen::Fen, CastlingMode, Chess, Color, EnPassantMode, Move, Position, Role, Square};

/// Lets the player step back through the game to see what happened, without
/// disturbing the live position the engine is playing against.
#[derive(Resource, Default)]
pub struct Review {
    /// `None` follows the live game. `Some(n)` shows the position after n plies.
    pub ply: Option<usize>,
}

impl Review {
    pub fn is_reviewing(&self) -> bool {
        self.ply.is_some()
    }

    /// Which ply is on screen, live or not.
    pub fn shown_ply(&self, game: &Game) -> usize {
        self.ply.unwrap_or(game.history.len())
    }

    pub fn step_back(&mut self, game: &Game) {
        let current = self.shown_ply(game);
        if current > 0 {
            self.ply = Some(current - 1);
        }
    }

    pub fn step_forward(&mut self, game: &Game) {
        let current = self.shown_ply(game);
        if current < game.history.len() {
            let next = current + 1;
            // Stepping onto the newest ply means we are live again.
            self.ply = if next >= game.history.len() {
                None
            } else {
                Some(next)
            };
        }
    }

    pub fn go_live(&mut self) {
        self.ply = None;
    }

    pub fn go_to_start(&mut self, game: &Game) {
        if !game.history.is_empty() {
            self.ply = Some(0);
        }
    }
}

/// What a piece is worth when counting who is ahead. The king is never
/// captured, so it has no value here.
pub fn piece_value(role: Role) -> i32 {
    match role {
        Role::Pawn => 1,
        Role::Knight | Role::Bishop => 3,
        Role::Rook => 5,
        Role::Queen => 9,
        Role::King => 0,
    }
}

/// Which side the human plays.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct HumanSide(pub Color);

impl Default for HumanSide {
    fn default() -> Self {
        Self(Color::White)
    }
}

/// Outcome of the game once it has ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameEnd {
    Checkmate { winner: Color },
    Stalemate,
    InsufficientMaterial,
}

/// The authoritative game state.
#[derive(Resource)]
pub struct Game {
    pub position: Chess,
    pub history: Vec<Move>,
    pub ended: Option<GameEnd>,
}

impl Default for Game {
    fn default() -> Self {
        Self {
            position: Chess::default(),
            history: Vec::new(),
            ended: None,
        }
    }
}

impl Game {
    pub fn reset(&mut self) {
        self.position = Chess::default();
        self.history.clear();
        self.ended = None;
    }

    pub fn turn(&self) -> Color {
        self.position.turn()
    }

    /// FEN string for the current position, for handing to the engine.
    pub fn fen(&self) -> String {
        Fen::from_position(&self.position, EnPassantMode::Legal).to_string()
    }

    /// The position as it stood after `ply` moves, for reviewing the game.
    pub fn position_at(&self, ply: usize) -> Chess {
        if ply >= self.history.len() {
            return self.position.clone();
        }
        let mut pos = Chess::default();
        for mv in self.history.iter().take(ply) {
            pos.play_unchecked(mv.clone());
        }
        pos
    }

    /// Whose move the given ply was. White opens, so even plies are White's.
    pub fn mover_at(ply: usize) -> Color {
        if ply % 2 == 0 {
            Color::White
        } else {
            Color::Black
        }
    }

    /// How far ahead White is in material. Negative means Black leads.
    ///
    /// Read from the board rather than the capture list, so a promotion counts
    /// properly: a pawn that becomes a queen is worth a queen.
    pub fn material_balance_at(&self, ply: usize) -> i32 {
        let position = self.position_at(ply);
        let board = position.board();
        let mut balance = 0;
        for idx in 0..64u8 {
            let Ok(square) = Square::try_from(idx) else {
                continue;
            };
            if let Some(piece) = board.piece_at(square) {
                let value = piece_value(piece.role);
                balance += if piece.color == Color::White {
                    value
                } else {
                    -value
                };
            }
        }
        balance
    }

    /// Which pieces each side has lost, in the order they were taken.
    ///
    /// Taken from the move history rather than by comparing against the
    /// starting line-up, because promotions make that comparison lie: a side
    /// can finish with more queens than it started with.
    pub fn losses_at(&self, ply: usize) -> ByColor<Vec<Role>> {
        let mut losses = ByColor::new_with(|_| Vec::new());
        for (i, mv) in self.history.iter().take(ply).enumerate() {
            if let Some(role) = mv.capture() {
                // The piece taken belonged to whoever was not moving.
                let victim = !Self::mover_at(i);
                losses.get_mut(victim).push(role);
            }
        }
        // Cheapest first, the way a captured pile is usually laid out.
        for color in [Color::White, Color::Black] {
            losses
                .get_mut(color)
                .sort_by_key(|role| (piece_value(*role), *role as u8));
        }
        losses
    }

    pub fn legal_moves(&self) -> Vec<Move> {
        self.position.legal_moves().into_iter().collect()
    }

    /// Every legal move that starts on `from`.
    pub fn legal_moves_from(&self, from: Square) -> Vec<Move> {
        self.legal_moves()
            .into_iter()
            .filter(|m| m.from() == Some(from))
            .collect()
    }

    pub fn is_in_check(&self) -> bool {
        self.position.is_check()
    }

    /// Apply a move, updating history and end state.
    pub fn play(&mut self, m: &Move) {
        self.position.play_unchecked(m.clone());
        self.history.push(m.clone());
        self.refresh_end_state();
    }

    fn refresh_end_state(&mut self) {
        self.ended = if self.position.is_checkmate() {
            Some(GameEnd::Checkmate {
                winner: !self.position.turn(),
            })
        } else if self.position.is_stalemate() {
            Some(GameEnd::Stalemate)
        } else if self.position.is_insufficient_material() {
            Some(GameEnd::InsufficientMaterial)
        } else {
            None
        };
    }

    /// Parse a UCI move string (e.g. "e2e4", "e7e8q") into a legal move.
    pub fn parse_uci(&self, uci: &str) -> Option<Move> {
        let uci: shakmaty::uci::UciMove = uci.parse().ok()?;
        uci.to_move(&self.position).ok()
    }

    /// Serialize a move back to UCI notation.
    pub fn move_to_uci(&self, m: &Move) -> String {
        m.to_uci(CastlingMode::Standard).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_position_has_twenty_legal_moves() {
        let game = Game::default();
        assert_eq!(game.legal_moves().len(), 20);
        assert_eq!(game.turn(), Color::White);
        assert!(game.ended.is_none());
    }

    #[test]
    fn fen_round_trips_through_uci() {
        let mut game = Game::default();
        assert!(game
            .fen()
            .starts_with("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -"));
        let mv = game.parse_uci("e2e4").expect("e2e4 is legal at the start");
        assert_eq!(game.move_to_uci(&mv), "e2e4");
        game.play(&mv);
        assert_eq!(game.turn(), Color::Black);
        assert_eq!(game.history.len(), 1);
        assert!(game.fen().contains(" b "));
    }

    #[test]
    fn illegal_uci_is_rejected() {
        let game = Game::default();
        assert!(game.parse_uci("e2e5").is_none());
        assert!(game.parse_uci("garbage").is_none());
    }

    #[test]
    fn scholars_mate_is_detected_as_checkmate() {
        let mut game = Game::default();
        for uci in ["e2e4", "e7e5", "f1c4", "b8c6", "d1h5", "g8f6", "h5f7"] {
            let mv = game
                .parse_uci(uci)
                .unwrap_or_else(|| panic!("{uci} should be legal"));
            game.play(&mv);
        }
        assert_eq!(
            game.ended,
            Some(GameEnd::Checkmate {
                winner: Color::White
            })
        );
    }

    #[test]
    fn legal_moves_from_a_square_all_start_there() {
        let game = Game::default();
        let from = "e2".parse::<Square>().unwrap();
        let moves = game.legal_moves_from(from);
        assert_eq!(moves.len(), 2); // e3 and e4
        assert!(moves.iter().all(|m| m.from() == Some(from)));
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    fn game_with(ucis: &[&str]) -> Game {
        let mut game = Game::default();
        for uci in ucis {
            let mv = game.parse_uci(uci).unwrap();
            game.play(&mv);
        }
        game
    }

    #[test]
    fn a_fresh_review_follows_the_live_game() {
        let game = game_with(&["e2e4", "e7e5"]);
        let review = Review::default();
        assert!(!review.is_reviewing());
        assert_eq!(review.shown_ply(&game), 2);
    }

    #[test]
    fn stepping_back_shows_the_previous_position() {
        let game = game_with(&["e2e4", "e7e5"]);
        let mut review = Review::default();
        review.step_back(&game);
        assert_eq!(review.ply, Some(1));
        assert!(review.is_reviewing());

        // After one ply only White's pawn has moved.
        let pos = game.position_at(1);
        assert!(pos.board().piece_at("e4".parse().unwrap()).is_some());
        assert!(pos.board().piece_at("e5".parse().unwrap()).is_none());
    }

    #[test]
    fn stepping_back_stops_at_the_start() {
        let game = game_with(&["e2e4", "e7e5"]);
        let mut review = Review::default();
        for _ in 0..10 {
            review.step_back(&game);
        }
        assert_eq!(review.ply, Some(0));
        // The opening position has every piece present.
        assert_eq!(game.position_at(0).board().occupied().count(), 32);
    }

    #[test]
    fn stepping_forward_onto_the_newest_move_returns_to_live() {
        let game = game_with(&["e2e4", "e7e5"]);
        let mut review = Review::default();
        review.go_to_start(&game);
        assert_eq!(review.ply, Some(0));
        review.step_forward(&game);
        assert_eq!(review.ply, Some(1));
        review.step_forward(&game);
        assert!(!review.is_reviewing(), "should be back on the live game");
    }

    #[test]
    fn going_live_from_anywhere_shows_the_current_position() {
        let game = game_with(&["e2e4", "e7e5", "g1f3"]);
        let mut review = Review::default();
        review.go_to_start(&game);
        review.go_live();
        assert_eq!(review.shown_ply(&game), 3);
        assert_eq!(
            game.position_at(review.shown_ply(&game))
                .board()
                .occupied()
                .count(),
            game.position.board().occupied().count()
        );
    }

    #[test]
    fn reviewing_does_not_disturb_the_live_position() {
        let game = game_with(&["e2e4", "e7e5"]);
        let before = game.fen();
        let mut review = Review::default();
        review.go_to_start(&game);
        // Replaying an earlier position must not touch the game itself.
        let _ = game.position_at(0);
        assert_eq!(game.fen(), before);
    }

    #[test]
    fn replaying_every_ply_matches_the_live_position_at_the_end() {
        let game = game_with(&["e2e4", "e7e5", "g1f3", "b8c6", "f1b5"]);
        assert_eq!(
            Fen::from_position(&game.position_at(game.history.len()), EnPassantMode::Legal)
                .to_string(),
            game.fen()
        );
    }

    #[test]
    fn white_moves_on_even_plies() {
        assert_eq!(Game::mover_at(0), Color::White);
        assert_eq!(Game::mover_at(1), Color::Black);
        assert_eq!(Game::mover_at(2), Color::White);
    }

    #[test]
    fn an_empty_game_cannot_be_reviewed_backwards() {
        let game = Game::default();
        let mut review = Review::default();
        review.step_back(&game);
        assert_eq!(review.ply, None, "nothing to step back to");
        review.go_to_start(&game);
        assert_eq!(review.ply, None);
    }
}

#[cfg(test)]
mod material_tests {
    use super::*;

    fn played(ucis: &[&str]) -> Game {
        let mut game = Game::default();
        for uci in ucis {
            let mv = game
                .parse_uci(uci)
                .unwrap_or_else(|| panic!("{uci} illegal"));
            game.play(&mv);
        }
        game
    }

    fn balance(game: &Game) -> i32 {
        game.material_balance_at(game.history.len())
    }

    #[test]
    fn the_opening_position_is_level() {
        let game = Game::default();
        assert_eq!(balance(&game), 0);
        let losses = game.losses_at(0);
        assert!(losses.white.is_empty() && losses.black.is_empty());
    }

    #[test]
    fn taking_a_pawn_puts_you_one_ahead() {
        let game = played(&["e2e4", "d7d5", "e4d5"]);
        assert_eq!(balance(&game), 1, "White is a pawn up");
        let losses = game.losses_at(game.history.len());
        assert_eq!(losses.black, vec![Role::Pawn], "Black lost the pawn");
        assert!(losses.white.is_empty());
    }

    #[test]
    fn trading_evenly_returns_to_level() {
        let game = played(&["e2e4", "d7d5", "e4d5", "d8d5"]);
        assert_eq!(balance(&game), 0, "a pawn each");
        let losses = game.losses_at(game.history.len());
        assert_eq!(losses.white, vec![Role::Pawn]);
        assert_eq!(losses.black, vec![Role::Pawn]);
    }

    #[test]
    fn losing_a_queen_for_a_pawn_shows_as_eight_behind() {
        let game = played(&["e2e4", "d7d5", "e4d5", "d8d5", "b1c3", "d5d8"]);
        // Even so far; now White drops the queen.
        let game = {
            let mut g = game;
            for uci in ["d1h5", "g8f6", "h5f7", "e8f7"] {
                let mv = g.parse_uci(uci).unwrap();
                g.play(&mv);
            }
            g
        };
        let losses = game.losses_at(game.history.len());
        assert!(losses.white.contains(&Role::Queen), "White lost its queen");
        assert!(
            balance(&game) < 0,
            "Black should be ahead, got {}",
            balance(&game)
        );
    }

    #[test]
    fn a_promotion_counts_as_the_new_piece_not_the_pawn() {
        // Comparing against the starting line-up would report a phantom
        // captured pawn and a negative queen count.
        let mut game = Game::default();
        game.position = "8/1P6/8/8/8/8/8/K6k w - - 0 1"
            .parse::<Fen>()
            .unwrap()
            .into_position(CastlingMode::Standard)
            .unwrap();
        let before = game.material_balance_at(game.history.len());
        let promote = game.parse_uci("b7b8q").expect("promotion should be legal");
        game.play(&promote);
        let after = game.material_balance_at(game.history.len());
        assert_eq!(after - before, 8, "a pawn worth 1 became a queen worth 9");
        assert!(
            game.losses_at(game.history.len()).white.is_empty(),
            "promoting must not look like losing a pawn"
        );
    }

    #[test]
    fn losses_are_listed_cheapest_first() {
        let game = played(&[
            "e2e4", "d7d5", "g1f3", "d5e4", "f3g5", "e4e3", "g5f7", "e3d2",
        ]);
        let losses = game.losses_at(game.history.len());
        let values: Vec<i32> = losses.white.iter().map(|r| piece_value(*r)).collect();
        let mut sorted = values.clone();
        sorted.sort();
        assert_eq!(values, sorted, "pile should run cheapest first");
    }

    #[test]
    fn reviewing_shows_the_material_as_it_stood_then() {
        let game = played(&["e2e4", "d7d5", "e4d5"]);
        assert_eq!(game.material_balance_at(2), 0, "level before the capture");
        assert_eq!(game.material_balance_at(3), 1, "a pawn up after it");
        assert!(game.losses_at(2).black.is_empty());
        assert_eq!(game.losses_at(3).black, vec![Role::Pawn]);
    }
}
