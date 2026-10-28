//! État de session partagé entre l'UI et le serveur MCP.
//!
//! Motivation (bug constaté en test live le 28/10) : les snapshots MCP étaient
//! publiés depuis `eframe::App::update()`. Or eframe cesse de rappeler
//! `update()` quand la fenêtre est minimisée ou entièrement occluse : le
//! backend continuait de recevoir les octets, mais le parseur vt100 (et donc
//! `read_screen` / `await_state`) restait figé.
//!
//! Correctif : le parseur, l'état de connexion et le flux de sortie vivent
//! dans un [`LiveSession`] derrière un `Arc<Mutex<…>>`. Un thread « pompe »
//! (créé par [`Pump::spawn`]) draine le canal d'événements du backend, met à
//! jour cet état et publie les snapshots MCP — **sans jamais dépendre de
//! l'UI**. L'UI ne fait plus que lire l'état pour peindre la grille.
//!
//! Le verrou n'est jamais tenu longtemps : parsing d'un lot d'octets, copie
//! d'un écran de 80×24, ou parcours d'une ligne pour le rendu.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backend::{BackendCommand, BackendEvent};
use crate::config::Config;
use crate::mcp::{McpPublisher, SessionSnapshot, SessionState};
use crate::terminal;

/// Taille du tampon circulaire du flux interprété (`read_output_stream`).
pub const OUTPUT_TAIL_MAX: usize = 64 * 1024;

/// Fréquence maximale de publication des snapshots MCP (~50 Hz).
const PUBLISH_MIN_INTERVAL: Duration = Duration::from_millis(20);

/// Période d'attente du thread pompe quand aucun événement n'arrive.
const PUMP_TICK: Duration = Duration::from_millis(20);

/// État de la connexion, affiché en barre de statut et traduit en
/// `SessionState` pour le MCP.
#[derive(Clone, Debug)]
pub enum Status {
    Connecting,
    Connected,
    Closed,
    Failed(String),
}

/// Tout ce qu'une session expose à la fois à l'UI et au MCP.
pub struct LiveSession {
    pub term: terminal::Terminal,
    pub status: Status,
    /// Dernier titre OSC 0/2 reçu (vide = titre statique de la config).
    pub title: String,
    pub host: String,
    pub port: u16,
    pub proto: String,
    /// Id MCP de la session (chaîne uuid v4, FIXÉ 27/10).
    pub id: String,
    /// Titre statique de repli (`-T`, `%s`…).
    pub static_title: String,
    /// Total d'octets reçus depuis l'ouverture (curseur absolu).
    pub output_seq: u64,
    /// Fin du flux reçu (tampon borné à [`OUTPUT_TAIL_MAX`]).
    pub output_tail: VecDeque<u8>,
}

impl LiveSession {
    pub fn new(cfg: &Config, id: String) -> Self {
        let (proto, port) = cfg.resolved();
        LiveSession {
            term: terminal::Terminal::new(cfg),
            status: Status::Connecting,
            title: String::new(),
            host: cfg.host.clone(),
            port,
            proto: proto.as_str().to_string(),
            id,
            static_title: cfg.static_title(),
            output_seq: 0,
            output_tail: VecDeque::new(),
        }
    }

    /// Réinitialise pour une nouvelle connexion (même `Arc`, nouveau backend).
    pub fn reset(&mut self, cfg: &Config, id: String) {
        let (proto, port) = cfg.resolved();
        self.term = terminal::Terminal::new(cfg);
        self.status = Status::Connecting;
        self.title.clear();
        self.host = cfg.host.clone();
        self.port = port;
        self.proto = proto.as_str().to_string();
        self.id = id;
        self.static_title = cfg.static_title();
        self.output_seq = 0;
        self.output_tail.clear();
    }

    /// Injecte un lot d'octets du backend (flux serveur → écran).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.term.feed(bytes);
        if let Some(t) = self.term.take_title() {
            if !t.is_empty() {
                self.title = t;
            }
        }
        self.record_output(bytes);
    }

    /// Enregistre le flux interprété (tampon 64 Ki + séquence absolue) pour
    /// `read_output_stream` — façon « copy all » [REA §7].
    pub fn record_output(&mut self, bytes: &[u8]) {
        self.output_seq += bytes.len() as u64;
        self.output_tail.extend(bytes);
        while self.output_tail.len() > OUTPUT_TAIL_MAX {
            self.output_tail.pop_front();
        }
    }

    /// Traduit l'état interne en `SessionState` d'après les prompts RÉELS
    /// observés dans putty.exe (docs/REA_EVIDENCE.md §5 : `login as: `,
    /// `%s@%s's password: `, `Proxy password: `…).
    pub fn detect_state(&self, lines: &[String]) -> SessionState {
        match &self.status {
            Status::Failed(_) => return SessionState::Failed,
            Status::Closed => return SessionState::Closed,
            _ => {}
        }
        let tail = lines
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim_end().to_ascii_lowercase())
            .unwrap_or_default();
        if tail.ends_with("login as:") {
            SessionState::LoginPrompt
        } else if tail.ends_with("password:") || tail.ends_with("'s password:") {
            SessionState::PasswordPrompt
        } else if tail.contains("passphrase")
            || tail.contains("(y/n)")
            || tail.contains("continue connecting")
        {
            SessionState::AuthOther
        } else if matches!(self.status, Status::Connected) {
            SessionState::Connected
        } else {
            SessionState::Connecting
        }
    }

    /// Fabrique l'unique snapshot de session (mono-fenêtre v0.1).
    ///
    /// Le snapshot reflète **toujours la vue live** (bas du journal), même si
    /// l'utilisateur a remonté l'historique dans la fenêtre : un agent MCP ne
    /// doit pas hériter de la position de défilement humaine (sinon
    /// `read_screen`/`await_state` semblent figés). La vue de l'UI est
    /// restaurée avant de rendre la main.
    pub fn snapshot(&mut self) -> SessionSnapshot {
        let saved_view = self.term.view_offset();
        if saved_view != 0 {
            self.term.set_view_offset(0);
        }
        let snap = self.snapshot_live();
        if saved_view != 0 {
            self.term.set_view_offset(saved_view);
        }
        snap
    }

    fn snapshot_live(&self) -> SessionSnapshot {
        let contents = self.term.screen().contents();
        let mut lines: Vec<String> = contents
            .split('\n')
            .map(|l| l.trim_end_matches(' ').to_string())
            .collect();
        while (lines.len() as u16) < self.term.rows() {
            lines.push(String::new());
        }
        let (cr, cc) = self.term.screen().cursor_position();
        let mut s =
            SessionSnapshot::starting(&self.id, &self.host, self.port, &self.proto);
        s.title = if self.title.is_empty() {
            self.static_title.clone()
        } else {
            self.title.clone()
        };
        s.state = self.detect_state(&lines);
        s.cols = self.term.cols();
        s.rows = self.term.rows();
        s.cursor = (cc, cr);
        s.lines = lines;
        // TODO v0.2 : vrai journal (vt100 0.18 n'expose que `scrollback_len` ;
        // `contents()` ne rend que les lignes visibles) — 0 = honnête.
        s.scrollback_len = 0;
        s.error = match &self.status {
            Status::Failed(e) => Some(e.clone()),
            _ => None,
        };
        s.output_seq = self.output_seq;
        s.output_tail = self.output_tail.iter().copied().collect();
        s
    }
}

fn lock_live(live: &Arc<Mutex<LiveSession>>) -> MutexGuard<'_, LiveSession> {
    live.lock().unwrap_or_else(|p| p.into_inner())
}

/// Thread « pompe » : backend → [`LiveSession`] → snapshots MCP.
pub struct Pump {
    alive: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Pump {
    /// Démarre la pompe. Le premier snapshot (« starting ») est publié
    /// immédiatement, avant même le premier octet du serveur.
    pub fn spawn(
        live: Arc<Mutex<LiveSession>>,
        events: Receiver<BackendEvent>,
        backend: Sender<BackendCommand>,
        publisher: Option<McpPublisher>,
    ) -> Pump {
        let alive = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&alive);
        let join = std::thread::Builder::new()
            .name("putty-rs-session-pump".into())
            .spawn(move || run(live, events, backend, publisher, flag))
            .ok();
        Pump { alive, join }
    }

    /// Arrête la pompe sans bloquer l'UI (le thread sort au plus tard après
    /// un tick ; son canal est déjà fermé par `backend.shutdown()`).
    pub fn stop(&mut self) {
        self.alive.store(false, Ordering::Release);
        // Détache : ne jamais joindre un thread qui pourrait attendre le
        // backend (le thread mourra de lui-même, son canal se ferme).
        let _ = self.join.take();
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

fn run(
    live: Arc<Mutex<LiveSession>>,
    events: Receiver<BackendEvent>,
    backend: Sender<BackendCommand>,
    publisher: Option<McpPublisher>,
    alive: Arc<AtomicBool>,
) {
    let mut last_publish: Option<Instant> = None;
    let mut dirty = true; // publie l'état initial (« starting »)
    log::info!("pompe: demarrage");

    loop {
        if !alive.load(Ordering::Acquire) {
            log::info!("pompe: arret demandee (alive=false)");
            break;
        }
        // Publication à jour, hors de tout repaint de l'UI.
        if dirty {
            let due = last_publish.map_or(true, |t| t.elapsed() >= PUBLISH_MIN_INTERVAL);
            if due {
                if let Some(p) = &publisher {
                    let (snap, seq, last) = {
                        let mut l = lock_live(&live);
                        let s = l.snapshot();
                        let last = s
                            .lines
                            .iter()
                            .rev()
                            .find(|x| !x.trim().is_empty())
                            .cloned()
                            .unwrap_or_default();
                        (vec![s], l.output_seq, last)
                    };
                    log::debug!("pompe: publish seq={seq} last={last:?}");
                    p.publish(snap, true);
                }
                last_publish = Some(Instant::now());
                dirty = false;
            }
        }

        match events.recv_timeout(PUMP_TICK) {
            Ok(BackendEvent::Received(bytes)) => {
                let outbox = {
                    let mut l = lock_live(&live);
                    if matches!(l.status, Status::Connecting) {
                        l.status = Status::Connected;
                    }
                    l.feed(&bytes);
                    l.term.drain_outbox()
                };
                // Réponses du terminal (answerback suite à ENQ…).
                for b in outbox {
                    let _ = backend.send(BackendCommand::Write(b));
                }
                dirty = true;
            }
            Ok(BackendEvent::Closed) => {
                {
                    let mut l = lock_live(&live);
                    if !matches!(l.status, Status::Failed(_)) {
                        l.status = Status::Closed;
                    }
                }
                dirty = true;
                // Dernier snapshot pour que le MCP voie l'état final.
                if let Some(p) = &publisher {
                    let snap = {
                        let mut l = lock_live(&live);
                        vec![l.snapshot()]
                    };
                    p.publish(snap, true);
                }
                break;
            }
            Ok(BackendEvent::Error(e)) => {
                log::error!("backend : {e}");
                lock_live(&live).status = Status::Failed(e);
                dirty = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(p) = &publisher {
                    let snap = {
                        let mut l = lock_live(&live);
                        vec![l.snapshot()]
                    };
                    p.publish(snap, true);
                }
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{McpBus, SessionState};
    use std::sync::mpsc::channel;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn snapshot_reflete_le_flux_sans_ui() {
        // Régression 28/10 : les snapshots doivent avancer même si aucun
        // `update()` eframe n'est appelé (fenêtre minimisée).
        let (ui, server) = McpBus::new();
        let live = Arc::new(Mutex::new(LiveSession::new(&cfg(), "sess-1".into())));
        let (ev_tx, ev_rx) = channel();
        let (cmd_tx, cmd_rx) = channel();
        let pump = Pump::spawn(
            Arc::clone(&live),
            ev_rx,
            cmd_tx,
            Some(ui.publisher()),
        );

        // Aucun repaint : seule la pompe travaille.
        ev_tx.send(BackendEvent::Received(b"hello from pump\r\n".to_vec())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found = false;
        while Instant::now() < deadline {
            if let Some(s) = server.find_session("sess-1") {
                if s.lines.iter().any(|l| l.contains("hello from pump")) {
                    found = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(found, "le snapshot MCP n'a pas avance sans frame UI");
        let s = server.find_session("sess-1").unwrap();
        assert_eq!(s.state, SessionState::Connected);
        assert!(s.output_seq >= 17);

        drop(pump);
        drop(ev_tx);
        drop(cmd_rx);
    }

    #[test]
    fn closed_est_publie() {
        let (ui, server) = McpBus::new();
        let live = Arc::new(Mutex::new(LiveSession::new(&cfg(), "sess-2".into())));
        let (ev_tx, ev_rx) = channel();
        let (cmd_tx, _cmd_rx) = channel();
        let _pump = Pump::spawn(Arc::clone(&live), ev_rx, cmd_tx, Some(ui.publisher()));
        ev_tx.send(BackendEvent::Closed).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut closed = false;
        while Instant::now() < deadline {
            if let Some(s) = server.find_session("sess-2") {
                if s.state == SessionState::Closed {
                    closed = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(closed, "l'etat closed n'a pas ete publie");
    }

    #[test]
    fn snapshot_montre_toujours_la_vue_live() {
        // Un agent MCP ne doit pas heriter du defilement de l'utilisateur.
        let mut c = cfg();
        c.scrollback = 500;
        let mut l = LiveSession::new(&c, "s".into());
        for i in 0..80 {
            l.feed(format!("X{i}\r\n").as_bytes());
        }
        l.term.scroll_by(10);
        assert!(!l.term.at_bottom());
        let s = l.snapshot();
        assert!(s.lines.iter().any(|x| x.contains("X79")));
        // La position de l'utilisateur est restauree apres la photo.
        assert_eq!(l.term.view_offset(), 10);
    }

    #[test]
    fn reset_change_id_et_efface_ecran() {
        let mut l = LiveSession::new(&cfg(), "a".into());
        l.feed(b"texte\r\n");
        l.reset(&cfg(), "b".into());
        assert_eq!(l.id, "b");
        assert_eq!(l.output_seq, 0);
        assert!(l.snapshot().lines.iter().all(|s| s.trim().is_empty()));
    }
}
