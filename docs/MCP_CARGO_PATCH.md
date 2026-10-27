# Patch Cargo + hooks d'intégration — serveur MCP embarqué (`src/mcp/`)

Fichiers produits par l'agent MCP : `src/mcp/mod.rs`, `src/mcp/bus.rs`, ce document.
Spec : `docs/MCP_PILOT.md`. **Aucune commande cargo lancée ici** (build piloté par l'agent main).

---

## 1. `Cargo.toml` — patch précis à appliquer

### 1.1 `[dependencies]` — bloc à ajouter (versions vérifiées docs.rs/crates.io, 27/10/2026)

```toml
# --- Serveur MCP embarqué (docs/MCP_PILOT.md) --------------------------------
# rmcp 3.8.0 = max_stable vérifiée (page docs.rs : publié 26/10/2026).
# Feature `server` ⇒ schemars + uuid + pastey + transport-async-rw (vérifié :
# https://docs.rs/crate/rmcp/3.8.0/features). `transport-streamable-http-server`
# ⇒ server-side-http (bytes, http, http-body(-util), rand, sse-stream,
# tokio-stream, tower-service, base64) + transport-streamable-http-server-session
# + transport-worker : rien d'autre à activer à la main.
rmcp = { version = "3.8.0", default-features = false, features = [
    "server",
    "macros",
    "transport-streamable-http-server",
    "transport-io",
] }
# Routeur HTTP exigé par l'exemple officiel d'intégration de StreamableHttpService
# (dev-dependency axum ^0.8 de rmcp 3.8.0, vu sur la page docs.rs du crate).
axum = "0.8"
# uuid v4 pour les ids de session (chaînes, FIXÉ 27/10) — rmcp Tirre uuid mais
# notre code fait `use uuid::Uuid` directement ⇒ dépendance directe requise.
uuid = { version = "1", features = ["v4"] }
# Derive JsonSchema sur nos structs de paramètres (rmcp dépend schemars ^1.0 /
# ^1.1.0 en dev : on épingle la même ligne 1.x, cargo unifie).
schemars = "1"
# Dérives serde dans bus.rs/mod.rs — PAS présentes dans le Cargo.toml actuel.
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### 1.2 `tokio` — ajuste la ligne existante (elle est optionnelle aujourd'hui)

```toml
# AVANT (optionnel, feature ssh uniquement) :
# tokio = { version = "1", features = ["rt-multi-thread", "net", "io-util", "time", "sync", "macros"], optional = true }
# APRÈS — obligatoire (le thread MCP tourne même sans feature ssh) + io-std
# requis par la feature rmcp transport-io (--mcp-stdio futur, MCP_PILOT décision 4) :
tokio = { version = "1", features = ["rt-multi-thread", "net", "io-util", "time", "sync", "macros", "io-std"] }
```

⚠️ Conséquences à valider côté agent main :
- la feature `ssh = ["dep:russh", "dep:tokio", "dep:async-trait"]` doit devenir
  `ssh = ["dep:russh", "dep:async-trait"]` (tokio n'est plus `dep:`-optionnel) ;
- **pas de feature `signal`** (inutile ici : pas de signaux, runtime créé à la main) ;
- `transport-io` peut être retiré si on veut raccourcir la compilation (stdio =
  non implémenté en v0.1) ; il n'apporte que tokio `io-std`+`net` (déjà présents).

### 1.3 Ce que j'ai **vérifié** sur docs.rs (rmcp 3.8.0)

| Point d'API | URL consultée |
|---|---|
| `#[tool]` (champs name/description/input_schema/annotations), `Parameters<T>` | https://docs.rs/rmcp/3.8.0/rmcp/attr.tool.html |
| `#[tool_router]` + flag `server_handler` (émet `impl ServerHandler`) | https://docs.rs/rmcp/3.8.0/rmcp/attr.tool_router.html |
| `Parameters` (extracteur) | https://docs.rs/rmcp/3.8.0/rmcp/handler/server/wrapper/struct.Parameters.html |
| `CallToolResult::structured/error/success`, `ContentBlock::text`, `ErrorData::invalid_params(msg, None)` (et doc « tool error vs protocol error ») | https://docs.rs/rmcp/3.8.0/rmcp/model/struct.CallToolResult.html |
| `StreamableHttpService::new(factory: Fn()->Result<S,io::Error>, Arc<M:SessionManager>, StreamableHttpServerConfig)`, tower `Service<Request<B>>`, `LocalSessionManager` (sessions via header `Mcp-Session-Id`) | https://docs.rs/rmcp/3.8.0/rmcp/transport/streamable_http_server/tower/struct.StreamableHttpService.html |
| Réexports `rmcp::transport::{StreamableHttpService, StreamableHttpServerConfig}` (features `-server`/`-session`), `rmcp::ErrorData`, `rmcp::schemars` | https://docs.rs/rmcp/3.8.0/rmcp/transport/index.html + page crate |
| Graphe de features complet (choix du bloc `features = [...]`) | https://docs.rs/crate/rmcp/3.8.0/features |

**Choix d'impl** : pas de `InMemoryTransport` ni de `service!`/`serve_client`
(API 0.x disparue en 3.8) ; `#[tool_router(server_handler)]` +
`StreamableHttpService` monté dans `axum::Router::new().route("/mcp", any(service))`,
servi par `axum::serve` sur un `TcpListener` lié à `127.0.0.1:port` — c'est le
pattern de l'exemple officiel de la doc 3.8 (dev-deps axum ^0.8/hyper ^1).

### 1.4 Marqueurs `[API-À-VÉRIFIER-build]` restants (corrections après 1ʳᵉ compilation)

1. **mod.rs, `run_server`** — `axum::routing::any(service)` : l'`impl Handler`
   d'axum 0.8 pour un `tower::Service<Request<Body>>` est attesté par l'exemple
   officiel (`route("/mcp", get(..).post(service))`), mais `any()` vs
   `.post()` strict + fallback GET SSE reste à confirmer. Si erreur de trait,
   remplacer par `axum::routing::post(service)` (le pilote impose « POST /mcp
   uniquement », MCP_PILOT décision 5).
2. **mod.rs, `StreamableHttpServerConfig::default()`** — la page doc montre le
   champ `legacy_session_mode` défaut `true` ; l'existence de `Default` est
   quasi sûre mais pas citée textuellement. Sinon : champ par champ.
3. **mod.rs, affectation `r.content = ...` sur `CallToolResult`** (`#[non_exhaustive]` :
   lecture/écriture de champs publics autorisées hors crate, seul le
   struct-literal est interdit) — à confirmer au build ; plan B : ne garder que
   `CallToolResult::structured(v)`.
4. **bus.rs/mod.rs** tests : dépendent de `serde_json` (sera ajouté en §1.1).

---

## 2. `src/main.rs` — snippet (agent main)

```rust
mod mcp; // déclarer à côté de `mod app; mod backend; ...`

// Dans main(), APRÈS le parsing cmdline (cmdline.rs) et la lecture config (config.rs),
// AVANT eframe::run_native. Règle FIXÉ 27/10 : -nomcp > -mcpport > registre McpPort > 3927.
let mcp_port: u16 = if args.nomcp {
    0
} else {
    args.mcpport // -mcpport <n> (CLI)
        .or_else(config::registry_mcp_port) // DWORD McpPort sous HKCU\Software\SimonTatham\PuTTY\Default ; 0 = désactivé
        .unwrap_or(mcp::DEFAULT_MCP_PORT)   // 3927
};

if mcp_port != 0 {
    let (ui_half, server_half) = mcp::McpBus::new();     // file 64 + Arc<Mutex<WorldState>>
    let _mcp_thread = mcp::spawn(server_half, mcp_port); // thread tokio dédié ; bind 127.0.0.1
    // Port pris ? spawn logge une erreur et le thread meurt : la fenêtre continue.
    app.mcp = Some(ui_half); // champ `pub mcp: Option<mcp::McpHandle>` dans App
}
```

Notes :
- `spawn` ne panique **jamais** sur « port déjà pris » (`Err` loggée, UI continue,
  test d'acceptation 1 du pilote : `-nomcp` ⇒ aucun port ouvert).
- Ne pas appeler `spawn` quand `--no-session`/mode tool PuTTY (`-cleanup`,
  `-pgpfp`…) : même condition `!no_session && !nomcp` que vos autres lancements.

## 3. `src/app.rs` — snippet (agent main)

```rust
use crate::mcp::{ConfigOp, ConfigReply, McpCommand, McpHandle, SessionSnapshot};
use std::time::Instant;

pub struct App {
    // ... champs existants ...
    pub mcp: Option<McpHandle>,          // None si MCP désactivé
    mcp_last_publish: Instant,           // init : Instant::now()
}

// Début de eframe::App::update — les commandes MCP sont traitées comme des
// événements clavier (MCP_PILOT §cycle de vie), jamais l'inverse.
if let Some(bus) = &self.mcp {
    // (a) commandes MCP → UI/backend/terminal
    for cmd in bus.drain_commands() {
        match cmd {
            McpCommand::SendText { id, text } => self.inject_keyboard_bytes(&id, text.as_bytes()),
            McpCommand::SendKeys { id, keys } => self.inject_named_keys(&id, &keys), // mapping terminal.rs (DECARM/app-keys)
            McpCommand::Resize { id, cols, rows } => self.resize_session(&id, cols, rows), // + NAWS/tail [REA §2]
            McpCommand::AcceptHostKey { id, once } => self.hostkey_answer(&id, once), // once = Connect Once [REA §2/§7]
            McpCommand::NewSession { host, port, proto } => {
                let id = crate::mcp::new_session_id(); // chaîne uuid v4 (FIXÉ 27/10)
                self.open_session(&id, &host, port, &proto);
            }
            McpCommand::CloseSession { id } => self.close_session_clean(&id), // exit(0x12)
            McpCommand::SetTheme { dark } => self.apply_theme_flag(dark),     // pont THEME_DARK.md
        }
    }
    // (b) get_config/set_config → registre PuTTY §4 + clés putty-rs (AppTheme, McpPort).
    //     TODO intégration : brancher config_get/config_set sur winreg HKCU + overrides mémoire.
    for req in bus.drain_config_requests() {
        let (ok, payload): (bool, String) = match &req.op {
            ConfigOp::Get { key } => self.config_get(key),
            ConfigOp::Set { key, value } => self.config_set(key, value),
        };
        let _ = req.respond.try_send(ConfigReply { ok, payload }); // jamais bloquer l'UI
    }
    // (c) publication de l'état du monde à ~10 Hz max (coalescing §Sécurité).
    if self.mcp_last_publish.elapsed() >= std::time::Duration::from_millis(100) {
        bus.publish(self.build_mcp_snapshots(), true);
        self.mcp_last_publish = Instant::now();
        ctx.request_repaint_after(std::time::Duration::from_millis(100)); // assure le tick
    }
}

// Fabrication des snapshots — mapping des états depuis les prompts RÉELS (REA §5) :
//   "login as: " en attente                     → SessionState::LoginPrompt
//   "'s password: " / "Password:" / "login:"    → SessionState::PasswordPrompt
//   dialogue PuTTYHostKeyDialog visible         → SessionState::HostkeyPending
//   auth clavier interactive / passphrase       → SessionState::AuthOther
//   error = format "Unable to open connection to\n%s\n%s" (REA §2), mot de passe
//   jamais présent (SSHLogOmitPasswords, RUNTIME §4).
// output_seq = total d'octets interprétés ; output_tail = ~64 Ko de fin
// (« copy all », REA §7). scrollback_len plafonné ScrollbackLines (2000 défaut).
```

Champs requis par `SessionSnapshot` : voir `src/mcp/bus.rs` (doc FR sur chaque champ).

## 4. `src/cmdline.rs` — 2 flags (agent main)

> **Règle de priorité (FIXÉ 27/10)** : `-mcpport <n>` (CLI) **>** valeur DWORD
> `McpPort` sous `HKCU\Software\SimonTatham\PuTTY\Default` **>** défaut `3927` ;
> `0` (CLI ou registre) = MCP désactivé ; **`-nomcp` l'emporte sur tout**.

```rust
// Dans la structure Args de cmdline.rs :
pub mcpport: Option<u16>, // défaut None ; 0 = désactivé (validé au parse)
pub nomcp: bool,          // défaut false

// Dans la boucle de parsing (style flags PuTTY, message d'erreur façon
// binaire [REA §3] : `option "%s" requires an argument`) :
"-mcpport" => {
    let v = rest.next()
        .ok_or_else(|| format!("option \"{}\" requires an argument", "-mcpport"))?;
    args.mcpport = Some(v.parse::<u16>()
        .map_err(|_| format!("option \"-mcpport\" : nombre attendu, reçu {v}"))?);
}
"-nomcp" => args.nomcp = true,
```

## 5. Ordre d'intégration recommandé (agent main)

1. **`Cargo.toml` §1** d'abord (les tests `src/mcp/bus.rs` exigent `serde_json`,
   `src/mcp/mod.rs` exige rmcp/axum/uuid/schemars) — puis `cargo check` : c'est
   là que tombent les 4 marqueurs `[API-À-VÉRIFIER-build]`.
2. `src/main.rs` : `mod mcp;` + création du bus + `mcp::spawn` (binaire
   fonctionnel sans serveur MCP si `-nomcp`).
3. `src/cmdline.rs` : les 2 flags (sans eux, pas de `-nomcp` ⇒ test
   d'acceptation 1 impossible).
4. `src/app.rs` : drain + publish (le handshake MCP répond `status:"starting"`
   tant que `publish(_, true)` n'a jamais tourné — pas de deadlock possible).
5. Backend/terminal : mapping états + prompts REA §5, `output_seq/output_tail`,
   `config_get/config_set` (TODO §3b).
6. Tests d'acceptation MCP_PILOT avec le curl `POST /mcp` (initialize →
   `result.protocolVersion` + header `mcp-session-id` fourni par rmcp).

## 6. Correspondance outils (15, noms exacts du pilote)

`list_sessions`, `new_session`, `close_session`, `send_text` (**sans CR auto**,
64 Ko max), `send_keys`, `read_screen`, `read_scrollback`, `read_output_stream`
(curseur = `output_seq` absolu), `await_state` (polling snapshot 100 ms sous
`tokio::time::timeout`, UI jamais bloquée), `connection_status` (états étendus
incluant `login_prompt`/`password_prompt`/`hostkey_pending`), `accept_host_key`
(`{stored: !once}`), `resize` (min 2×2 ⇒ `-32602`), `set_theme`, `get_config`,
`set_config` (canal `ConfigOp` aller-retour, timeout 3 s côté MCP).
Ressources MCP (`puttyrs://…`) : non implémentées en v0.1 (hors périmètre bus),
à brancher plus tard via un `#[prompt]`/resources rmcp si voulu.
