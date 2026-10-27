# Piloter putty-rs via MCP

`putty-rs` embarque un serveur **MCP (Model Context Protocol)** qui expose le terminal en cours d'exécution à un agent IA : lister les sessions, envoyer des touches, lire l'écran, répondre aux invites d'authentification, accepter une clé d'hôte, etc.

Ce document décrit le branchement côté client. La spécification de référence reste [`MCP_PILOT.md`](MCP_PILOT.md) ; en cas de divergence, c'est elle qui fait foi.

---

## 1. Transport et démarrage

| Élément | Valeur |
|---|---|
| Transport | **HTTP « streamable »** (JSON-RPC 2.0), pas stdio |
| Endpoint | `http://127.0.0.1:3927/mcp` |
| Écoute | **loopback uniquement** (`127.0.0.1`), jamais `0.0.0.0` |
| Port par défaut | `3927` (`PUTY` sur un pavé téléphonique) |
| Version de protocole visée | `2025-06-18` |
| Session | header de réponse `mcp-session-id`, à renvoyer sur les requêtes suivantes s'il est présent |
| Réponses | JSON (`application/json`) **ou** SSE (`text/event-stream`, lignes `data: {...}`) |

Démarrage :

```powershell
# Port par défaut 3927
putty-rs.exe

# Port personnalisé (extension putty-rs)
putty-rs.exe -mcpport 4000

# Désactivation complète du serveur MCP
putty-rs.exe -nomcp
```

Le registre prévoit aussi la valeur DWORD `McpPort` sous
`HKCU\Software\SimonTatham\PuTTY\Default` ; `0` désactive le serveur.
La précédence exacte entre `-mcpport` et `McpPort` n'est pas précisée dans
`MCP_PILOT.md` : en cas de doute, vérifier le port réellement ouvert.

Au premier client, si l'interface n'est pas encore prête, le serveur peut
répondre `status: "starting"`. Réessayer après quelques centaines de
millisecondes.

---

## 2. Brancher un client MCP

### 2.1 OpenCode

OpenCode doit utiliser le transport **remote/HTTP**. Exemple de configuration
(dans le fichier de configuration OpenCode, global ou projet) :

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "putty-rs": {
      "type": "remote",
      "url": "http://127.0.0.1:3927/mcp",
      "enabled": true
    }
  }
}
```

- `"type": "remote"` correspond au transport HTTP/SSE de putty-rs.
- `"type": "local"` est réservé aux serveurs **stdio** (`command` + `args`) et
  ne s'applique **pas** au serveur embarqué, qui écoute sur un port HTTP.
- Aucun header d'authentification n'est nécessaire : le serveur est sans auth
  et limité à la loopback (voir §7).

> **TODO — clients stdio uniquement.** Si le client n'accepte que le transport
> stdio, il faudra utiliser le mode `--mcp-stdio` **prévu** par putty-rs.
> `MCP_PILOT.md` mentionne la feature rmcp `transport-io` comme « stdio de
> secours pour `--mcp-stdio` », mais le comportement CLI (activation, framing,
> cycle de vie) n'est pas encore spécifié ni garanti implémenté : ne le
> considérer comme disponible qu'après mise à jour de la spec et du binaire.
> En attendant, utiliser un client supportant le HTTP streamable, ou un pont
> stdio→HTTP externe.

### 2.2 Claude Desktop / Cursor (format générique `mcpServers`)

La plupart des clients « desktop » acceptent une entrée `mcpServers` avec une
URL :

```json
{
  "mcpServers": {
    "putty-rs": {
      "url": "http://127.0.0.1:3927/mcp"
    }
  }
}
```

Emplacements usuels (variables selon la version du client) :

- Claude Desktop (Windows) : `%APPDATA%\Claude\claude_desktop_config.json`
- Cursor : `%USERPROFILE%\.cursor\mcp.json` (global) ou `.cursor/mcp.json`
  (projet)

Selon la version, Claude Desktop peut n'accepter que des serveurs **stdio**
dans ce fichier. Dans ce cas :

1. vérifier que la version supporte les serveurs MCP distants/HTTP ;
2. sinon, utiliser un pont stdio→HTTP, ou attendre `--mcp-stdio` (TODO §2.1).

Cursor accepte les entrées `url` pour les serveurs MCP distants dans ses
versions récentes.

---

## 3. Outils exposés

Reprise fidèle du tableau de [`MCP_PILOT.md`](MCP_PILOT.md) (§« Outils MCP
exposés »). La spec annonce **13 outils** ; le tableau ci-dessous comporte
**14 lignes** : les 13 premières sont les outils « métier », la 14e regroupe
`get_config` **et** `set_config` (soit 15 noms d'outils si on les compte
séparément). Voir §8.

| Outil | Entrées | Retour | Notes |
|---|---|---|---|
| `list_sessions` | – | `[{id, title, state, host?, port?, proto?}]` | `state ∈ {connecting, auth_prompt, connected, closed}` mappé depuis les prompts `[REA §5]` (`login as: ` → auth_prompt) |
| `new_session` | `{host, port?, proto?}` ou `{load}` | `{id}` | ouvre une fenêtre onglet/fenêtre selon `AppTheme`/options |
| `close_session` | `{id}` | `{ok}` | équivaut fermer = `exit(0x12)` propre |
| `send_text` | `{id, text}` | `{bytes_written}` | octets UTF-8/latin1 injectés dans le flux clavier (ligne + `\r`) |
| `send_keys` | `{id, keys[]}` | `{ok}` | `Enter ↑ ↓ ← → Tab Esc Ctrl-C …` → mêmes séquences que `terminal.rs` (DECARM/app-keys) |
| `read_screen` | `{id, mode?:"full"\|"visible"}` | `{title, cols, rows, cursor{x,y}, lines[]}` | texte rendu + flags attributs simplifiés (bold/color index) |
| `read_scrollback` | `{id, before, count}` | `{lines[]}` | limite = `ScrollbackLines` |
| `read_output_stream` | `{id, cursor}` | `{chunk, next_cursor}` | flux brut post-escape (texte déjà interprété, façon « copy all » `[REA §7 menu]`) |
| `await_state` | `{id, expect?, timeout_ms?, regex?}` | `{matched, state, excerpt}` | bloque côté MCP (pas UI) — pour scripted login |
| `connection_status` | `{id}` | `{state, cipher?, kex?, hostkey_fp?, banner?, error?}` | `error` format `Unable to open connection to\n%s\n%s` `[REA §2]` |
| `accept_host_key` | `{id, once?:bool}` | `{stored:bool}` | répond au dialogue clé d'hôte `[REA §2/§7]` ; `once` = « Connect Once » |
| `resize` | `{id, cols, rows}` | `{ok}` | valide min 2×2, maj NAWS/tail via `term_size` `[REA §2]` |
| `set_theme` | `{theme:"dark"\|"light"}` | `{applied}` | pont vers THEME_DARK.md `[DESIGN]` |
| `get_config` / `set_config` | `{key, value?}` | `{value}` | clés registre PuTTY §4 + clés putty-rs (`AppTheme`, `McpPort`) |

### Ressources MCP

Le serveur expose aussi deux ressources en lecture seule, rafraîchies à la
demande (voir `MCP_PILOT.md`) :

- `puttyrs://sessions`
- `puttyrs://session/{id}/screen`

---

## 4. Tester à la main avec curl

Le serveur attend principalement des **POST JSON-RPC** sur `/mcp`. Le test
d'acceptation de `MCP_PILOT.md` mentionne un `GET` ; en pratique, utiliser les
requêtes POST ci-dessous (le `GET` peut renvoyer `405` selon l'implémentation).

Les réponses peuvent être du JSON brut **ou** du SSE. Dans le second cas,
chercher la ligne `data: {...}`. Toujours borner avec `--max-time` : un flux
SSE peut rester ouvert.

### 4.1 `initialize`

```bash
BASE=http://127.0.0.1:3927/mcp

curl -i --max-time 5 "$BASE" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}'
```

Réponse typique (JSON ou événement SSE) :

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"...","version":"..."}}}
```

Si le serveur renvoie un header `mcp-session-id`, le conserver :

```bash
SID=$(curl -s -D - -o /dev/null --max-time 5 "$BASE" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}' \
  | tr -d '\r' | awk -F': ' 'tolower($1)=="mcp-session-id"{print $2}')

# Header de session à réutiliser (tableau vide si le serveur est stateless).
AUTH=()
if [ -n "$SID" ]; then AUTH=(-H "mcp-session-id: $SID"); fi
```

Puis envoyer la notification d'initialisation (sans `id`, réponse HTTP vide
ou `202`) :

```bash
curl -s -o /dev/null -w '%{http_code}\n' --max-time 5 "$BASE" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  "${AUTH[@]}" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'
```

### 4.2 `tools/list`

```bash
curl -s --max-time 5 "$BASE" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  "${AUTH[@]}" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
```

### 4.3 `tools/call` — exemple `list_sessions`

```bash
curl -s --max-time 5 "$BASE" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  "${AUTH[@]}" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}'
```

Le résultat MCP est enveloppé dans `result.content[].text` ; le texte contient
le JSON de l'outil, par exemple :

```json
[{"id":"s1","title":"alice@host","state":"auth_prompt","host":"host","port":22,"proto":"ssh"}]
```

### 4.4 Variante Windows PowerShell 5.1

`curl` est un alias de `Invoke-WebRequest` en PowerShell 5.1 : utiliser
explicitement `curl.exe`.

```powershell
$base = 'http://127.0.0.1:3927/mcp'
$body = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}'

curl.exe -i --max-time 5 $base `
  -H "Content-Type: application/json" `
  -H "Accept: application/json, text/event-stream" `
  -d $body
```

---

## 5. Scripts fournis

Deux scripts PowerShell 5.1 (compatibles hôte Windows, pas besoin de pwsh 7)
sont fournis dans `scripts/`.

### 5.1 `mcp_smoke.ps1`

Vérifie le transport et les deux premiers outils :

1. `initialize` (`protocolVersion` `2025-06-18`) ;
2. extraction du header `mcp-session-id` s'il existe ;
3. `notifications/initialized` ;
4. `tools/call list_sessions` ;
5. `tools/call connection_status` sur la première session listée.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\mcp_smoke.ps1

# Port personnalisé / timeout plus court
.\scripts\mcp_smoke.ps1 -BaseUrl http://127.0.0.1:4000/mcp -TimeoutSec 5
```

Chaque étape affiche `[OK]`, `[WARN]` ou `[FAIL]` avec une réponse JSON
compacte. Code de sortie : `0` si tout passe, `1` sinon.

### 5.2 `mcp_demo_login.ps1`

Démo scriptée d'un login SSH, calquée sur les prompts observés dans
`putty.exe` :

1. `await_state { expect: "auth_prompt", regex: "^login as: " }` (30 s) ;
2. `send_text` du nom d'utilisateur + `\r` ;
3. `await_state { regex: "(?i)(login as:|password:)" }` avec file de réponses :
   - prompt `login as:` répété → renvoi du nom d'utilisateur ;
   - prompt `password:` (format réel `%s@%s's password: ` ou `Password:`) →
     `send_text` du mot de passe + `\r` ;
4. `await_state { expect: "connected" }` (20 s) ;
5. `read_screen { mode: "visible" }` (résumé du titre, de la taille, du
   curseur et des dernières lignes) ;
6. `close_session` (sauf `-KeepOpen`).

```powershell
# Session déjà ouverte : la première session non "closed" est utilisée
.\scripts\mcp_demo_login.ps1 -User alice

# Session précise (id tel qu'affiché par list_sessions)
.\scripts\mcp_demo_login.ps1 -User alice -SessionId "s1"

# Aucune session ouverte : en créer une via new_session
.\scripts\mcp_demo_login.ps1 -User alice -HostName 127.0.0.1 -Port 22

# Laisser la session ouverte à la fin
.\scripts\mcp_demo_login.ps1 -User alice -KeepOpen
```

Le mot de passe n'est jamais affiché. S'il n'est pas fourni, il est demandé
via `Read-Host -AsSecureString`. Les variables d'environnement
`PUTTY_RS_DEMO_USER` et `PUTTY_RS_DEMO_PASSWORD` sont aussi acceptées, mais
la saisie interactive est préférable (pas d'historique ni de variable
d'environnement persistante).

Codes de sortie : `0` succès, `1` échec MCP/terminal, `2` identifiants
manquants.

---

## 6. Cycle de vie d'une session pilotée

Exemple de scénario typique (les noms d'outils sont ceux du tableau §3) :

1. `new_session { host: "127.0.0.1", port: 22, proto: "ssh" }` → `{id}` ;
2. `await_state { id, expect: "auth_prompt", regex: "^login as: ", timeout_ms: 30000 }` ;
3. `send_text { id, text: "alice\r" }` ;
4. `await_state { id, regex: "(?i)password:", timeout_ms: 30000 }` ;
5. `send_text { id, text: "<mot de passe>\r" }` ;
6. `await_state { id, expect: "connected", timeout_ms: 20000 }` ;
7. `read_screen { id, mode: "visible" }` ;
8. `close_session { id }`.

Si le serveur présente une clé d'hôte inconnue, répondre avec
`accept_host_key { id, once: true }` (« Connect Once ») ou sans `once` pour
stocker la clé dans `SshHostKeys` (voir `MCP_PILOT.md`, tests d'acceptation).

---

## 7. Sécurité

- **Loopback uniquement.** Le serveur écoute sur `127.0.0.1`, jamais sur
  `0.0.0.0`. Ne pas le publier via un proxy, un tunnel ou une redirection de
  port.
- **Pas d'authentification.** Comme indiqué dans `MCP_PILOT.md`, **n'importe
  quel processus local peut piloter le terminal** (rôle clavier). Ne pas
  lancer putty-rs avec MCP actif sur un poste multi-utilisateur non maîtrisé.
- **Désactivation.** `-nomcp` ou `McpPort=0` ferme le serveur. À utiliser pour
  les sessions sensibles ou les tests qui ne doivent exposer aucun port.
- **Mots de passe masqués.** `connection_status` ne doit jamais exposer un
  mot de passe (`-pw`) : la règle d'omission des logs PuTTY
  (`SSHLogOmitPasswords`) s'applique. Les scripts fournis n'affichent ni ne
  journalisent le mot de passe.
- **Mot de passe en clair sur la loopback.** Le corps JSON-RPC n'est pas
  chiffré (HTTP simple) : le mot de passe transite en clair entre le script et
  le serveur local. Éviter les postes où un processus local non fiable peut
  capturer le trafic loopback. Ne pas utiliser `-pw` en ligne de commande
  (visible dans l'historique et la liste des processus).
- **Limites.** `send_text` est limité à 64 Kio par appel ; les snapshots sont
  plafonnés (10/s, coalescing) — voir `MCP_PILOT.md`.
- **Évolution prévue.** Un jeton `--mcp-token` est envisagé `[DESIGN]` mais
  n'est pas disponible dans la spec actuelle.

---

## 8. Limites connues et points à trancher

Ces points sont signalés pour éviter d'inventer un comportement non spécifié :

- **Nombre d'outils.** `MCP_PILOT.md` annonce « 13 outils », mais son tableau
  contient 14 lignes : 13 outils métier + une ligne `get_config` /
  `set_config` (2 noms). Total réel : 15 noms d'outils. `[AMBIGU]`
- **`send_text` et le retour chariot.** La spec dit « ligne + `\r` » (le
  serveur ajouterait le CR), mais le test d'acceptation écrit `send_text user`
  sans CR. Les scripts envoient explicitement `\r` (comme demandé) : si le
  serveur ajoute aussi un CR, un double Entrée peut se produire. `[AMBIGU]`
- **État pendant le prompt de mot de passe.** L'énumération ne contient que
  `connecting`, `auth_prompt`, `connected`, `closed` ; il n'y a pas de
  `password_prompt`. La démo n'utilise donc pas `expect` pour cette étape et
  se fie à la regex `(?i)(login as:|password:)`. `[AMBIGU]`
- **`--mcp-stdio`.** `MCP_PILOT.md` mentionne la feature rmcp `transport-io`
  comme « stdio de secours pour `--mcp-stdio` » (liste des features de
  `Cargo.toml`), mais le mode lui-même (activation, framing, cycle de vie)
  n'est pas encore spécifié. `[AMBIGU]`
- **`GET /mcp`.** Le test d'acceptation mentionne un `GET` ; le transport
  streamable utilise normalement des POST JSON-RPC. Les recettes §4 utilisent
  POST. `[AMBIGU]`
- **Version de protocole.** Les scripts demandent `2025-06-18` et affichent la
  version réellement négociée ; le serveur (rmcp) peut choisir une autre
  version supportée. `[AMBIGU]`
- **Précédence `-mcpport` / `McpPort`.** Non spécifiée explicitement dans
  `MCP_PILOT.md`. `[AMBIGU]`
- **`mcp-session-id` optionnel.** Le protocole MCP permet au serveur de ne pas
  attribuer de session ; les scripts continuent alors sans header (avec un
  `[WARN]`). `[AMBIGU]` côté implémentation putty-rs.
- **Type de `id` de session.** `MCP_PILOT.md` écrit `{id}` sans préciser
  nombre ou chaîne. Les scripts renvoient l'id de `list_sessions` tel quel ;
  l'option `-SessionId` de la démo est une chaîne. `[AMBIGU]`

---

## 9. Dépannage

| Symptôme | Piste |
|---|---|
| `Serveur MCP injoignable` / connexion refusée | putty-rs n'est pas lancé, a été lancé avec `-nomcp`, ou `McpPort=0`. Vérifier le port avec `Get-NetTCPConnection -LocalPort 3927`. |
| HTTP `404` | Mauvais chemin : l'endpoint est `/mcp`, pas `/`. |
| HTTP `400` / `406` | Vérifier `Content-Type: application/json` et `Accept: application/json, text/event-stream`. |
| `mcp-session-id` absent | Serveur sans gestion de session (autorisé) ; les scripts continuent avec `[WARN]`. |
| `curl` semble bloqué | Réponse SSE ouverte : ajouter `--max-time 5`. |
| `await_state` timeout sur `^login as: ` | Le prompt n'apparaît pas si `-l <user>` est utilisé (cf. test 10 de `MCP_PILOT.md`). Utiliser la regex `password:` ou relancer sans `-l`. |
| `await_state` timeout sur `connected` | Vérifier `connection_status` (bannière, erreur réseau, clé d'hôte non acceptée). |
| `curl` introuvable en PowerShell 5.1 | `curl` est un alias : utiliser `curl.exe`. |
| `protocolVersion` différente de `2025-06-18` | Le serveur a négocié une autre version ; les scripts l'affichent. Vérifier la compatibilité du client. |

---

## Références

- [`MCP_PILOT.md`](MCP_PILOT.md) — spécification du serveur MCP embarqué.
- [`REA_EVIDENCE.md`](REA_EVIDENCE.md) — formats observés dans `putty.exe`
  (prompts `login as: `, `%s@%s's password: `, messages d'erreur).
- [`PUTTY_FEATURE_SPEC.md`](PUTTY_FEATURE_SPEC.md) — spécification produit.
