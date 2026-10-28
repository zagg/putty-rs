//! Bus interne UI ⇄ serveur MCP (voir `docs/MCP_PILOT.md`).
//!
//! Ce module ne dépend **que de `std` + `serde`** (aucune dépendance GUI,
//! backend ou tokio) afin que `app.rs` le draine à chaque frame eframe sans
//! jamais attendre le serveur MCP, et inversement.
//!
//! Deux moitiés issues de [`McpBus::new`] :
//! - [`McpHandle`] : côté **UI** (thread eframe) — draine les commandes,
//!   publie les snapshots d'état (~10 Hz), répond aux requêtes de config.
//! - [`McpServerHalf`] : côté **serveur MCP** (thread tokio dédié, clonable,
//!   partagé entre les sessions MCP) — soumet les commandes, lit le monde.
//!
//! Sémantique volontairement non bloquante : `try_send` (file bornée à 64)
//! côté commandes, mutex jamais détenu plus que le temps d'un `clone()`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Capacité de la file de commandes MCP → UI. Au-delà, `submit` échoue
/// proprement (l'UI ne suit plus : on refuse au lieu de bloquer le MCP).
pub const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Temps maximal accordé à l'UI pour répondre à une requête get/set_config.
pub const CONFIG_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Commandes MCP → UI
// ---------------------------------------------------------------------------

/// Commande exécutée par le thread UI à la frame suivante (comme un
/// événement clavier normal — cf. `docs/MCP_PILOT.md` §« cycle de vie »).
#[derive(Debug, Clone, Deserialize)]
pub enum McpCommand {
    /// Injecte des octets UTF-8 dans le flux clavier, **sans CR automatique**
    /// (décision FIXÉ 27/10 : le client MCP envoie `"user\r"` lui-même).
    SendText { id: String, text: String },
    /// Touches nommées (`Enter`, `↑`, `Tab`, `Ctrl-C`…) → mêmes séquences que
    /// `terminal.rs` (DECARM / app-keys). L'UI fait le mapping nom→octets.
    SendKeys { id: String, keys: Vec<String> },
    /// Redimensionne (validé min 2×2 par le serveur MCP avant envoi).
    Resize { id: String, cols: u16, rows: u16 },
    /// Répond au dialogue clé d'hôte ; `once` = « Connect Once »
    /// (ne pas stocker dans `SshHostKeys`).
    AcceptHostKey { id: String, once: bool },
    /// Ouvre une nouvelle session ; l'UI génère l'id (uuid, chaîne).
    NewSession {
        host: String,
        port: u16,
        proto: String,
    },
    /// Ferme proprement une session (équivalent `exit(0x12)`).
    CloseSession { id: String },
    /// Bascule thème sombre/clair (pont THEME_DARK.md).
    SetTheme { dark: bool },
}

// ---------------------------------------------------------------------------
// Snapshots UI → MCP
// ---------------------------------------------------------------------------

/// État machine d'une session, mappé depuis les prompts réels observés
/// (`docs/REA_EVIDENCE.md` §5) : `login as: ` → `LoginPrompt` ;
/// `%s@%s's password: ` / `Password:` → `PasswordPrompt` ; dialogue clé
/// d'hôte → `HostkeyPending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Session déclarée mais pas encore initialisée (handshake MCP possible
    /// avant que l'UI soit prête → `status: "starting"`).
    Starting,
    Connecting,
    /// Invite de type `login as: ` (REA §5, @0x14011d796).
    LoginPrompt,
    /// Invite de type `%s@%s's password: ` / `Password:` (REA §5).
    PasswordPrompt,
    /// Autre challenge d'auth (clé privée, keyboard-interactive, TIS/KI…).
    AuthOther,
    /// Dialogue « The host key is not cached for this server: » (REA §2/§5).
    HostkeyPending,
    Connected,
    /// Échec ; `error` porte le format PuTTY
    /// `Unable to open connection to\n%s\n%s` (REA §2).
    Failed,
    Closed,
}

impl SessionState {
    /// Représentation filaire stable (identique au serde snake_case).
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionState::Starting => "starting",
            SessionState::Connecting => "connecting",
            SessionState::LoginPrompt => "login_prompt",
            SessionState::PasswordPrompt => "password_prompt",
            SessionState::AuthOther => "auth_other",
            SessionState::HostkeyPending => "hostkey_pending",
            SessionState::Connected => "connected",
            SessionState::Failed => "failed",
            SessionState::Closed => "closed",
        }
    }
}

/// Instantané d'une session, publié périodiquement (~10 Hz) par l'UI.
/// Les ids de session sont des **chaînes uuid** (FIXÉ 27/10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Identifiant opaque exposé aux outils MCP (chaîne uuid v4).
    pub id: String,
    /// Titre de fenêtre (WTITLE observé REA §1, OSC 0/2).
    pub title: String,
    pub state: SessionState,
    pub host: String,
    pub port: u16,
    /// `ssh` | `telnet` | `raw` | `serial`.
    pub proto: String,
    pub cols: u16,
    pub rows: u16,
    /// Position curseur (x = colonne, y = ligne, 0-based, grille visible).
    pub cursor: (u16, u16),
    /// Grille visible rendue (texte sans attributs ; `rows` lignes de
    /// `cols` caractères, padding espace). Pour `read_scrollback`, l'UI peut
    /// y préfixer les lignes de journal (cf. TODO app.rs).
    pub lines: Vec<String>,
    /// Nombre de lignes retenues au-delà de l'écran (plafonné par
    /// `ScrollbackLines` registre).
    pub scrollback_len: usize,
    /// Message d'erreur éventuel, format PuTTY (REA §2), mot de passe masqué
    /// (règle `SSHLogOmitPasswords`, RUNTIME §4).
    pub error: Option<String>,
    /// Séquence incrémentale du flux de sortie : nombre **total** d'octets
    /// émis par le terminal depuis l'ouverture de la session (façon
    /// « copy all », REA §7). Sert de curseur absolu à `read_output_stream`.
    pub output_seq: u64,
    /// Fin du flux interprété (tampon borné, côté UI ~64 Ko).
    pub output_tail: Vec<u8>,
}

impl SessionSnapshot {
    /// Snapshot minimal « démarrer et on verra » (utilisé par l'UI tant que
    /// le backend n'a rien publié).
    pub fn starting(id: impl Into<String>, host: impl Into<String>, port: u16, proto: impl Into<String>) -> Self {
        SessionSnapshot {
            id: id.into(),
            title: String::new(),
            state: SessionState::Starting,
            host: host.into(),
            port,
            proto: proto.into(),
            cols: 80,
            rows: 24,
            cursor: (0, 0),
            lines: Vec::new(),
            scrollback_len: 0,
            error: None,
            output_seq: 0,
            output_tail: Vec::new(),
        }
    }
}

/// Vue globale servie aux outils MCP (lecture seule côté MCP).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorldState {
    pub sessions: Vec<SessionSnapshot>,
}

// ---------------------------------------------------------------------------
// Requêtes config (canal additionnel, clé/valeur → réponse)
// ---------------------------------------------------------------------------

/// Opération de config déléguée à l'UI (`get_config`/`set_config` → clés
/// registre PuTTY §4 + clés putty-rs `AppTheme`, `McpPort`).
#[derive(Debug, Clone)]
pub enum ConfigOp {
    Get { key: String },
    Set { key: String, value: String },
}

/// Réponse de l'UI à une [`ConfigOp`] : `payload` = valeur (Get) ou message
/// d'erreur si `ok == false`.
#[derive(Debug, Clone)]
pub struct ConfigReply {
    pub ok: bool,
    pub payload: String,
}

/// Requête config aller-retour, acheminée par la seconde file du bus.
#[derive(Debug)]
pub struct ConfigRequest {
    pub op: ConfigOp,
    pub respond: SyncSender<ConfigReply>,
}

// ---------------------------------------------------------------------------
// Le bus
// ---------------------------------------------------------------------------

/// Fabrique les deux moitiés du bus. À appeler dans `main.rs` avant
/// `eframe::run_native`.
pub struct McpBus;

impl McpBus {
    /// Retourne `(côté UI, côté serveur MCP)`.
    pub fn new() -> (McpHandle, McpServerHalf) {
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<McpCommand>(COMMAND_QUEUE_CAPACITY);
        let (cfg_tx, cfg_rx) = mpsc::sync_channel::<ConfigRequest>(COMMAND_QUEUE_CAPACITY);
        let world = Arc::new(Mutex::new(WorldState::default()));
        let ui_ready = Arc::new(AtomicBool::new(false));

        let ui = McpHandle {
            cmd_rx,
            cfg_rx,
            world: Arc::clone(&world),
            ui_ready: Arc::clone(&ui_ready),
        };
        let server = McpServerHalf {
            cmd_tx,
            cfg_tx,
            world,
            ui_ready,
        };
        (ui, server)
    }
}

/// Moitié UI — vit dans `App` (putty-rs), drainée à chaque frame.
/// Non `Sync` (mpsc::Receiver) : n'utiliser que depuis le thread eframe.
pub struct McpHandle {
    cmd_rx: Receiver<McpCommand>,
    cfg_rx: Receiver<ConfigRequest>,
    world: Arc<Mutex<WorldState>>,
    ui_ready: Arc<AtomicBool>,
}

impl McpHandle {
    /// Raflle toutes les commandes en attente (non bloquant). À appeler en
    /// début de `eframe::App::update`.
    pub fn drain_commands(&self) -> Vec<McpCommand> {
        let mut out = Vec::new();
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            out.push(cmd);
            if out.len() >= 4 * COMMAND_QUEUE_CAPACITY {
                break; // garde-fou : 1 frame = 1 rafale
            }
        }
        out
    }

    /// Raflle les requêtes config en attente. Pour chacune, l'UI doit
    /// répondre via `req.respond` (sinon `ask_config` côté MCP timeoute).
    pub fn drain_config_requests(&self) -> Vec<ConfigRequest> {
        let mut out = Vec::new();
        while let Ok(req) = self.cfg_rx.try_recv() {
            out.push(req);
        }
        out
    }

    /// Publie l'état du monde (grilles, titres, états) + le drapeau
    /// « UI prête ». À appeler à ~10 Hz max (coalescing, cf. MCP_PILOT).
    pub fn publish(&self, sessions: Vec<SessionSnapshot>, ready: bool) {
        self.publisher().publish(sessions, ready);
    }

    /// Moitié « écriture » détachable, `Send + Sync` : le thread pompe de
    /// session publie les snapshots sans passer par le thread UI (correctif
    /// du gel des snapshots quand eframe ne redessine plus).
    pub fn publisher(&self) -> McpPublisher {
        McpPublisher {
            world: Arc::clone(&self.world),
            ui_ready: Arc::clone(&self.ui_ready),
        }
    }
}

/// Poignée de publication des snapshots, clonable et utilisable depuis
/// n'importe quel thread (typiquement la pompe de session).
#[derive(Clone)]
pub struct McpPublisher {
    world: Arc<Mutex<WorldState>>,
    ui_ready: Arc<AtomicBool>,
}

impl McpPublisher {
    /// Remplace l'état du monde et met à jour le drapeau « UI prête ».
    /// Le verrou n'est tenu que le temps d'une affectation.
    pub fn publish(&self, sessions: Vec<SessionSnapshot>, ready: bool) {
        {
            let mut w = self
                .world
                .lock()
                .unwrap_or_else(|p| p.into_inner()); // jamais paniquer sur poison
            w.sessions = sessions;
        }
        self.ui_ready.store(ready, Ordering::Release);
    }
}

/// Moitié serveur MCP — clonable, partagée entre toutes les sessions MCP.
#[derive(Clone)]
pub struct McpServerHalf {
    cmd_tx: SyncSender<McpCommand>,
    cfg_tx: SyncSender<ConfigRequest>,
    world: Arc<Mutex<WorldState>>,
    ui_ready: Arc<AtomicBool>,
}

impl McpServerHalf {
    /// File la commande vers l'UI. Erreur lisible si la file est pleine ou
    /// si l'UI a rendu son receiver (fenêtre fermée).
    pub fn submit(&self, cmd: McpCommand) -> Result<(), String> {
        match self.cmd_tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                Err("file de commandes MCP pleine (l'UI ne draine plus ?)".into())
            }
            Err(TrySendError::Disconnected(_)) => {
                Err("interface putty-rs fermée : plus de consommateur MCP".into())
            }
        }
    }

    /// Copie instantanée du monde (lock bref + clone).
    pub fn world(&self) -> WorldState {
        self.world
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Retrouve une session par son id uuid.
    pub fn find_session(&self, id: &str) -> Option<SessionSnapshot> {
        self.world()
            .sessions
            .into_iter()
            .find(|s| s.id == id)
    }

    /// `true` dès que l'UI a publié un premier snapshot (handshake MCP
    /// autorisé sans bloquer, sinon les outils rapportent `status:"starting"`).
    pub fn ui_ready(&self) -> bool {
        self.ui_ready.load(Ordering::Acquire)
    }

    /// Délégué get/set_config vers l'UI, avec timeout (l'UI n'est jamais
    /// bloquée ; c'est le tool MCP qui attend). `value == None` → Get.
    pub fn ask_config(&self, key: &str, value: Option<String>) -> Result<String, String> {
        let (tx, rx) = mpsc::sync_channel::<ConfigReply>(1);
        let op = match value {
            Some(v) => ConfigOp::Set {
                key: key.to_string(),
                value: v,
            },
            None => ConfigOp::Get {
                key: key.to_string(),
            },
        };
        self.cfg_tx
            .try_send(ConfigRequest { op, respond: tx })
            .map_err(|_| "file config MCP pleine (UI saturée)".to_string())?;
        let reply = rx
            .recv_timeout(CONFIG_RESPONSE_TIMEOUT)
            .map_err(|_| "délai dépassé : l'UI n'a pas répondu à la requête config".to_string())?;
        if reply.ok {
            Ok(reply.payload)
        } else {
            Err(reply.payload)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_commande_aller_retour() {
        let (ui, server) = McpBus::new();
        // File vide → rien.
        assert!(ui.drain_commands().is_empty());

        server
            .submit(McpCommand::SendText {
                id: "11111111-2222-3333-4444-555555555555".into(),
                text: "user\r".into(),
            })
            .expect("submit");
        server
            .submit(McpCommand::SetTheme { dark: true })
            .expect("submit SetTheme");

        let cmds = ui.drain_commands();
        assert_eq!(cmds.len(), 2);
        match &cmds[0] {
            McpCommand::SendText { id, text } => {
                assert_eq!(id, "11111111-2222-3333-4444-555555555555");
                assert_eq!(text, "user\r");
            }
            other => panic!("mauvaise commande : {other:?}"),
        }
        assert!(matches!(cmds[1], McpCommand::SetTheme { dark: true }));
        // La file est vidée.
        assert!(ui.drain_commands().is_empty());
    }

    #[test]
    fn bus_file_pleine_refuse_sans_bloquer() {
        let (_ui, server) = McpBus::new();
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            server
                .submit(McpCommand::CloseSession { id: "x".into() })
                .unwrap();
        }
        // 65e commande : refus propre (pas de blocage du thread tokio).
        let err = server
            .submit(McpCommand::CloseSession { id: "x".into() })
            .unwrap_err();
        assert!(err.contains("pleine"), "{err}");
    }

    #[test]
    fn bus_snapshot_publish_puis_find() {
        let (ui, server) = McpBus::new();
        assert!(!server.ui_ready());
        assert!(server.find_session("abc").is_none());

        ui.publish(
            vec![SessionSnapshot::starting("abc", "127.0.0.1", 22, "ssh")],
            true,
        );
        assert!(server.ui_ready());
        let s = server.find_session("abc").expect("session publiée");
        assert_eq!(s.host, "127.0.0.1");
        assert_eq!(s.port, 22);
        assert_eq!(s.state, SessionState::Starting);
        assert_eq!(server.world().sessions.len(), 1);
    }

    #[test]
    fn etat_state_serialise_snake_case() {
        // Le wire format des états est contractuel côté scripts de smoke.
        let json = serde_json::to_string(&SessionState::HostkeyPending).unwrap();
        assert_eq!(json, "\"hostkey_pending\"");
        let json = serde_json::to_string(&SessionState::PasswordPrompt).unwrap();
        assert_eq!(json, "\"password_prompt\"");
        let back: SessionState = serde_json::from_str("\"login_prompt\"").unwrap();
        assert_eq!(back, SessionState::LoginPrompt);
        assert_eq!(back.as_str(), "login_prompt");
    }

    #[test]
    fn config_aller_retour() {
        // `McpHandle` est Send (mpsc::Receiver l'est) : on peut le déplacer
        // dans un thread « UI factice » qui répond aux requêtes config,
        // pendant que le thread « MCP » attend via ask_config().
        let (ui, server) = McpBus::new();
        std::thread::spawn(move || {
            // Répondeur continu (~1 s) : traite toutes les requêtes reçues.
            for _ in 0..500 {
                for req in ui.drain_config_requests() {
                    match &req.op {
                        ConfigOp::Get { key } => {
                            let _ = req.respond.try_send(ConfigReply {
                                ok: true,
                                payload: format!("{key}=dark"),
                            });
                        }
                        ConfigOp::Set { key, value } => {
                            let _ = req.respond.try_send(ConfigReply {
                                ok: true,
                                payload: format!("{key}<={value}"),
                            });
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        assert_eq!(server.ask_config("AppTheme", None).unwrap(), "AppTheme=dark");
        assert_eq!(
            server.ask_config("McpPort", Some("3927".into())).unwrap(),
            "McpPort<=3927"
        );
    }
}
