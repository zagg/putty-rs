//! Serveur MCP embarqué dans putty-rs (spec : `docs/MCP_PILOT.md`).
//!
//! Transport **streamable HTTP** (`POST /mcp`) sur **127.0.0.1:3927** via le
//! SDK officiel `rmcp` 3.8.0. Un thread tokio dédié héberge le serveur ;
//! l'UI eframe reste seule maître du modèle terminal et communique par le
//! bus [`crate::mcp::bus`] (aucune dépendance à `app`/`backend` ici).
//!
//! ## API rmcp 3.8.0 réellement vérifiée sur docs.rs (27/10/2026)
//! - Routing des outils : `#[rmcp::tool_router(server_handler)]` sur un
//!   `impl` block + `#[rmcp::tool(description = …)]` sur des méthodes
//!   `async fn(&self, Parameters(p): Parameters<T>) -> Result<CallToolResult, ErrorData>`.
//!   (Le `service!`/`#[tool(tool_box)]` de la 0.x n'existe plus : la 3.8 est
//!   à style « extractors ».)
//!   https://docs.rs/rmcp/3.8.0/rmcp/attr.tool_router.html
//!   https://docs.rs/rmcp/3.8.0/rmcp/attr.tool.html
//! - Extracteur de paramètres : `rmcp::handler::server::wrapper::Parameters<T>`
//!   (T: `Deserialize` + `schemars::JsonSchema`).
//!   https://docs.rs/rmcp/3.8.0/rmcp/handler/server/wrapper/struct.Parameters.html
//! - Résultats/erreurs : `rmcp::model::CallToolResult::{structured,error,success}`,
//!   `ContentBlock::text`, `ErrorData::invalid_params(msg, None)` (réexporté
//!   à la racine : `rmcp::ErrorData`).
//!   https://docs.rs/rmcp/3.8.0/rmcp/model/struct.CallToolResult.html
//! - Transport HTTP serveur : `rmcp::transport::StreamableHttpService::new(
//!   service_factory: Fn() -> Result<S, io::Error>, Arc<M: SessionManager>,
//!   StreamableHttpServerConfig)` ; c'est un `tower::Service<Request<B>>`
//!   monté dans un routeur **axum 0.8** (dev-dep axum ^0.8 de rmcp, exemple
//!   officiel de la doc). Sessions : `LocalSessionManager` (défaut,
//!   `Mcp-Session-Id`).
//!   https://docs.rs/rmcp/3.8.0/rmcp/transport/streamable_http_server/tower/struct.StreamableHttpService.html
//! - Features crates.io : `server` ⇒ `schemars`+`uuid`+`transport-async-rw`+
//!   `pastey` ; `transport-streamable-http-server` ⇒ `server-side-http` +
//!   `transport-streamable-http-server-session` + `transport-worker`.
//!   https://docs.rs/crate/rmcp/3.8.0/features
//!
//! Sécurité : bind **loopback uniquement**, pas d'auth (poste de dev) —
//! tout process local peut piloter le terminal (cf. MCP_PILOT §Sécurité).

mod bus;

pub use bus::{
    ConfigOp, ConfigReply, McpBus, McpCommand, McpHandle, McpServerHalf,
    SessionSnapshot, SessionState,
};

use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::handler::server::wrapper::Parameters;
// Importé pour l'`impl ServerHandler` émis par `#[tool_router(server_handler)]`
// (la macro le qualifie mais l'import explicite évite tout doute à la compilation).
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// Port par défaut : « PUTY » sur le pavé téléphone (MCP_PILOT §cycle de vie).
pub const DEFAULT_MCP_PORT: u16 = 3927;

/// Limite d'octets par appel `send_text` (MCP_PILOT §Sécurité).
const MAX_SEND_TEXT_BYTES: usize = 64 * 1024;

/// Délai par défaut de `await_state` quand le client n'en donne pas.
const DEFAULT_AWAIT_TIMEOUT_MS: u64 = 10_000;
/// Plafond du délai `await_state` (le tool bloque son task tokio, jamais l'UI).
const MAX_AWAIT_TIMEOUT_MS: u64 = 120_000;
/// Pas de boucle de lecture des snapshots dans `await_state` / `new_session`.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Délai max d'attente de l'apparition d'une session créée par `new_session`.
const NEW_SESSION_WAIT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Types de paramètres des outils (schémas JSON via schemars)
// ---------------------------------------------------------------------------

/// Paramètre universel `{id}` (id = chaîne uuid exposée par `list_sessions`).
#[derive(Debug, Deserialize, JsonSchema)]
struct IdParam {
    /// Identifiant de session (uuid, voir `list_sessions`).
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NewSessionParam {
    /// Hôte distant (nom ou IP).
    host: String,
    /// Port TCP (défaut 22).
    #[serde(default)]
    port: Option<u16>,
    /// Protocole : `ssh` (défaut) | `telnet` | `raw` | `serial`.
    #[serde(default)]
    proto: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SendTextParam {
    id: String,
    /// Texte à injecter dans le flux clavier. **Pas de CR automatique** :
    /// envoyez `"user\r"` pour valider la ligne.
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SendKeysParam {
    id: String,
    /// Touches nommées : `Enter`, `Tab`, `Esc`, `↑`/`Up`, `Ctrl-C`, …
    /// (mêmes séquences que `terminal.rs`).
    keys: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadScreenParam {
    id: String,
    /// `visible` (défaut) | `full` (avec journal, si l'UI l'a publié).
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadScrollbackParam {
    id: String,
    /// Décalage depuis la fin (0 = dernières lignes), défaut 0.
    #[serde(default)]
    before: Option<usize>,
    /// Nombre de lignes demandées, défaut 50, plafonné à 500.
    #[serde(default)]
    count: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadStreamParam {
    id: String,
    /// Curseur absolu retourné par l'appel précédent (`next_cursor`) ; 0 =
    /// depuis le début du flux connu.
    #[serde(default)]
    cursor: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AwaitStateParam {
    id: String,
    /// État attendu parmi `starting, connecting, login_prompt,
    /// password_prompt, auth_other, hostkey_pending, connected, failed,
    /// closed`.
    #[serde(default)]
    expect: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Motif recherché dans l'écran (sous-chaîne ; ancrage possible par
    /// `^début` / `fin$`). Ex. `^login as: `.
    #[serde(default)]
    regex: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AcceptHostKeyParam {
    id: String,
    /// `true` = « Connect Once » : ne pas stocker dans `SshHostKeys` (REA §2).
    #[serde(default)]
    once: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ResizeParam {
    id: String,
    /// Colonnes, min 2.
    cols: u16,
    /// Lignes, min 2.
    rows: u16,
}

/// Thème accepté par `set_theme` (pont THEME_DARK.md).
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Theme {
    Dark,
    Light,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetThemeParam {
    theme: Theme,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetConfigParam {
    /// Clé registre PuTTY (docs/PUTTY_FEATURE_SPEC §4) ou putty-rs
    /// (`AppTheme`, `McpPort`).
    key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetConfigParam {
    key: String,
    value: String,
}

// ---------------------------------------------------------------------------
// Le service MCP
// ---------------------------------------------------------------------------

/// Gestionnaire des 15 outils MCP, clonable (une instance par session MCP,
/// créée par la `service_factory` de `StreamableHttpService`).
#[derive(Clone)]
pub struct PuTTYRsServer {
    bus: McpServerHalf,
}

impl PuTTYRsServer {
    pub fn new(bus: McpServerHalf) -> Self {
        PuTTYRsServer { bus }
    }

    /// Résultat succès JSON : `structured_content` + miroir texte (certains
    /// clients n'affichent que `content`).
    fn ok(v: Value) -> Result<CallToolResult, ErrorData> {
        let mut r = CallToolResult::structured(v.clone());
        r.content = vec![ContentBlock::text(v.to_string())];
        Ok(r)
    }

    /// Erreur « métier » visible par l'appelant (le tool a tourné, mais
    /// n'a pas pu satisfaire la demande).
    fn fail(msg: impl Into<String>) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::error(vec![ContentBlock::text(msg.into())]))
    }

    /// Erreur protocole JSON-RPC -32602 (paramètres invalides).
    fn bad(msg: impl Into<String>) -> ErrorData {
        ErrorData::invalid_params(msg.into(), None)
    }

    /// Fiche de session pour les listings (id, title, state, host, port, proto).
    fn session_dto(s: &SessionSnapshot) -> Value {
        json!({
            "id": s.id,
            "title": s.title,
            "state": s.state.as_str(),
            "host": s.host,
            "port": s.port,
            "proto": s.proto,
        })
    }

    /// Résumé d'écran pour `await_state.excerpt` : 5 dernières lignes non vides.
    fn excerpt(s: &SessionSnapshot) -> String {
        let mut keep: Vec<String> =
            s.lines.iter().filter(|l| !l.trim().is_empty()).cloned().collect();
        if keep.len() > 5 {
            keep = keep.split_off(keep.len() - 5);
        }
        keep.join("\n")
    }
}

/// Motif « regex » minimaliste du v0.1 (le périmètre MCP_PILOT ne couvre que
/// sous-chaîne + ancres) : `^pref` ancre le début de ligne, `suf$` la fin,
/// sinon recherche par sous-chaîne sur n'importe quelle ligne.
/// [API-À-VÉRIFIER-build] si un jour un vrai `regex` crate est requis.
fn screen_matches(s: &SessionSnapshot, pattern: &str) -> bool {
    for line in &s.lines {
        let (anchored_start, pat) = pattern.strip_prefix('^').map_or((false, pattern), |p| (true, p));
        let (anchored_end, pat) = pat.strip_suffix('$').map_or((false, pat), |p| (true, p));
        let hit = match (anchored_start, anchored_end) {
            (true, true) => line == pat,
            (true, false) => line.starts_with(pat),
            (false, true) => line.ends_with(pat),
            (false, false) => line.contains(pat),
        };
        if hit {
            return true;
        }
    }
    false
}

/// Le routeur d'outils : `#[tool_router(server_handler)]` génère aussi
/// `impl ServerHandler for PuTTYRsServer` (call_tool/list_tools/get_info).
/// doc: https://docs.rs/rmcp/3.8.0/rmcp/attr.tool_router.html
#[rmcp::tool_router(server_handler)]
impl PuTTYRsServer {
    // 1 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "list_sessions",
        description = "Liste les sessions putty-rs ouvertes (id, title, state, host, port, proto). Renvoie status:\"starting\" tant que l'UI n'a pas publié d'état."
    )]
    async fn list_sessions(&self) -> Result<CallToolResult, ErrorData> {
        let sessions: Vec<Value> = self.bus.world().sessions.iter().map(Self::session_dto).collect();
        let mut v = json!({ "sessions": sessions });
        if !self.bus.ui_ready() {
            v["status"] = json!("starting");
        }
        Self::ok(v)
    }

    // 2 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "new_session",
        description = "Ouvre une nouvelle session (onglet/fenêtre selon options). Entrées {host, port?, proto?} ; retourne {id}. L'id uuid est généré par l'UI puis attendu ici (~ms)."
    )]
    async fn new_session(
        &self,
        Parameters(p): Parameters<NewSessionParam>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.host.trim().is_empty() {
            return Err(Self::bad("host requis (nom ou IP)"));
        }
        let proto = p.proto.unwrap_or_else(|| "ssh".into()).to_lowercase();
        if !matches!(proto.as_str(), "ssh" | "telnet" | "raw" | "serial") {
            return Err(Self::bad(format!(
                "proto invalide '{proto}' (ssh|telnet|raw|serial)"
            )));
        }
        let port = p.port.unwrap_or(22);
        let known: Vec<String> = self
            .bus
            .world()
            .sessions
            .iter()
            .filter(|s| s.host == p.host && s.port == port)
            .map(|s| s.id.clone())
            .collect();
        self.bus
            .submit(McpCommand::NewSession {
                host: p.host.clone(),
                port,
                proto: proto.clone(),
            })
            .map_err(Self::bad)?;
        // L'UI crée la session (id uuid) dès sa prochaine frame : on l'attend
        // sans jamais bloquer l'UI (sleep sur la task tokio du MCP).
        let res = tokio::time::timeout(NEW_SESSION_WAIT, async {
            loop {
                if let Some(s) = self
                    .bus
                    .world()
                    .sessions
                    .iter()
                    .find(|s| s.host == p.host && s.port == port && !known.contains(&s.id))
                {
                    let id = s.id.clone();
                    return Ok::<_, String>(id);
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        })
        .await;
        match res {
            Ok(Ok(id)) => Self::ok(json!({ "id": id })),
            Ok(Err(e)) => Self::fail(e),
            Err(_) => Self::fail(format!(
                "session {proto}://{host}:{port} demandée mais absente du monde après {} s (check list_sessions)",
                NEW_SESSION_WAIT.as_secs(),
                proto = proto,
                host = p.host,
                port = port,
            )),
        }
    }

    // 3 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "close_session",
        description = "Ferme proprement une session (équivaut à la fermeture PuTTY, exit(0x12)). Entrées {id} ; retourne {ok}."
    )]
    async fn close_session(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        if self.bus.find_session(&p.id).is_none() {
            return Self::fail(format!("session inconnue : {}", p.id));
        }
        self.bus
            .submit(McpCommand::CloseSession { id: p.id })
            .map_err(Self::bad)?;
        Self::ok(json!({ "ok": true }))
    }

    // 4 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "send_text",
        description = "Injecte du texte (UTF-8) dans le flux clavier d'une session, SANS CR automatique : envoyez \"user\\r\". Entrées {id, text} ; retourne {bytes_written}. Max 64 Ko/appel."
    )]
    async fn send_text(
        &self,
        Parameters(p): Parameters<SendTextParam>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.text.len() > MAX_SEND_TEXT_BYTES {
            return Err(Self::bad(format!(
                "send_text limité à {} Ko (reçu {} octets)",
                MAX_SEND_TEXT_BYTES / 1024,
                p.text.len()
            )));
        }
        if self.bus.find_session(&p.id).is_none() {
            return Self::fail(format!("session inconnue : {}", p.id));
        }
        let written = p.text.as_bytes().len();
        self.bus
            .submit(McpCommand::SendText { id: p.id, text: p.text })
            .map_err(Self::bad)?;
        Self::ok(json!({ "bytes_written": written }))
    }

    // 5 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "send_keys",
        description = "Envoie des touches nommées (Enter, Tab, Esc, Up/↑, Ctrl-C…) ; l'UI produit les mêmes séquences que le clavier local (DECARM/app-keys). Entrées {id, keys[]} ; retourne {ok}."
    )]
    async fn send_keys(
        &self,
        Parameters(p): Parameters<SendKeysParam>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.keys.is_empty() {
            return Err(Self::bad("keys[] requis (ex. [\"Enter\"])"));
        }
        if self.bus.find_session(&p.id).is_none() {
            return Self::fail(format!("session inconnue : {}", p.id));
        }
        self.bus
            .submit(McpCommand::SendKeys { id: p.id, keys: p.keys })
            .map_err(Self::bad)?;
        Self::ok(json!({ "ok": true }))
    }

    // 6 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "read_screen",
        description = "Lit l'écran rendu d'une session. Entrées {id, mode?\"full\"|\"visible\"} ; retourne {title, cols, rows, cursor{x,y}, lines[]}."
    )]
    async fn read_screen(
        &self,
        Parameters(p): Parameters<ReadScreenParam>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(s) = self.bus.find_session(&p.id) else {
            return Self::fail(format!("session inconnue : {}", p.id));
        };
        let mode = p.mode.unwrap_or_else(|| "visible".into());
        if !matches!(mode.as_str(), "visible" | "full") {
            return Err(Self::bad(format!("mode invalide '{mode}' (full|visible)")));
        }
        // v0.1 : l'UI publie l'écran visible dans `lines` ; mode "full" =
        // idem (l'UI peut préfixer le journal — TODO intégration app.rs).
        Self::ok(json!({
            "title": s.title,
            "cols": s.cols,
            "rows": s.rows,
            "cursor": { "x": s.cursor.0, "y": s.cursor.1 },
            "lines": s.lines,
            "mode": mode,
        }))
    }

    // 7 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "read_scrollback",
        description = "Relit le journal d'une session. Entrées {id, before, count} ; retourne {lines[]}. before = décalage depuis la fin, count plafonné à 500."
    )]
    async fn read_scrollback(
        &self,
        Parameters(p): Parameters<ReadScrollbackParam>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(s) = self.bus.find_session(&p.id) else {
            return Self::fail(format!("session inconnue : {}", p.id));
        };
        let count = p.count.unwrap_or(50).clamp(1, 500);
        let before = p.before.unwrap_or(0);
        let total = s.lines.len();
        if total == 0 || before >= total {
            return Self::ok(json!({ "lines": [], "scrollback_len": s.scrollback_len }));
        }
        let end = total.saturating_sub(before);
        let start = end.saturating_sub(count);
        Self::ok(json!({
            "lines": &s.lines[start..end],
            "scrollback_len": s.scrollback_len,
        }))
    }

    // 8 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "read_output_stream",
        description = "Flux de sortie post-interprétation (façon « copy all »). Entrées {id, cursor} ; retourne {chunk, next_cursor, truncated}. Reprendre avec next_cursor."
    )]
    async fn read_output_stream(
        &self,
        Parameters(p): Parameters<ReadStreamParam>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(s) = self.bus.find_session(&p.id) else {
            return Self::fail(format!("session inconnue : {}", p.id));
        };
        // output_seq = total d'octets émis ; output_tail = fin conservée.
        let tail = &s.output_tail;
        let start_seq = s.output_seq.saturating_sub(tail.len() as u64);
        let cursor = p.cursor.unwrap_or(0).min(s.output_seq);
        let truncated = cursor < start_seq;
        let from = cursor.saturating_sub(start_seq) as usize;
        let chunk = String::from_utf8_lossy(&tail.get(from..).unwrap_or(&[])).to_string();
        Self::ok(json!({
            "chunk": chunk,
            "next_cursor": s.output_seq,
            "truncated": truncated,
        }))
    }

    // 9 -------------------------------------------------------------------
    #[rmcp::tool(
        name = "await_state",
        description = "Attend (côté MCP, jamais l'UI) qu'une session atteigne un état ou qu'un motif apparaisse à l'écran. Entrées {id, expect?, timeout_ms?, regex?} ; retourne {matched, state, excerpt}."
    )]
    async fn await_state(
        &self,
        Parameters(p): Parameters<AwaitStateParam>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.expect.is_none() && p.regex.is_none() {
            return Err(Self::bad("expect et/ou regex requis"));
        }
        let timeout = Duration::from_millis(
            p.timeout_ms
                .unwrap_or(DEFAULT_AWAIT_TIMEOUT_MS)
                .clamp(100, MAX_AWAIT_TIMEOUT_MS),
        );
        let started = Instant::now();
        // Boucle de polling 100 ms sous tokio::time::timeout : la task MCP
        // dort, le thread eframe n'est jamais attendu.
        let polled = tokio::time::timeout(timeout, async {
            loop {
                if let Some(s) = self.bus.find_session(&p.id) {
                    let state_ok = p
                        .expect
                        .as_ref()
                        .is_none_or(|e| s.state.as_str() == e.as_str());
                    let regex_ok = p
                        .regex
                        .as_ref()
                        .is_none_or(|r| screen_matches(&s, r));
                    if state_ok && regex_ok {
                        return Ok::<_, String>(s);
                    }
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        })
        .await;
        match polled {
            Ok(Ok(s)) => Self::ok(json!({
                "matched": true,
                "state": s.state.as_str(),
                "excerpt": Self::excerpt(&s),
            })),
            Ok(Err(e)) => Self::fail(e),
            Err(_) => {
                let (state, excerpt) = match self.bus.find_session(&p.id) {
                    Some(s) => (s.state.as_str().to_string(), Self::excerpt(&s)),
                    None => ("unknown".to_string(), String::new()),
                };
                Self::ok(json!({
                    "matched": false,
                    "state": state,
                    "excerpt": excerpt,
                    "waited_ms": started.elapsed().as_millis() as u64,
                }))
            }
        }
    }

    // 10 ------------------------------------------------------------------
    #[rmcp::tool(
        name = "connection_status",
        description = "État détaillé d'une connexion. Entrées {id} ; retourne {state, host, port, proto, error?, cipher?, kex?, hostkey_fp?, banner?}. cipher/kex/hostkey_fp/banner servis en v0.1 dès que le backend les publie (null en attendant). Jamais de mot de passe (SSHLogOmitPasswords)."
    )]
    async fn connection_status(
        &self,
        Parameters(p): Parameters<IdParam>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(s) = self.bus.find_session(&p.id) else {
            return Self::fail(format!("session inconnue : {}", p.id));
        };
        let mut v = json!({
            "state": s.state.as_str(),
            "host": s.host,
            "port": s.port,
            "proto": s.proto,
            "cipher": Value::Null,
            "kex": Value::Null,
            "hostkey_fp": Value::Null,
            "banner": Value::Null,
        });
        if let Some(e) = &s.error {
            v["error"] = json!(e);
        }
        if !self.bus.ui_ready() {
            v["status"] = json!("starting");
        }
        Self::ok(v)
    }

    // 11 ------------------------------------------------------------------
    #[rmcp::tool(
        name = "accept_host_key",
        description = "Répond au dialogue « The host key is not cached… ». Entrées {id, once?} ; retourne {stored}. once:true = Connect Once (non stocké dans SshHostKeys)."
    )]
    async fn accept_host_key(
        &self,
        Parameters(p): Parameters<AcceptHostKeyParam>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(s) = self.bus.find_session(&p.id) else {
            return Self::fail(format!("session inconnue : {}", p.id));
        };
        if s.state != SessionState::HostkeyPending {
            return Self::fail(format!(
                "aucune clé d'hôte en attente (état actuel : {})",
                s.state.as_str()
            ));
        }
        let once = p.once.unwrap_or(false);
        self.bus
            .submit(McpCommand::AcceptHostKey { id: p.id, once })
            .map_err(Self::bad)?;
        Self::ok(json!({ "stored": !once }))
    }

    // 12 ------------------------------------------------------------------
    #[rmcp::tool(
        name = "resize",
        description = "Redimensionne la session (min 2×2, maj NAWS/tail côté backend). Entrées {id, cols, rows} ; retourne {ok}."
    )]
    async fn resize(
        &self,
        Parameters(p): Parameters<ResizeParam>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.cols < 2 || p.rows < 2 {
            return Err(Self::bad(format!(
                "taille minimale 2×2 (reçu {}×{})",
                p.cols, p.rows
            )));
        }
        if self.bus.find_session(&p.id).is_none() {
            return Self::fail(format!("session inconnue : {}", p.id));
        }
        self.bus
            .submit(McpCommand::Resize {
                id: p.id,
                cols: p.cols,
                rows: p.rows,
            })
            .map_err(Self::bad)?;
        Self::ok(json!({ "ok": true }))
    }

    // 13 ------------------------------------------------------------------
    #[rmcp::tool(
        name = "set_theme",
        description = "Bascule le thème de l'interface. Entrées {theme:\"dark\"|\"light\"} ; retourne {applied}."
    )]
    async fn set_theme(
        &self,
        Parameters(p): Parameters<SetThemeParam>,
    ) -> Result<CallToolResult, ErrorData> {
        self.bus
            .submit(McpCommand::SetTheme {
                dark: matches!(p.theme, Theme::Dark),
            })
            .map_err(Self::bad)?;
        Self::ok(json!({ "applied": true }))
    }

    // 14 ------------------------------------------------------------------
    #[rmcp::tool(
        name = "get_config",
        description = "Lit une clé de config (registre PuTTY §4 + clés putty-rs AppTheme/McpPort). Entrées {key} ; retourne {value}. Délégué à l'UI, timeout 3 s."
    )]
    async fn get_config(
        &self,
        Parameters(p): Parameters<GetConfigParam>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.bus.ask_config(&p.key, None) {
            Ok(value) => Self::ok(json!({ "key": p.key, "value": value })),
            Err(e) => Self::fail(format!("get_config({}) : {e}", p.key)),
        }
    }

    // 15 ------------------------------------------------------------------
    #[rmcp::tool(
        name = "set_config",
        description = "Écrit une clé de config (mêmes clés que get_config). Entrées {key, value} ; retourne {value} confirmé. Délégué à l'UI, timeout 3 s."
    )]
    async fn set_config(
        &self,
        Parameters(p): Parameters<SetConfigParam>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.bus.ask_config(&p.key, Some(p.value.clone())) {
            Ok(value) => Self::ok(json!({ "key": p.key, "value": value })),
            Err(e) => Self::fail(format!("set_config({}) : {e}", p.key)),
        }
    }
}

// ---------------------------------------------------------------------------
// Démarrage du serveur
// ---------------------------------------------------------------------------

/// Démarre le serveur HTTP MCP **uniquement sur `127.0.0.1:port`**, route
/// `/mcp` (POST streamable HTTP ; GET/DELETE gérés par rmcp pour SSE et
/// fin de session). N'utilise pas anyhow : `Result<(), String>`.
///
/// Échec de bind (port déjà pris) → `Err` loggé par [`spawn`], la fenêtre
/// putty-rs continue de vivre.
/// Pont entre le `tower::Service` de rmcp et le `Handler` d'axum 0.8
/// (l'adaptateur `service::Tower` ayant été retiré en 0.8). Le service
/// transite par le STATE du routeur ; seul `req` est un extracteur requête.
type McpTowerService = StreamableHttpService<PuTTYRsServer, LocalSessionManager>;

#[axum::debug_handler(state = McpTowerService)]
async fn mcp_bridge(
    axum::extract::State(svc): axum::extract::State<McpTowerService>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use tower::ServiceExt as _;
    // Error = Infallible côté rmcp : expect sans coût réel.
    let resp = svc
        .oneshot(req.map(axum::body::Body::new))
        .await
        .expect("mcp service : erreur infaillible (rmcp)");
    resp.map(axum::body::Body::new)
}

pub async fn run_server(bus_half: McpServerHalf, port: u16) -> Result<(), String> {
    let handler = PuTTYRsServer::new(bus_half);
    let service = StreamableHttpService::new(
        // Une instance du handler par session MCP (Mcp-Session-Id).
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    // Route unique /mcp. En axum 0.8 l'adaptateur `service::Tower` a été
    // supprimé : le `tower::Service` rmcp transite par l'ÉTAT du routeur et
    // `mcp_bridge` fait le pont (Handler async + tower::ServiceExt::oneshot).
    let app = axum::Router::new()
        .route("/mcp", axum::routing::any(mcp_bridge))
        .with_state(service);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr} impossible : {e} (port déjà pris ?)"))?;
    log::info!("serveur MCP putty-rs à l'écoute sur http://{addr}/mcp (loopback uniquement)");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("serveur HTTP MCP arrêté : {e}"))
}

/// Crée le **thread tokio dédié** (rt-multi-thread) qui porte le serveur.
/// Ne panique jamais si le port est pris : l'erreur est loggée et le thread
/// s'arrête, l'UI continue (mode dégradé sans MCP).
pub fn spawn(bus_half: McpServerHalf, port: u16) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("putty-rs-mcp".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("MCP : création du runtime tokio impossible : {e} (MCP désactivé)");
                    return;
                }
            };
            rt.block_on(async move {
                if let Err(e) = run_server(bus_half, port).await {
                    log::error!("MCP : {e} — putty-rs continue sans serveur MCP");
                }
            });
        })
        // Un échec de spawn de thread (mémoire) est irrécupérable côté MCP
        // seulement : on panique sur un message explicite, jamais sur le port.
        .expect("spawn thread putty-rs-mcp")
}

/// Génère un id de session (chaîne uuid v4, FIXÉ 27/10). Utilisé par l'UI
/// dans `McpCommand::NewSession` et par les tests ; exposé pub pour que
/// `app.rs` l'utilise sans dépendre de uuid directement.
pub fn new_session_id() -> String {
    Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: SessionState, lines: &[&str]) -> SessionSnapshot {
        SessionSnapshot {
            id: "id-1".into(),
            title: "t".into(),
            state,
            host: "h".into(),
            port: 22,
            proto: "ssh".into(),
            cols: 80,
            rows: 3,
            cursor: (0, 0),
            lines: lines.iter().map(|s| s.to_string()).collect(),
            scrollback_len: 0,
            error: None,
            output_seq: 0,
            output_tail: Vec::new(),
        }
    }

    #[test]
    fn ancre_et_sous_chaine() {
        let s = snap(SessionState::LoginPrompt, &["login as: ", "autre ligne"]);
        assert!(screen_matches(&s, "^login as: "));
        assert!(screen_matches(&s, "login"));
        assert!(screen_matches(&s, "ligne$"));
        assert!(!screen_matches(&s, "^autre$"));
    }

    #[test]
    fn excerpt_non_vides() {
        let s = snap(SessionState::Connected, &["", "a", "b", "   ", "c"]);
        assert_eq!(PuTTYRsServer::excerpt(&s), "a\nb\nc");
    }

    #[test]
    fn read_output_stream_reprise_curseur() {
        // Vérifie la logique curseur/tail sans serveur complet.
        let mut s = snap(SessionState::Connected, &[]);
        s.output_seq = 100;
        s.output_tail = b"abcdefghij".to_vec(); // derniers 10 octets : 90..100
        let start_seq = s.output_seq - s.output_tail.len() as u64;
        assert_eq!(start_seq, 90);
        // Curseur 95 → 5 derniers octets.
        let from = (95 - start_seq) as usize;
        assert_eq!(&s.output_tail[from..], b"fghij");
        // Curseur trop ancien → tronqué, renvoie tout le tail.
        let cur = 10u64.min(s.output_seq);
        assert!(cur < start_seq);
    }
}
