# Spéc — Serveur MCP embarqué dans `putty-rs` (pilotage live)

Objectif : exposer le terminal aux agents IA via **MCP (Model Context Protocol)** pendant que la fenêtre tourne, pour piloter des sessions en direct (envoyer des touches, lire l'écran, répondre aux invites, accepter les clés d'hôte…).

## Transport & cycle de vie
- Crate SDK officiel : **`rmcp = "3.8.0"`** (vérifié crates.io 27/10/2026 ; `max_stable_version = 3.8.0`) avec feature **streamable HTTP**.
- Serveur HTTP **sur 127.0.0.1 uniquement** (jamais 0.0.0.0), port par défaut **3927** (`PUTY` sur pavé téléphone), configurable :
  - CLI PuTTY-compat (extension putty-rs) : `-mcpport <n>`, `-nomcp` (désactive) — style drapeaux PuTTY `[REA §3]`.
  - Registre : valeur `McpPort` (DWORD) sous `HKCU\Software\SimonTatham\PuTTY\Default` `[DESIGN, aligné schéma §4]` ; `0` = désactivé.
- Runtime : thread dédié **tokio** (`rt-multi-thread, macros, net`) ; la boucle eframe reste maître du rendu.
- Bus interne : `std::sync::mpsc` / `crossbeam` + `Arc<Mutex<...>>` :
  - **État de session partagé** (`src/session.rs`, `LiveSession`) : parseur vt100, statut de connexion, tampon de sortie. Un **thread « pompe »** draine les événements du backend et met à jour cet état, puis publie les snapshots MCP (`McpPublisher`) — **sans dépendre des repaints eframe**. L'UI ne fait que lire l'état pour peindre. Correctif du 28/10 : quand la fenêtre est minimisée/cachée (eframe ne rappelle plus `update()`), `read_screen`/`await_state` continuent d'avancer alors que l'ancien code restait figé.
  - UI → MCP : snapshots (grille écran, titre, curseur, état connexion, buffer de journal avec séquence incrémentale), produits par la pompe (≈50 Hz max, coalescing) ; l'UI publie en plus les changements qu'elle décide (resize, fermeture, thème).
  - MCP → UI : file de commandes (`SendText`, `SendKey`, `Resize`, `AcceptHostKey`, `Connect`, `Close`, `SetTheme`) exécutées par le thread UI à chaque frame (comme un événement clavier normal). Limite connue : fenêtre *cachée* (`SW_HIDE`) = plus de frame UI, donc les commandes attendent ; les **lectures**, elles, restent à jour.
  - `read_screen` photographie toujours la **vue live** (bas du journal), même si l'utilisateur a remonté l'historique : la position de l'UI est restaurée après la photo.
- Handshake : à l'arrivée du premier client MCP, le serveur ne bloque pas si l'UI n'est pas prête ; renvoyer `status: "starting"`.

## Outils MCP exposés (schémas `#[tool]` rmcp, sérialisation serde)
| Outil | Entrées | Retour | Notes |
|---|---|---|---|
| `list_sessions` | – | `[{id, title, state, host?, port?, proto?}]` | `state ∈ {connecting, auth_prompt, connected, closed}` mappé depuis les prompts `[REA §5]` (`login as: ` → auth_prompt) |
| `new_session` | `{host, port?, proto?}` ou `{load}` | `{id}` | ouvre une fenêtre onglet/fenêtre selon `AppTheme`/options |
| `close_session` | `{id}` | `{ok}` | équivaut fermer = `exit(0x12)` propre |
| `send_text` | `{id, text}` | `{bytes_written}` | octets UTF-8/latin1 injectés dans le flux clavier, **sans CR automatique** (le client envoie `"user\r"` ; décision [DESIGN 27/10] pour lever l'ambiguïté des scripts de test) |
| `send_keys` | `{id, keys[]}` | `{ok}` | `Enter ↑ ↓ ← → Tab Esc Ctrl-C …` → mêmes séquences que `terminal.rs` (DECARM/app-keys) |
| `read_screen` | `{id, mode?:"full"\|"visible"}` | `{title, cols, rows, cursor{x,y}, lines[]}` | texte rendu + flags attributs simplifiés (bold/color index) |
| `read_scrollback` | `{id, before, count}` | `{lines[]}` | limite = `ScrollbackLines` |
| `read_output_stream` | `{id, cursor}` | `{chunk, next_cursor}` | flux brut post-escape (texte déjà interprété, façon « copy all » `[REA §7 menu]`) |
| `await_state` | `{id, expect?, timeout_ms?, regex?}` | `{matched, state, excerpt}` | bloque côté MCP (pas UI) — pour scripted login |
| `connection_status` | `{id}` | `{state, cipher?, kex?, hostkey_fp?, banner?, error?}` | `state ∈ {starting, connecting, login_prompt, password_prompt, auth_other, hostkey_pending, connected, failed, closed}` mappé depuis les prompts `[REA §5]` (`login as: ` → login_prompt ; `%s@%s's password: `/`Password:` → password_prompt ; dialogue clé d'hôte → hostkey_pending) ; `error` format `Unable to open connection to\n%s\n%s` `[REA §2]` |
| `accept_host_key` | `{id, once?:bool}` | `{stored:bool}` | répond au dialogue clé d'hôte `[REA §2/§7]` ; `once` = « Connect Once » |
| `resize` | `{id, cols, rows}` | `{ok}` | valide min 2×2, maj NAWS/tail via `term_size` `[REA §2]` |
| `set_theme` | `{theme:"dark"\|"light"}` | `{applied}` | pont vers THEME_DARK.md `[DESIGN]` |
| `get_config` / `set_config` | `{key, value?}` | `{value}` | clés registre PuTTY §4 + clés putty-rs (`AppTheme`, `McpPort`) |

Ressources MCP (`rmcp` resources) : `puttyrs://sessions`, `puttyrs://session/{id}/screen` (lecture seule, refresh à la demande).

## Sécurité
- Bind loopback uniquement ; pas d'auth (usage poste de dev) — documenter que **n'importe quel process local peut piloter le terminal** (rôle clavier). Option future `--mcp-token` `[DESIGN]`.
- `send_text` limite 64 Ki/appel ; débit snapshots plafonné (~50/s, coalescing de 20 ms).
- Ne jamais exposer le mot de passe (`-pw`) via les outils : masquer dans `connection_status` (règle d'omission des logs PuTTY `SSHLogOmitPasswords` `[RUNTIME §4]`).
- **Session id MCP** : chaîne (uuid) `[FIXÉ 27/10]` — les `id` de session putty-rs exposés aux outils sont aussi des chaînes.
- **Ordre de priorité du port** `[FIXÉ 27/10]` : `-mcpport` (CLI) > registre `McpPort` > défaut `3927` ; `-nomcp` l'emporte sur tout.

## Indécisions levées (27/10/2026, réponses aux [AMBIGU] des scripts de smoke)
1. Outils : **15 noms** (dont la paire `get_config`/`set_config`) — la mention « 13 » était un comptage d'arrondis ; le tableau fait foi.
2. `send_text` : **pas de CR auto** (voir tableau) ; les scripts existants (`user\r`) sont corrects.
3. État password : `password_prompt` ajouté à l'énumération `state` (voir `connection_status`) — la démo peut l'utiliser.
4. stdio de secours : `--mcp-stdio` = transport `transport-io` rmcp, **à activer explicitement**, friction GUI (pas de console allouée) documentée dans MCP_USAGE.md ; non implémenté en v0.1.
5. Handshake : **POST /mcp uniquement** (streamable HTTP) ; le « GET » du test 2 ci-dessous devient « POST initialize ».
6. Version de protocole : rmcp négocie ; les scripts affichent la version retenue — conforme.
7/8/9/10 : voir §Sécurité (port, ids chaîne, session-id optionnel toléré) et tableau (regex prompts).

## Tests d'acceptation (devront passer une fois buildé)
1. Lancer `putty-rs.exe -load demo-server-a -nomcp` : aucun port ouvert.
2. Sans `-nomcp` : `POST http://127.0.0.1:3927/mcp` (handshake `initialize` streamable) répond avec `result.protocolVersion` + header `mcp-session-id`.
3. Outillage manuel via curl/JSON-RPC : `new_session{host:"127.0.0.1",port:22,proto:"ssh"}` → `await_state(auth_prompt, regex:"^login as: ")` → `send_text` user → `read_screen` contient le shell → `close_session`.
4. `accept_host_key{once:true}` ne stocke pas dans `SshHostKeys` ; sans `once` → clé présente dans `HKCU…\SshHostKeys` après coup.

## Fichiers à produire (intégrés au scaffold en cours)
- `putty-rs/src/mcp/mod.rs` — serveur rmcp, transport HTTP, mapping tools ↔ bus.
- `putty-rs/src/mcp/bus.rs` — types `McpCommand`/`SessionSnapshot`, file + mutex, `McpPublisher`.
- `putty-rs/src/session.rs` — `LiveSession` (parseur vt100 + statut + flux) et thread pompe backend → état partagé → snapshots MCP.
- Hooks `main.rs`/`app.rs` : thread serveur si `mcp_enabled`, drain des commandes dans `eframe::App::update`, pompe de session indépendante du rendu.
- `Cargo.toml` : +`rmcp = { version = "3.8", default-features = false, features = ["server", "macros", "transport-streamable-http-server", "transport-io"] }` (liste de features **vérifiée** sur crates.io v3.8.0 : `server` ⇒ `schemars`+`uuid`+`pastey`+`transport-async-rw` ; `transport-streamable-http-server` ⇒ `server-side-http` (uuid, rand, tokio-stream, http, http-body(-util), bytes, sse-stream, tower, base64) — l'exemple d'intégration attend un routeur **axum/hyper** au-dessus de `StreamableHttpService`, vérifier docs.rs au moment de coder ; `transport-io` = stdio de secours pour `--mcp-stdio`), +`tokio = { version = "1", features = ["rt-multi-thread", "net", "io-util", "macros", "time", "sync"] }`, +`schemars` si exposé dans les signatures, +`axum` (version imposée par rmcp 3.8 — lire son `Cargo.lock`/docs) si requis.
- README : section « Piloter putty-rs en live via MCP » + exemple `curl` JSON-RPC `initialize`/`tools/call`.
