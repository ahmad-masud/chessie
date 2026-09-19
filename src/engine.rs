//! Stockfish driver: owns the engine subprocess and talks UCI to it on a
//! background thread, so the render loop never blocks on a search.

use bevy::prelude::*;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Mutex;
use std::thread;

/// Stockfish refuses to be weaker than this via `UCI_Elo`; below it we add
/// our own blunders on top of a 1320 engine.
pub const ENGINE_MIN_ELO: u32 = 1320;
pub const ENGINE_MAX_ELO: u32 = 3190;
/// Weakest rating the UI offers, reached purely through blunder injection.
pub const UI_MIN_ELO: u32 = 250;

/// Requests sent to the engine thread.
enum Request {
    NewGame,
    SetElo(u32),
    Search { fen: String, movetime_ms: u64 },
    Quit,
}

/// How good a position looks, from the point of view of the side to move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Score {
    /// Hundredths of a pawn.
    Centipawns(i32),
    /// Mate in this many moves. Negative means the side to move is being mated.
    Mate(i32),
}

/// Pulls the score out of a UCI `info` line, if it carries one.
pub fn parse_score(line: &str) -> Option<Score> {
    let mut tokens = line.split_whitespace();
    while let Some(token) = tokens.next() {
        if token != "score" {
            continue;
        }
        return match tokens.next()? {
            "cp" => tokens.next()?.parse().ok().map(Score::Centipawns),
            "mate" => tokens.next()?.parse().ok().map(Score::Mate),
            _ => None,
        };
    }
    None
}

/// Replies coming back from the engine thread.
pub enum Reply {
    BestMove {
        uci: String,
        /// The engine's own read on the position it just searched.
        score: Option<Score>,
    },
    Failed(String),
}

#[derive(Resource)]
pub struct Engine {
    tx: Sender<Request>,
    /// `Receiver` is `Send` but not `Sync`, and a Bevy resource must be both.
    rx: Mutex<Receiver<Reply>>,
    /// True between issuing a search and receiving its result.
    pub thinking: bool,
    pub available: bool,
    pub last_error: Option<String>,
}

impl Engine {
    /// Locate and launch Stockfish. Falls back across common install paths.
    pub fn launch(limit_strength: bool) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (rep_tx, rep_rx) = mpsc::channel::<Reply>();

        match spawn_stockfish() {
            Ok(child) => {
                thread::spawn(move || engine_thread(child, req_rx, rep_tx, limit_strength));
                Self {
                    tx: req_tx,
                    rx: Mutex::new(rep_rx),
                    thinking: false,
                    available: true,
                    last_error: None,
                }
            }
            Err(e) => {
                error!("could not start Stockfish: {e}");
                Self {
                    tx: req_tx,
                    rx: Mutex::new(rep_rx),
                    thinking: false,
                    available: false,
                    last_error: Some(e),
                }
            }
        }
    }

    pub fn new_game(&self) {
        let _ = self.tx.send(Request::NewGame);
    }

    pub fn set_elo(&self, elo: u32) {
        let _ = self
            .tx
            .send(Request::SetElo(elo.clamp(ENGINE_MIN_ELO, ENGINE_MAX_ELO)));
    }

    pub fn search(&mut self, fen: String, movetime_ms: u64) {
        if !self.available {
            return;
        }
        self.thinking = true;
        let _ = self.tx.send(Request::Search { fen, movetime_ms });
    }

    /// Non-blocking poll, called every frame.
    pub fn poll(&mut self) -> Option<Reply> {
        let received = match self.rx.lock() {
            Ok(rx) => rx.try_recv(),
            Err(_) => return None,
        };
        match received {
            Ok(reply) => {
                self.thinking = false;
                Some(reply)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                if self.available {
                    self.available = false;
                    self.thinking = false;
                    self.last_error = Some("engine process exited".into());
                }
                None
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Quit);
    }
}

fn spawn_stockfish() -> Result<Child, String> {
    // The engine shipped inside the app comes first, so a packaged copy never
    // depends on what happens to be installed.
    let exe = std::env::current_exe().unwrap_or_default();
    let candidates = crate::paths::engine_candidates(&exe);
    let mut last = String::new();
    for path in &candidates {
        match Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                info!("started Stockfish from {path}");
                return Ok(child);
            }
            Err(e) => last = format!("{path}: {e}"),
        }
    }
    Err(format!(
        "no chess engine found (tried {} locations). Last error: {last}",
        candidates.len()
    ))
}

/// Owns the subprocess for its lifetime, translating requests into UCI.
fn engine_thread(mut child: Child, rx: Receiver<Request>, tx: Sender<Reply>, limit_strength: bool) {
    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => return,
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return,
    };
    let mut reader = BufReader::new(stdout);

    // Handshake.
    if send(&mut stdin, "uci").is_err() {
        let _ = tx.send(Reply::Failed("engine closed during handshake".into()));
        return;
    }
    let _ = wait_for(&mut reader, "uciok");
    let _ = send(
        &mut stdin,
        &format!("setoption name UCI_LimitStrength value {limit_strength}"),
    );
    let _ = send(&mut stdin, "isready");
    let _ = wait_for(&mut reader, "readyok");

    while let Ok(req) = rx.recv() {
        match req {
            Request::Quit => break,
            Request::NewGame => {
                let _ = send(&mut stdin, "ucinewgame");
                let _ = send(&mut stdin, "isready");
                let _ = wait_for(&mut reader, "readyok");
            }
            Request::SetElo(elo) => {
                let _ = send(&mut stdin, "setoption name UCI_LimitStrength value true");
                let _ = send(&mut stdin, &format!("setoption name UCI_Elo value {elo}"));
                let _ = send(&mut stdin, "isready");
                let _ = wait_for(&mut reader, "readyok");
            }
            Request::Search { fen, movetime_ms } => {
                if send(&mut stdin, &format!("position fen {fen}")).is_err()
                    || send(&mut stdin, &format!("go movetime {movetime_ms}")).is_err()
                {
                    let _ = tx.send(Reply::Failed("engine stopped accepting input".into()));
                    break;
                }
                match read_bestmove(&mut reader) {
                    Some((uci, score)) => {
                        if tx.send(Reply::BestMove { uci, score }).is_err() {
                            break;
                        }
                    }
                    None => {
                        let _ = tx.send(Reply::Failed("engine produced no move".into()));
                        break;
                    }
                }
            }
        }
    }

    let _ = send(&mut stdin, "quit");
    let _ = child.wait();
}

fn send(stdin: &mut ChildStdin, line: &str) -> std::io::Result<()> {
    stdin.write_all(line.as_bytes())?;
    stdin.write_all(b"\n")?;
    stdin.flush()
}

fn wait_for(reader: &mut BufReader<std::process::ChildStdout>, token: &str) -> Option<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line.trim_start().starts_with(token) {
            return Some(());
        }
    }
}

fn read_bestmove(
    reader: &mut BufReader<std::process::ChildStdout>,
) -> Option<(String, Option<Score>)> {
    let mut line = String::new();
    // The search reports as it goes; the last score before `bestmove` is the
    // engine's final word on the position.
    let mut score = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim();
        if line.starts_with("info") {
            if let Some(found) = parse_score(line) {
                score = Some(found);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("bestmove ") {
            let mv = rest.split_whitespace().next()?;
            if mv == "(none)" {
                return None;
            }
            return Some((mv.to_string(), score));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Drives the engine synchronously, the way the Bevy systems do frame to frame.
    fn await_reply(engine: &mut Engine) -> Reply {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(reply) = engine.poll() {
                return reply;
            }
            assert!(Instant::now() < deadline, "engine did not reply in time");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn engine_or_skip() -> Option<Engine> {
        let engine = Engine::launch(true);
        if !engine.available {
            eprintln!("skipping: Stockfish not installed");
            return None;
        }
        Some(engine)
    }

    #[test]
    fn engine_returns_a_legal_opening_move() {
        let Some(mut engine) = engine_or_skip() else {
            return;
        };
        engine.new_game();
        engine.set_elo(1500);

        let mut game = crate::chess::Game::default();
        engine.search(game.fen(), 200);
        assert!(engine.thinking);

        match await_reply(&mut engine) {
            Reply::BestMove { uci, .. } => {
                let mv = game.parse_uci(&uci).expect("engine move should be legal");
                game.play(&mv);
                assert_eq!(game.history.len(), 1);
            }
            Reply::Failed(e) => panic!("engine failed: {e}"),
        }
        assert!(!engine.thinking, "thinking flag should clear on reply");
    }

    #[test]
    fn engine_answers_a_human_move_and_play_alternates() {
        let Some(mut engine) = engine_or_skip() else {
            return;
        };
        engine.new_game();
        engine.set_elo(ENGINE_MIN_ELO);

        let mut game = crate::chess::Game::default();
        // Human opens; the engine must reply as Black.
        let e4 = game.parse_uci("e2e4").unwrap();
        game.play(&e4);

        let mut expected = game.history.len();
        for round in 0..3 {
            engine.search(game.fen(), 100);
            match await_reply(&mut engine) {
                Reply::BestMove { uci, .. } => {
                    let mv = game
                        .parse_uci(&uci)
                        .unwrap_or_else(|| panic!("illegal move {uci} in round {round}"));
                    game.play(&mv);
                }
                Reply::Failed(e) => panic!("engine failed: {e}"),
            }
            expected += 1;
            assert_eq!(game.history.len(), expected);
            if game.ended.is_some() {
                break;
            }
            // Stand in for the human with any legal reply.
            let reply = game
                .legal_moves()
                .into_iter()
                .next()
                .expect("game continues");
            game.play(&reply);
            expected += 1;
            assert_eq!(game.history.len(), expected);
        }
    }

    #[test]
    fn respects_the_documented_elo_bounds() {
        // These mirror what the Stockfish binary reports for UCI_Elo.
        assert_eq!(ENGINE_MIN_ELO, 1320);
        assert_eq!(ENGINE_MAX_ELO, 3190);
        assert!(UI_MIN_ELO < ENGINE_MIN_ELO);
    }
}
