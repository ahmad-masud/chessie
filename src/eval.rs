//! How the current position is going, for the bar down the side of the screen.
//!
//! The opponent engine is deliberately weakened, so its opinion is worth as
//! little as its play. The evaluation therefore comes from a second Stockfish
//! running at full strength, which never plays a move.

use bevy::prelude::*;
use shakmaty::{Color, Position};

use crate::engine::{Engine, Score};

/// A second engine, at full strength, used only to judge positions.
#[derive(Resource)]
pub struct Analyser(pub Engine);

/// The latest read on the position being shown.
#[derive(Resource, Default)]
pub struct Evaluation {
    /// Always from White's point of view, whoever is to move.
    pub score: Option<Score>,
    /// Which ply this describes, so a stale answer can be ignored.
    pub ply: Option<usize>,
    /// The position we are waiting to hear about.
    pub pending: Option<usize>,
    /// The position we want judged. Kept separate from `pending` so a request
    /// that arrives while the analyser is busy is not simply dropped.
    pub wanted: Option<usize>,
}

/// UCI reports from the side to move's point of view. Flip it so the bar does
/// not swap ends every move.
pub fn to_white_perspective(score: Score, side_to_move: Color) -> Score {
    match (score, side_to_move) {
        (s, Color::White) => s,
        (Score::Centipawns(cp), Color::Black) => Score::Centipawns(-cp),
        (Score::Mate(n), Color::Black) => Score::Mate(-n),
    }
}

/// Lichess's conversion from centipawns to a winning chance. A pawn up is
/// worth much more in a simple endgame than the raw number suggests, and this
/// curve is the usual way of expressing that.
fn chance_from_centipawns(cp: i32) -> f32 {
    let cp = cp.clamp(-2000, 2000) as f32;
    1.0 / (1.0 + (-0.00368208 * cp).exp())
}

impl Evaluation {
    /// White's chance of winning, from 0 (lost) to 1 (won). Half is level.
    pub fn white_chance(&self) -> f32 {
        match self.score {
            None => 0.5,
            Some(Score::Centipawns(cp)) => chance_from_centipawns(cp),
            // A forced mate is not a probability, it is a result.
            Some(Score::Mate(n)) if n > 0 => 1.0,
            Some(Score::Mate(n)) if n < 0 => 0.0,
            // Mate in zero: the game is already over.
            Some(Score::Mate(_)) => 0.5,
        }
    }

    /// Short label for the bar, in the way chess writes it.
    pub fn label(&self) -> String {
        match self.score {
            None => "…".to_string(),
            Some(Score::Centipawns(cp)) => {
                let pawns = cp as f32 / 100.0;
                if pawns >= 0.0 {
                    format!("+{pawns:.1}")
                } else {
                    format!("{pawns:.1}")
                }
            }
            Some(Score::Mate(n)) if n > 0 => format!("M{n}"),
            Some(Score::Mate(n)) if n < 0 => format!("-M{}", -n),
            Some(Score::Mate(_)) => "#".to_string(),
        }
    }
}

/// Asks the analyser about whatever position is on screen, including while
/// reviewing, so stepping back shows how things stood then.
pub fn request_analysis(
    game: Res<crate::chess::Game>,
    review: Res<crate::chess::Review>,
    mut analyser: ResMut<Analyser>,
    mut evaluation: ResMut<Evaluation>,
) {
    // Note what wants judging whenever the position on screen changes.
    if game.is_changed() || review.is_changed() {
        evaluation.wanted = Some(review.shown_ply(&game));
    }

    let Some(wanted) = evaluation.wanted else {
        return;
    };
    if !analyser.0.available {
        return;
    }
    // Busy with the previous position: hold the request rather than dropping
    // it, or a move made while it was thinking would leave the bar stale.
    if analyser.0.thinking || evaluation.pending.is_some() {
        return;
    }
    if evaluation.ply == Some(wanted) {
        evaluation.wanted = None;
        return;
    }

    let position = game.position_at(wanted);
    let fen =
        shakmaty::fen::Fen::from_position(&position, shakmaty::EnPassantMode::Legal).to_string();
    evaluation.pending = Some(wanted);
    evaluation.wanted = None;
    // Short enough to keep up with play, long enough to be worth reading.
    analyser.0.search(fen, 250);
}

/// Takes the answer and turns it into White's point of view.
pub fn receive_analysis(
    game: Res<crate::chess::Game>,
    mut analyser: ResMut<Analyser>,
    mut evaluation: ResMut<Evaluation>,
) {
    let Some(reply) = analyser.0.poll() else {
        return;
    };
    match reply {
        crate::engine::Reply::BestMove { score, .. } => {
            let ply = evaluation.pending.take();
            let Some(ply) = ply else {
                return;
            };
            let side_to_move = game.position_at(ply).turn();
            evaluation.score = score.map(|s| to_white_perspective(s, side_to_move));
            evaluation.ply = Some(ply);
        }
        crate::engine::Reply::Failed(err) => {
            warn!("analysis failed: {err}");
            evaluation.pending = None;
            analyser.0.available = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{parse_score, Score};

    fn eval(score: Score) -> Evaluation {
        Evaluation {
            score: Some(score),
            ply: Some(0),
            pending: None,
            wanted: None,
        }
    }

    #[test]
    fn a_score_is_pulled_out_of_a_real_info_line() {
        let line = "info depth 20 seldepth 28 multipv 1 score cp 35 nodes 1 pv e2e4";
        assert_eq!(parse_score(line), Some(Score::Centipawns(35)));
        let line = "info depth 12 score mate 3 nodes 9 pv f7f8";
        assert_eq!(parse_score(line), Some(Score::Mate(3)));
        let line = "info depth 12 score cp -931 nodes 9";
        assert_eq!(parse_score(line), Some(Score::Centipawns(-931)));
    }

    #[test]
    fn lines_without_a_score_are_ignored() {
        assert_eq!(parse_score("info depth 1 nodes 0 nps 0"), None);
        assert_eq!(parse_score("bestmove e2e4 ponder e7e5"), None);
        assert_eq!(parse_score(""), None);
    }

    #[test]
    fn a_score_is_flipped_when_black_is_to_move() {
        // Stockfish reports from the side to move. White a queen up reads
        // +934 with White to move but -900 with Black to move; both describe
        // the same winning position for White.
        let white_to_move = to_white_perspective(Score::Centipawns(934), Color::White);
        let black_to_move = to_white_perspective(Score::Centipawns(-900), Color::Black);
        assert_eq!(white_to_move, Score::Centipawns(934));
        assert_eq!(black_to_move, Score::Centipawns(900));
        assert!(
            eval(white_to_move).white_chance() > 0.5 && eval(black_to_move).white_chance() > 0.5,
            "both should show White winning"
        );
    }

    #[test]
    fn mate_is_flipped_too() {
        assert_eq!(
            to_white_perspective(Score::Mate(3), Color::Black),
            Score::Mate(-3)
        );
        assert_eq!(
            to_white_perspective(Score::Mate(3), Color::White),
            Score::Mate(3)
        );
    }

    #[test]
    fn a_level_position_splits_the_bar_in_half() {
        assert!((eval(Score::Centipawns(0)).white_chance() - 0.5).abs() < 1e-6);
        assert!((Evaluation::default().white_chance() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_bar_moves_the_right_way_and_never_leaves_the_screen() {
        let mut previous = 0.0;
        for cp in [-3000, -900, -300, -100, 0, 100, 300, 900, 3000] {
            let chance = eval(Score::Centipawns(cp)).white_chance();
            assert!(
                chance >= previous,
                "chance fell from {previous} to {chance} as White improved"
            );
            assert!((0.0..=1.0).contains(&chance), "{chance} is off the bar");
            previous = chance;
        }
    }

    #[test]
    fn a_forced_mate_fills_the_bar() {
        assert_eq!(eval(Score::Mate(1)).white_chance(), 1.0);
        assert_eq!(eval(Score::Mate(-1)).white_chance(), 0.0);
    }

    #[test]
    fn the_label_reads_the_way_chess_writes_it() {
        // 0.35 is not exactly representable and rounds down.
        assert_eq!(eval(Score::Centipawns(35)).label(), "+0.3");
        assert_eq!(eval(Score::Centipawns(120)).label(), "+1.2");
        assert_eq!(eval(Score::Centipawns(-250)).label(), "-2.5");
        assert_eq!(eval(Score::Mate(3)).label(), "M3");
        assert_eq!(eval(Score::Mate(-2)).label(), "-M2");
        assert_eq!(Evaluation::default().label(), "…");
    }

    #[test]
    fn a_pawn_matters_much_less_than_a_queen() {
        let pawn = eval(Score::Centipawns(100)).white_chance();
        let queen = eval(Score::Centipawns(900)).white_chance();
        assert!(
            pawn > 0.5 && pawn < 0.65,
            "a pawn up should be a nudge, got {pawn}"
        );
        assert!(queen > 0.95, "a queen up should be near-won, got {queen}");
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    use crate::engine::Score;

    /// Mirrors the guard in `request_analysis`, so the sequencing can be
    /// checked without a live engine.
    fn should_ask(evaluation: &Evaluation, thinking: bool) -> bool {
        match evaluation.wanted {
            None => false,
            Some(wanted) => {
                !thinking && evaluation.pending.is_none() && evaluation.ply != Some(wanted)
            }
        }
    }

    #[test]
    fn a_request_made_while_busy_is_held_not_dropped() {
        // The bug this guards: a move played while the analyser was still
        // thinking used to be discarded, leaving the bar showing the previous
        // position for the rest of the game.
        let mut evaluation = Evaluation {
            wanted: Some(4),
            ..Default::default()
        };
        assert!(!should_ask(&evaluation, true), "must not ask while busy");
        assert_eq!(evaluation.wanted, Some(4), "the request must survive");

        // Once free, it goes out.
        assert!(should_ask(&evaluation, false));
        evaluation.pending = evaluation.wanted.take();
        assert_eq!(evaluation.pending, Some(4));
    }

    #[test]
    fn a_position_already_judged_is_not_asked_about_again() {
        let evaluation = Evaluation {
            wanted: Some(7),
            ply: Some(7),
            score: Some(Score::Centipawns(20)),
            pending: None,
        };
        assert!(!should_ask(&evaluation, false));
    }

    #[test]
    fn nothing_is_asked_when_nothing_is_wanted() {
        assert!(!should_ask(&Evaluation::default(), false));
    }

    #[test]
    fn only_one_question_is_in_flight_at_a_time() {
        let evaluation = Evaluation {
            wanted: Some(9),
            pending: Some(8),
            ..Default::default()
        };
        assert!(!should_ask(&evaluation, false), "already waiting on ply 8");
    }
}
