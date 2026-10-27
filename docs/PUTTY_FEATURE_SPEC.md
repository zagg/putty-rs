# Spécification fonctionnelle — clone Rust de PuTTY 0.83 (Windows x64)

Cible de référence : `putty.exe` 0.83 x64, sha256 `12498d4e…47d0366`, analysé sous Ghidra.
But : spécification **priorisée** et **testable** pour un développeur Rust, pas un manuel utilisateur.

## 0. Conventions

| Tag | Signification |
|---|---|
| `[REA]` | Chaîne / comportement **observé dans le binaire** (preuve REA). Fait. |
| `[DOC]` | Documenté dans le manuel officiel 0.83 (Chapter2/3/4), **non revu statiquement** dans le binaire ici. |
| `[REA+DOC]` | Les deux sources concordent. |
| `[SUPPOSE]` | Inférence (source amont connue, valeur historique) — **à vérifier** avant implémentation. |

Contraintes d'architecture imposées par le binaire (à respecter ou à documenter comme écart) :

| Contrainte | Preuve | Conséquence Rust |
|---|---|---|
| `ws2_32` **non** importée statiquement | `[REA]` | Winsock chargé à l'exécution (`LoadLibrary`/`GetProcAddress`). En Rust, lier `windows-sys` en statique est fonctionnellement OK mais **diverge** du binaire : décision à tracer. |
| `user32`, `shcore`, `winmm` partiellement dynamiques | `[REA]` : `FlashWindowEx`, `ToUnicodeEx`, `PlaySoundW`, `GetDpiForMonitor`, `MonitorFromPoint` via `GetProcAddress` | Résolution paresseuse + repli si absent (Windows ancien). |
| DPI `PerMonitorV2`, Common Controls 6 | `[REA]` (manifeste) | `SetProcessDpiAwarenessContext` + manifeste embarqué. |
| Copyright « 1997-2025 Simon Tatham », licence MIT | `[REA]` | Conserver l'attribution, ne pas copier le code (réécriture). |

Nommage des classes de fenêtres à réutiliser tel quel (compatibilité tests/outils) :
`PuTTYConfigBox`, `PuTTYTimerWindow`, `PuTTYHostKeyDialog`, `PuTTYHostKeyMoreInfo`, `PuTTY: hidden timing window` `[REA]`.

---

## 1. Options de ligne de commande

Syntaxe générale `[DOC]` : `putty.exe [options] [user@]host` ; `putty.exe -load "session"` ; `putty.exe telnet://host[:port]/` ; `@session` (déprécié).

| Option | Arg. | Effet | Statut | Prio |
|---|---|---|---|---|
| `[user@]host` | — | Connexion directe, réglages = « Default Settings » | `[DOC]` | P0 |
| `-ssh` / `-telnet` / `-rlogin` / `-supdup` / `-raw` / `-serial` / `-ssh-connection` | — | Force le protocole | `[DOC]` | P0 (ssh/raw), P1 (telnet), P2 (autres) |
| `telnet://host[:port]/` | URL | Raccourci telnet (gestionnaire d'URL) | `[DOC]` | P1 |
| `-load <nom>` | session | Charge une session sauvegardée | `[DOC]` | P1 |
| `@<nom>` | session | Idem `-load`, forme dépréciée, `@` en 1er caractère | `[DOC]` | P2 |
| `-cleanup` | — | Supprime entrées registre + `PUTTY.RND` + jump list, **après confirmation** | `[DOC]` + `[REA]` (clés, `\PUTTY.RND`, `Jumplist`) | P2 |
| `-v` | — | Verbosité (Event Log) | `[DOC]` | P1 |
| `-l <user>` | login | Auto-login username | `[DOC]` | P0 |
| `-P <port>` | port | Port distant ; écrase le défaut protocole | `[DOC]` + `[REA]` (`PortNumber`) | P0 |
| `-pw <pass>` / `-pwfile <f>` | secret | Mot de passe (SSH only) ; `-pwfile` = 1re ligne du fichier | `[DOC]` | P0 |
| `-i <key.ppk>` | fichier | Clé privée PPK (ou clé publique si Pageant) | `[DOC]` + `[REA]` (format PPK) | P0 (PPK P2) |
| `-cert <f>` | fichier | Certificat associé à la clé | `[DOC]` | P2 |
| `-agent` / `-noagent` | — | Auth via Pageant | `[DOC]` | P2 |
| `-A` / `-a` | — | Agent forwarding on/off | `[DOC]` + `[REA]` (`config-ssh-agentfwd`) | P2 |
| `-X` / `-x` | — | X11 forwarding on/off | `[DOC]` + `[REA]` (`config-ssh-x11`) | P2 |
| `-t` / `-T` | — | Force / interdit l'allocation de pty | `[DOC]` + `[REA]` (`config-ssh-pty`) | P0 |
| `-N` | — | Pas de shell ni commande (SSH-2) | `[DOC]` | P1 |
| `-nc <hôte:port>` | cible | Remplace le shell par une connexion réseau distante | `[DOC]` | P2 |
| `-m <fichier>` | fichier | Commande distante lue dans un fichier | `[DOC]` | P1 |
| `-C` | — | Compression SSH | `[DOC]` + `[REA]` (`config-ssh-comp`) | P2 |
| `-1` / `-2` | — | Version SSH forcée (pas de repli auto) | `[DOC]` + `[REA]` (`config-ssh-prot`) | P2 (SSH1) |
| `-4` / `-6` | — | Famille IP forcée | `[DOC]` | P1 |
| `-L` / `-R` / `-D` | spec | Redirections locales / distantes / SOCKS dynamique | `[DOC]` + `[REA]` (`config-ssh-portfwd`, `proxy/socks5.c`) | P2 |
| `-loghost <nom[:port]>` | nom | Nom logique (cache clé d'hôte, titre, prompt) | `[DOC]` | P1 |
| `-hostkey <empreinte\|blob>` | clé | Clé d'hôte attendue, répétable | `[DOC]` + `[REA]` (`SshHostKeys`) | P1 |
| `-no-trivial-auth` | — | Déconnexion si auth réussie sans preuve | `[DOC]` | P1 |
| `-sercfg <liste>` | csv | Baud, bits, stop, parité, flow (ex. `19200,8,n,1,N`) | `[DOC]` + `[REA]` (`config-serial-*`) | P2 |
| `-sessionlog <f>` | fichier | Log « All session output » | `[DOC]` | P1 |
| `-sshlog <f>` | fichier | Log « SSH packets » | `[DOC]` + `[REA]` | P1 |
| `-sshrawlog <f>` | fichier | Log « SSH packets and raw data » | `[DOC]` + `[REA]` | P1 |
| `-logoverwrite` / `-logappend` | — | Écrase / ajoute si le fichier existe | `[DOC]` | P1 |
| `-proxycmd <cmd>` | commande | Proxy « Local » (commandes locales, `\\` pour `\`) | `[DOC]` | P2 |
| `-restrict-acl` | — | Restreint l'ACL du processus, propagé aux sous-processus | `[DOC]` | P2 |
| `-host-ca` | — | Ouvre la config des CA d'hôtes puis quitte | `[DOC]` + `[REA]` (`SshHostCAs`) | P2 |
| `-legacy-stdio-prompts`, `-legacy-charset-handling` | — | Compat 0.81 (outils console / prompts) | `[DOC]` | P2 |
| `-pgpfp` | — | Affiche les empreintes PGP puis quitte | `[DOC]` | P2 |

Erreurs de ligne de commande : boîte de titre `"%s Command Line Error"` préfixée `PuTTY`, idem `"%s Fatal Error"` / `"%s Internal Error"` `[REA]`. Une option inconnue ⇒ message + sortie **sans** ouvrir de session (testable).

---

## 2. Valeurs par défaut

| Paramètre | Défaut | Source |
|---|---|---|
| Protocole | SSH | `[DOC]` |
| Ports par protocole | SSH 22, Telnet 23, Rlogin 513, SUPDUP 95, Raw 0 (à saisir) | `[DOC]` (sauf Raw=0 : `[SUPPOSE]`) |
| Taille terminal | 80 × 24 | `[SUPPOSE]` (non explicite dans la doc 0.83 ; valeur de référence) |
| Scrollback | 2000 lignes | `[DOC]` (3.1.2) |
| `TERM` | `xterm` | `[DOC]` (4.15.3) + `[REA]` (chaîne « xterm ») |
| Answerback (^E) | `PuTTY` | `[DOC]` (4.3.7) + `[REA]` (`Answerback`, libellé « Answerback to ^E: ») |
| Jeu de caractères distant | UTF-8 | `[DOC]` (4.10.1) |
| Police | monospace (famille exacte selon version) | `[SUPPOSE]` — voir `Font`/`FontVTMode` `[REA]` |
| Taille de police / mode VT | 10 pt ; `FontVTMode` = dessin de lignes Unicode | `[SUPPOSE]` |
| Couleur 256 | activée | `[SUPPOSE]` (`Xterm256Colour` `[REA]`) |
| Souris | mode xterm autorisé | `[SUPPOSE]` (`MouseIsXterm` `[REA]`) |
| Cloche | son système Windows ; surcharge : > 5 cloches / 2 s ⇒ muet, réarmement après 5 s de silence | `[DOC]` (4.5) |
| Indication taskbar sur cloche | désactivée | `[DOC]` |
| Fermeture fenêtre en fin de session | « Only on clean exit » | `[DOC]` |
| Titre fenêtre | `<hôte> - PuTTY` | `[DOC]` (4.9.1) |
| Log | aucun (`LogType=0`), en-tête inclus, flush fréquent, « omit passwords » activé, « omit session data » désactivé | `[DOC]` (4.2) |
| Nom de fichier log par défaut | `putty.log` | `[REA]` (non documenté) |
| Comportement si log existant | demander à l'utilisateur | `[DOC]` |
| Proxy | None | `[DOC]` |
| Famille IP | Auto (IPv6 si dispo, sinon IPv4) | `[DOC]` |
| Nagle | désactivé | `[DOC]` (4.14.2) |
| TCP keepalive | désactivé ; keepalive applicatif = 0 s (off) | `[DOC]` |
| SSH | version 2, compression off, pty alloué, Pageant on, keyboard-interactive on, X11 off, agent fwd off, bannière pré-auth affichée, changement d'utilisateur refusé | `[DOC]` |
| Rekey | 60 min / 1 Gio | `[SUPPOSE]` (doc : « recommandé ≤ 60 min », « ≤ 1 Go ») |
| Ordre clés d'hôte | Ed25519, Ed448, ECDSA, DSA, RSA | `[DOC]` (4.19.1) |
| Vitesse terminal annoncée | `38400,38400` | `[SUPPOSE]` |
| Backspace | `^?` (127) | `[DOC]` (recommandation 4.4.1) + `[SUPPOSE]` (défaut effectif) |
| Telnet | négociation **active**, Return = Telnet New Line (`config-telnetnl`), touches spéciales off (`config-telnetkey`) | `[DOC]` (4.30) + `[REA]` (noms) |
| Sérial | COM1, 9600, 8 bits, 1 stop, parité none, flow XON/XOFF | `[SUPPOSE]` |
| SUPDUP | location « The Internet », ASCII étendu `None`, `**MORE**` on, scrolling on | `[DOC]` |

---

## 3. Schéma de registre

Racine : `HKCU\Software\SimonTatham\PuTTY` `[REA+DOC]`. Sous-clés `Sessions`, `SshHostKeys`, `SshHostCAs`, `Jumplist` `[REA]`. AppUserModelID `SimonTatham.PuTTY` `[REA]`.

### 3.1 Valeurs confirmées dans le binaire

| Valeur | Type | Rôle | Statut |
|---|---|---|---|
| `TerminalType` | REG_SZ | Chaîne `TERM` envoyée | `[REA]` |
| `Answerback` | REG_SZ | Réponse à ^E (défaut « PuTTY ») | `[REA]` |
| `ScrollbackLines` | REG_DWORD | Taille du scrollback | `[REA]` |
| `PortNumber` | REG_DWORD | Port de connexion | `[REA]` |
| `FontVTMode` | REG_DWORD | Mode de dessin des lignes/jeu de police | `[REA]` |
| `Xterm256Colour` | REG_DWORD (bool) | Autorise le mode 256 couleurs | `[REA]` |
| `MouseIsXterm` | REG_DWORD (enum) | Mode de reporting souris (xterm vs Windows) | `[REA]` |
| `RandSeedFile` | REG_SZ | Chemin du fichier de graine (`\PUTTY.RND`) | `[DOC]` (4.33) + `[REA]` (chaîne `\PUTTY.RND`) |

### 3.2 Schéma complet attendu (noms à vérifier)

Toutes les valeurs ci-dessous sont **à confirmer** par capture registre (lancer PuTTY, sauver une session, `reg export`). Type `D`=DWORD, `S`=SZ, `B`=binaire.

| Valeur | Type | Rôle | Statut |
|---|---|---|---|
| `HostName` | S | Hôte cible | à vérifier (`Host` possible) |
| `Protocol` | D | 0=raw,1=telnet,2=rlogin,3=ssh,4=serial,5=supdup,… (enum à figer) | à vérifier |
| `PortNumber` | D | Port | `[REA]` |
| `UserName` | S | Auto-login | à vérifier |
| `PublicKeyFile` | S | Clé privée | à vérifier |
| `DetachedCertificate` | S | Certificat | à vérifier |
| `TerminalType` | S | `TERM` | `[REA]` |
| `TerminalSpeed` | S | `38400,38400` | à vérifier |
| `Answerback` | S | Réponse ^E | `[REA]` |
| `ScrollbackLines` | D | Scrollback | `[REA]` |
| `Font` | S | Nom de police | à vérifier |
| `FontHeight` / `FontCharSet` | D | Taille / charset | à vérifier |
| `FontVTMode` | D | Mode lignes | `[REA]` |
| `Xterm256Colour` | D | 256 couleurs | `[REA]` |
| `MouseIsXterm` | D | Souris xterm | `[REA]` |
| `LogType` | D | 0=none,1=printable,2=all,3=ssh,4=ssh+raw | à vérifier |
| `LogFileName` | S | Chemin (substitutions `&Y&M&D&T&H&P`) | `[DOC]` |
| `LogFileExists` | D | 0=ask,1=overwrite,2=append | à vérifier |
| `LogFlush`, `LogHeader`, `LogOmitPasswords`, `LogOmitData` | D | Options de log | `[DOC]` (sémantique) |
| `ProxyType`/`ProxyMethod` | D | 0=none,1=HTTP,2=SOCKS4,3=SOCKS5,4=Telnet,5=Local,6..=SSH | à vérifier |
| `ProxyHost`, `ProxyPort`, `ProxyUsername`, `ProxyPassword`, `ProxyCommand` | S/D | Proxy | à vérifier |
| `PingInterval` | D | Keepalive applicatif (s) | à vérifier |
| `TCPNoDelay`, `TCPKeepalives` | D | Options TCP | `[DOC]` (sémantique) |
| `AddressFamily` | D | 0=auto,1=v4,2=v6 | à vérifier |
| `WinTitle` | S | Titre forcé | `[DOC]` |
| `WinColumns`, `WinRows` | D | Taille initiale | à vérifier |
| `SshProt`, `SshCompression`, `SshPty`, `SshNoShell` | D | Options SSH | à vérifier (cf. `config-ssh-*` `[REA]`) |
| `SshBanner`, `SshKi`, `SshNoAuth`, `SshChangeUser`, `SshAgentFwd`, `SshX11` | D | Options auth/forwarding | à vérifier (cf. `config-ssh-*` `[REA]`) |
| `SshKexOrder`, `SshHostKeyOrder`, `SshCipherOrder` | S | Listes d'algorithmes | à vérifier (cf. `config-ssh-kex-order`, `-hostkey-order`, `-encryption` `[REA]`) |
| `SshBug*` | D | Contournements serveurs bogués | à vérifier (cf. `config-ssh-bug-*` `[REA]`) |
| `SerialLine`, `SerialSpeed`, `SerialDataBits`, `SerialStopBits`, `SerialParity`, `SerialFlow` | S/D | Sérial | à vérifier (cf. `config-serial-*` `[REA]`) |
| `TelnetKey`, `TelnetNl` | D | Options Telnet | à vérifier (cf. `config-telnetkey`, `config-telnetnl` `[REA]`) |

Sessions : `Sessions\<nom>` contient l'ensemble des valeurs ; la session spéciale s'appelle `Default Settings` `[DOC]`. Un nom de session ne peut pas contenir `\`.
`SshHostKeys` : nom de valeur = `<hôte>:<port>` (ou `<hôte>` pour le port par défaut), contenu = clé publique SSH-2 `[SUPPOSE]` (format exact à vérifier).
`SshHostCAs` : CA acceptées par hôte `[REA]` (nom), format à vérifier.
`Jumplist` : sessions récemment lancées ; saut de liste `putty %s&%p:%u`, catégorie « Connect to PuTTY session ' » `[REA]`.

---

## 4. Sous-systèmes par priorité

### P0 — socle utilisable (fenêtre + VT + TCP + SSH)

#### P0.1 Fenêtre, boucle d'événements, configuration

| Élément | Attendu testable | Source |
|---|---|---|
| Fenêtre terminal | Créée avec la classe `PuTTYConfigBox` pour la config ; géométrie 80×24 cellules ; fond noir par défaut | `[REA]` + `[DOC]` |
| Minuterie | Fenêtre cachée `PuTTYTimerWindow` / `PuTTY: hidden timing window` ; blink du curseur, cloche, timeouts réseau | `[REA]` |
| Redimensionnement | Défaut = « change rows/columns » (police fixe) ; envoie un `window-change` SSH et une négociation NAWS Telnet | `[DOC]` |
| Titre initial | `<hôte> - PuTTY` (ex. `server1.example.com - PuTTY`) | `[DOC]` |
| Titre après login | Préfixe `user@host` (`user@host - PuTTY`) | `[SUPPOSE]` — à valider |
| Fermeture | `Alt+F4` ferme (ou avertit) ; « Warn before closing » ; « Only on clean exit » | `[DOC]` |
| Menu système | `Alt+Space`, clic gauche sur l'icône, clic droit sur la barre de titre, `Ctrl`+clic droit = menu contextuel | `[DOC]` |
| Plein écran | `Alt+Enter` (si activé) ; bordures/titre/scrollbar masqués | `[DOC]` |
| DPI | Per-Monitor V2 : redimensionnement sans flou, `GetDpiForMonitor`/`MonitorFromPoint` résolus dynamiquement | `[REA]` |
| Copier/coller | Sélection = copie auto ; clic droit = colle ; `Shift+Ins` colle ; `Ctrl+Ins` copie ; `Ctrl+Shift+C/V` (V off par défaut) | `[DOC]` (3.1.1, 4.11) |
| Sélection | Double-clic = mot (classes 0/1/2), triple-clic = ligne, `Alt`+glisser = rectangle | `[DOC]` (3.1.1, 4.12.1) |
| Collage | Filtre les caractères de contrôle sauf `\n`, `\t`, `\b`, `DEL` (sauf option « permit control characters ») | `[DOC]` (4.11.5) |
| Scrollback | Molette + `Shift+PgUp/PgDn`, `Ctrl+PgUp/PgDn`, `Ctrl+Shift+PgUp/PgDn` ; retour au live à la réception de données | `[DOC]` (3.1.2) |
| Event Log | Fenêtre dédiée, sélection + bouton Copy | `[DOC]` (3.1.3.1) |
| Curseur | Bloc/underline/barre, clignotant ; bloc vide si fenêtre inactive | `[DOC]` |

#### P0.2 Émulation terminal VT/xterm

| Élément | Attendu testable | Source |
|---|---|---|
| Parseur | États ground / ESC / CSI / OSC / DCS ; UTF-8 par défaut ; largeur CJK optionnelle | `[SUPPOSE]` (structure) + `[DOC]` (charset) |
| Curseur | `CUU/CUD/CUF/CUB`, `CUP/HVP`, `CNL/CPL`, `CHA/VPA`, save/restore (`DECSC/DECRC`, `ESC 7/8`) | `[SUPPOSE]` (jeu xterm standard) |
| Effacement | `ED` (0/1/2/3), `EL` (0/1/2), `ECH`, `ICH/DCH`, `IL/DL` ; couleur d'effacement = fond courant si option activée | `[DOC]` (4.3.5) |
| SGR | 8/16 couleurs, brillant, inverse, souligné, clignotant (désactivable ⇒ fond gras), 256 couleurs, 24 bits | `[DOC]` (4.13) |
| Zone de défilement | `DECSTBM`, `DECOM` (origin mode), `IND/RI/NEL`, autowrap `DECAWM` | `[DOC]` (4.3.1/4.3.2) |
| Écran alterné | `?1049`/`?47`/`?1047` ; « push erased text into scrollback » activé par défaut | `[DOC]` (4.6.4, 4.7.4) |
| Titre | `OSC 0` / `OSC 2` ⇒ titre fenêtre ; `OSC 1` = titre d'icône (ignoré sauf option) ; « disable remote title changing » existe | `[DOC]` (4.9.1, 4.6.5) |
| Réponse titre | Requête titre : défaut = **aucune réponse** (« None »), options « Empty string » / « Window title » | `[DOC]` (4.6.6) |
| Answerback | Réception `^E` (0x05) ⇒ envoi de la chaîne `PuTTY` (configurable, vide possible) | `[DOC]` + `[REA]` |
| Cloche | `BEL` (0x07) ⇒ son système ; style visuel = flash blanc de la fenêtre ; surcharge 5/2 s ; toute frappe réarme | `[DOC]` (4.5) |
| Souris | Reporting xterm (`?1000/1002/1003/1006`) si `MouseIsXterm` ; `Shift` rend la sélection locale | `[DOC]` (4.6.2) + `[REA]` |
| Bracketed paste | `?2004` respecté ; désactivable | `[DOC]` (4.6.12) |
| Lignes de dessin | Mode `DEC` line drawing + mapping Unicode / OEM / poor-man's | `[DOC]` (4.10.4) |
| Backspace destructif | `0x7F` reçu = recule + efface (désactivable) | `[DOC]` (4.6.8) |
| Jeu de caractères distant | `ESC ( B`, `ESC ( 0`, désignation G0/G1, charset configurable par le serveur (désactivable) | `[DOC]` (4.6.9) |

#### P0.3 Backend TCP nu (protocole Raw)

| Élément | Attendu testable | Source |
|---|---|---|
| Connexion | `-raw hôte -P 8080` : octets tapés envoyés tels quels, octets reçus affichés tels quels, **aucune** négociation | `[DOC]` (3.7) |
| Port | Raw n'a pas de port par défaut ⇒ l'utilisateur doit le saisir | `[DOC]` |
| Échec | Boîte `PuTTY Fatal Error`, texte `Unable to open connection to\n%s\n%s` (hôte + raison) | `[REA]` |
| Options réseau | Nagle off, TCP keepalive off, famille IP auto, keepalive applicatif sans objet en Raw | `[DOC]` |
| DNS | Résolution locale ; échec ⇒ même boîte d'erreur, pas de fuite de session | `[DOC]` + `[SUPPOSE]` |

#### P0.4 SSH-2 (kex, auth, channel shell)

| Élément | Attendu testable | Source |
|---|---|---|
| Version | Bannière client `SSH-2.0-PuTTY_Release_0.83` (ou équivalent clone), rejet si version ≠ 2 quand `-2` | `[SUPPOSE]` |
| Kex | Curve25519, ECDH (nistp256/384/521), DH group14/16/18 + GEX, hybrides PQ (NTRU/ML-KEM) | `[DOC]` (4.18.1) |
| Clés d'hôte | Ed25519, Ed448, ECDSA, RSA, DSA ; préférence aux clés déjà connues | `[DOC]` (4.19) |
| Chiffrement | AEAD (AES-GCM, ChaCha20-Poly1305) + modes CTR + HMAC ; liste ordonnée configurable | `[DOC]` (4.20) |
| Dialogue clé d'hôte | Classes `PuTTYHostKeyDialog` / `PuTTYHostKeyMoreInfo` ; texte 0.83 : « The host key is not cached for this server: … », empreinte `SHA256:…`, boutons **Accept** / **Connect Once** / **Cancel**, lien **More info…** (empreinte MD5 + clé complète) | `[REA]` + `[DOC]` (2.2) |
| Stockage | Accept ⇒ `SshHostKeys` ; clé différente ⇒ avertissement renforcé + abandon possible | `[DOC]` (2.2) |
| Auth | Ordre : none → publickey (Pageant/fichier) → keyboard-interactive → password ; GSSAPI optionnel | `[DOC]` (4.21) |
| Prompt login | `login as: ` dans la fenêtre terminal, précédé du sigil PuTTY (icône) infalsifiable | `[REA]` + `[DOC]` (2.3) |
| Bannière pré-auth | Affichée avant le prompt de mot de passe, supprimable | `[DOC]` (4.21.1) |
| Mot de passe | Non échoé, curseur immobile, envoi à `Return` seulement | `[DOC]` (2.3) |
| Canal | `session` + `pty-req` (`TERM=xterm`, 80×24, modes `ERASE`, `IUTF8`) + `shell` **ou** `exec` (`-m`) **ou** `subsystem` | `[DOC]` (4.17.1, 4.24) |
| `-N` | Aucun `shell`/`exec` ; la session reste ouverte pour les forwardings | `[DOC]` |
| Resize | `window-change` à chaque redimensionnement | `[DOC]` |
| Keepalive | Requête globale / `IGNORE` à intervalle `PingInterval` (0 = off) | `[DOC]` |
| Rekey | Temps et volume, rekey forcé via menu Special Commands | `[DOC]` (4.18.2) |
| Erreurs | `%s Fatal Error` / `%s Internal Error` ; « Failed to initialize COM subsystem » si COM indisponible | `[REA]` |

### P1 — fidélité produit

| Sous-système | Attendu testable | Source |
|---|---|---|
| **Telnet IAC** | Négociation **active** au démarrage ; `IAC WILL/WONT/DO/DONT` ; options ECHO, SGA, TTYPE (`xterm`), NAWS (taille), NEW_ENVIRON/OLD_ENVIRON (ambiguïté configurable) ; `IAC IAC` = 0xFF littéral ; sous-négociation `SB…SE` ignorée proprement | `[DOC]` (4.30) |
| Telnet touches | Return = Telnet New Line par défaut (`config-telnetnl` `[REA]`) ; option « keyboard sends Telnet special commands » (Backspace/Ctrl-C/Ctrl-Z) | `[DOC]` + `[REA]` |
| Telnet commandes | Menu Special Commands : Are You There, Break, Synch, Erase Character/Line, Go Ahead, NOP, Abort Process/Output, Interrupt, Suspend, EOR, EOF ; `Ctrl+Break` = break | `[DOC]` (3.1.3.2) |
| **Registre / sessions** | Liste des `Sessions\*` triée, `Default Settings` en tête ; Save / Load / Delete ; double-clic lance ; le chargement applique **toutes** les valeurs ; `-cleanup` supprime tout après confirmation | `[DOC]` (4.1.2) + `[REA]` |
| **Journalisation** | Modes none / printable / all / ssh / ssh+raw ; en-tête exact `=~=~=~=~=~=~=~=~=~=~=~= PuTTY log %s =~=~=~=~=~=~=~=~=~=~=~=\r\n` ; fichier par défaut `putty.log` ; substitutions `&Y &M &D &T &H &P` (insensibles à la casse) ; question si fichier existant (sauf `-logoverwrite`/`-logappend`) ; omission des mots de passe activée par défaut | `[REA]` + `[DOC]` (4.2) |
| **Titre dynamique** | `printf '\033]0;hello\007'` ⇒ titre `hello - PuTTY` (suffixe conservé) ; `OSC 1` n'affecte que l'icône ; désactivable ; requête de titre sans réponse par défaut | `[DOC]` (4.9.1, 4.6.5/6) |
| **Cloche visuelle** | Option « Visual bell » ⇒ flash blanc ; « Steady »/« Flashing » ⇒ `FlashWindowEx` sur la taskbar si la fenêtre n'a pas le focus ; surcharge 5/2 s | `[DOC]` + `[REA]` (`FlashWindowEx`) |
| Clavier | Backspace `^H`/`^?` (défaut `^?`), `Shift+Backspace` = l'autre ; Home/End xterm vs rxvt ; F1–F12 modes (ESC[n~, Linux, Xterm R6, VT400, VT100+, SCO, Xterm 216) ; flèches « Ctrl toggles app mode » ; `AltGr`/Compose ; `Ctrl+Alt ≠ AltGr` | `[DOC]` (4.4) |
| Menu / duplication | New Session, Duplicate Session, Restart Session, Saved Sessions, Change Settings (panneau réduit), Copy All to Clipboard, Clear Scrollback, Reset Terminal, Full Screen | `[DOC]` (3.1.3) |
| Traduction | UTF-8 par défaut, jeux ISO-8859-*, Win125x, CP437, code page manuelle | `[DOC]` (4.10.1) |
| Collage bracketed | `?2004` : `ESC[200~` … `ESC[201~` autour du collage | `[DOC]` (4.6.12) |

### P2 — extensions

| Sous-système | Attendu testable | Source |
|---|---|---|
| **X11 forwarding** | `-X` ⇒ requête `x11-req`, `DISPLAY` distant = `<hôte>:10.0`, cookies `MIT-MAGIC-COOKIE-1` (défaut) ou `XDM-AUTHORIZATION-1`, `.Xauthority` local optionnel ; Event Log « Requesting X11 forwarding » / « X11 forwarding enabled » | `[DOC]` (3.4, 4.25) + `[REA]` (`config-ssh-x11`) |
| **Agent forwarding** | `-A` ⇒ `auth-agent-req@openssh.com`, canal `auth-agent@openssh.com` vers Pageant | `[DOC]` (4.21.7) + `[REA]` (`config-ssh-agentfwd`) |
| **Port forwarding** | `-L/-R/-D` ; `direct-tcpip` / `tcpip-forward` ; SOCKS dynamique 4/4A/5 (TCP seulement) ; IPv6 `[::1]:port` ; « local ports accept connections from other hosts » | `[DOC]` (3.5, 4.26) + `[REA]` (`config-ssh-portfwd`, `proxy/socks5.c`) |
| **Proxies** | None (défaut), HTTP CONNECT, SOCKS4, SOCKS5, Telnet (`connect hôte port`), Local (`-proxycmd`), SSH-jump (port forwarding / commande / sous-système) ; liste d'exclusion ; DNS local ou distant ; log proxy vers Event Log | `[DOC]` (4.16) + `[REA]` |
| **Sérial** | COM1..n, vitesse, 7/8 bits, 1/1.5/2 stop, parité n/o/e/m/s, flow N/X/R/D ; pas de déconnexion détectable ⇒ reste ouvert jusqu'à fermeture ; Special Commands = Break | `[DOC]` (3.6, 4.29) + `[REA]` (`config-serial-*`) |
| **SSH-1** | `-1`, clé RSA1, TIS/CryptoCard, pas de repli automatique 1↔2 | `[DOC]` (4.17.4, 4.21.5) + `[REA]` (`config-ssh-prot`) |
| **Partage de connexion** | Upstream/downstream ; pipe `\\.\pipe\putty-connshare`, mutex `Local\putty-connshare-mutex`, ID `SSHCONNECTION@putty.projects.tartarus.org-2.0-` ; l'upstream survit à la fermeture de son terminal tant qu'un downstream existe ; PSCP/PSFTP jamais upstream | `[REA]` + `[DOC]` (4.17.5) |
| **PPK / Pageant** | Lecture `PuTTY-User-Key-File-`, MAC `putty-private-key-file-mac-key`, refus « PuTTY key format too new » ; passphrase demandée | `[REA]` |
| Rlogin / SUPDUP | Rlogin (port 513, `.rhosts`, local username) ; SUPDUP (port 95, location, ASCII étendu, `**MORE**`, scrolling) | `[DOC]` (3.9, 3.10, 4.31, 4.32) |
| Divers | `-restrict-acl`, `-host-ca`, `-pgpfp`, `-cleanup`, GSSAPI, plugin d'auth, impression distante, palette logique | `[DOC]` |

---

## 5. Tests d'acceptation minimaux (P0/P1)

| # | Test | Attendu |
|---|---|---|
| 1 | `putty-rs -raw localhost -P 12345` avec `nc -l` | Octets bidirectionnels identiques, aucun préambule |
| 2 | `putty-rs -raw inexistant.invalid -P 1` | Boîte `PuTTY Fatal Error` + « Unable to open connection to … » |
| 3 | `printf '\033[31mX\033[0m\n'` | `X` rouge, retour à la couleur par défaut |
| 4 | `printf '\033]0;hello\007'` | Titre `hello - PuTTY` |
| 5 | `printf '\033[?1049h'` puis `\033[?1049l` | Écran alterné, contenu précédent restauré |
| 6 | `printf '\007'` | Bip système ; 6 bips en < 2 s ⇒ silence |
| 7 | Redimensionner la fenêtre en SSH | `window-change` envoyé, `stty size` distant cohérent |
| 8 | Première connexion SSH | Dialogue de clé d'hôte (3 boutons) ; Accept ⇒ valeur créée dans `SshHostKeys` |
| 9 | Reconnexion avec clé changée | Avertissement renforcé, connexion bloquée par défaut |
| 10 | `-l bob` sur serveur sans clé | Prompt `login as:` **non** affiché, `bob` utilisé |
| 11 | `-sshlog f.log` | En-tête `=~=~=…= PuTTY log …` puis paquets SSH lisibles |
| 12 | `putty-rs -load "ma session"` | Tous les réglages sauvegardés appliqués (port, TERM, police, log) |
| 13 | `putty-rs -cleanup` | Confirmation puis suppression `Sessions`/`SshHostKeys`/`SshHostCAs`/`Jumplist`/`PUTTY.RND` |
| 14 | `putty-rs -telnet hôte` | Négociation active (TTYPE/NAWS) visible dans un serveur de test |
| 15 | `putty-rs -bogus` | `PuTTY Command Line Error`, code de sortie ≠ 0, pas de fenêtre terminal |

---

## 6. Rapport de production

**Fichier écrit** : `putty-rs/docs/PUTTY_FEATURE_SPEC.md` (seul fichier touché ; aucun build lancé).

**Confirmé `[REA]` (fait)** : clés et sous-clés de registre ; AppUserModelID ; jump list ; 5 classes de fenêtres ; formats de titres d'erreur ; texte « Unable to open connection to » ; « Failed to initialize COM subsystem » ; prompt `login as: ` ; pipe/mutex/ID de connshare ; préfixes PPK + MAC + « key format too new » ; en-tête et nom de log par défaut ; `\PUTTY.RND` ; 7 noms de valeurs de config (`TerminalType`, `Answerback`, `ScrollbackLines`, `PortNumber`, `FontVTMode`, `Xterm256Colour`, `MouseIsXterm`) ; identifiants `config-ssh-*`, `config-serial-*`, `config-telnet*`, `-sshlog`, `-sshrawlog` ; chargement dynamique Winsock et API user32/shcore/winmm ; manifeste DPI/Common Controls ; MIT + copyright.

**`[DOC]` non revu statiquement** : liste complète des options CLI (seuls `-sshlog`/`-sshrawlog` sont attestés), valeurs par défaut (ports, 2000 lignes, TERM, cloche, close-on-exit, SSH-2, rekey, ordre des algorithmes), texte du dialogue de clé d'hôte et boutons, sémantique Telnet (négociation active, New Line), substitutions `&Y&M&D&T&H&P`, raccourcis clavier/souris, comportements X11/forwarding, panneaux de configuration.

**`[SUPPOSE]` à lever en priorité** : 80×24 ; police et taille par défaut ; `38400,38400` ; backspace `^?` ; défauts sérial (COM1/9600/8N1/XON-XOFF) ; bannière de version SSH exacte ; format des valeurs `SshHostKeys`/`SshHostCAs` ; mapping `config-ssh-*` → noms de valeurs registre.

**Questions ouvertes**
1. Nom exact de la valeur d'hôte (`HostName` vs `Host`) et enum `Protocol` — à obtenir par `reg export` d'une session sauvegardée.
2. Titre après login : PuTTY affiche-t-il réellement `user@host - PuTTY`, et le titre distant est-il encadré de parenthèses ? La doc 0.83 ne décrit que `<hôte> - PuTTY` et le remplacement par le titre distant.
3. `Ctrl+Alt+F12` : aucune trace dans la doc 0.83 ; seul `Ctrl+Break` (break) est documenté. Combo à confirmer par test sur le binaire, sinon à retirer de la spec.
4. OSC 52 (presse-papier distant) : support présent en 0.83 ? Non listé dans les preuves REA ni dans la doc lue.
5. `ws2_32` dynamique : conserver la fidélité (chargement runtime) ou lier statiquement en Rust ? Décision d'architecture à trancher (impact : pas de dépendance statique, messages d'erreur différents si Winsock absent).
6. Périmètre PPK/Pageant : outil séparé (`puttygen`/`pageant`) ou intégré au clone ? Les chaînes PPK sont dans `putty.exe` mais l'UI de génération ne l'est pas.
7. `SshHostKeys` : clé = `hôte:port` en clair, valeur = blob binaire ou base64 ? À vérifier avant d'écrire le module de cache.
8. Faut-il reproduire les noms de classes Win32 à l'identique (compat outils tiers / tests UI) ou est-ce un détail interne ?
