# ANALYSIS_NOTES — putty-rs : hypothèses et preuves

Cible de référence : **PuTTY Release 0.83 x64**, `C:\Program Files\PuTTY\putty.exe`
sha256 `12498d4e4bf07747a9a52d6803d3211fd731ded6473b41cf4795ac56947d0366`
(**SHA-256 CONFIRMÉ** par `rea open_binary` le 2026-10-27 : identique à celui
fourni dans la consigne).

Sources de preuves utilisées :
1. **scan byte-brut** ASCII du fichier (offsets = offsets fichier, base 10) —
   fait ici en lecture seule, sans exécution ;
2. inventaire de chaînes REA/provider Ghidra — **partiel** sur ce binaire ;
3. `docs/REA_EVIDENCE.md` (campagne REA complémentaire) : inventaire Ghidra
   des noms de clés registre `[OBS]` + dump `reg query HKCU` `[RUNTIME]`.

Statuts : `[OBS]` observé dans le binaire · `[RUNTIME]` observé dans les
données écrites par PuTTY · `[DOC]` documentation PuTTY 0.83 · `[SUPPOSE]`
hypothèse · `[SIMPLIF]`/`[LIMITATION]` écart assumé.

---

## 1. Faits OBSERVÉS (corrigent ou confirment la consigne)

| # | Fait | Source | Effet dans putty-rs |
|---|------|--------|---------------------|
| 1 | `Software\SimonTatham\PuTTY\Sessions` @1095242 | scan | **corrige la consigne** : `…\PuSessions` n'existe pas → `config::SESSIONS_ROOT` |
| 2 | `Default Settings` @1095896 (VA 0x14010c6d8) | scan | `config::DEFAULT_SETTINGS` |
| 3 | `…\PuTTY\SshHostKeys` VA 0x14010bad0 | scan | cache clés d'hôte (à implémenter) |
| 4 | `Answerback to ^E:` @1145268 ; `Answerback` @1110911 | scan | answerback ↔ ENQ ; clé registre ✔ |
| 5 | `Answerback` défaut = **« PuTTY »** | `[RUNTIME]` REA_EVIDENCE §4 | `Config::default().answerback = "PuTTY"` (la consigne disait vide) |
| 6 | `ScrollbackLines` @1096409, défaut `[RUNTIME]` = 2000 | scan+RT | défaut 2000 ✔ |
| 7 | `TerminalType` @1117188, `[RUNTIME]` `xterm` | scan+RT | défaut `xterm` ✔ |
| 8 | `TerminalSpeed` `[RUNTIME]` = chaîne `38400,38400` | RT | lu comme chaîne « in,out » |
| 9 | **`TermWidth=80` / `TermHeight=24`** (PAS `WindowWidth/Height`) | `[OBS]`+RT | lecture/écriture sous ces noms |
| 10 | **`PortNumber=22`** (PAS `Port`) | `[OBS]`+RT | idem |
| 11 | **`BackspaceIsDelete`** (PAS `BackspaceKey`) ; session locale = 1 | `[OBS]`+RT | lu (`0=^H`, `1=DEL`) ; défaut garde ^H `[DOC]` — **contradiction session locale → [REA] sur `Default Settings` vierge** |
| 12 | **`Beep`/`BeepInd`/`BellOverload{N,T,S}`** (PAS `BellType`) | `[OBS]`+RT | cloche lue via `Beep`/`BeepInd`, repli legacy |
| 13 | `BlinkCur`, `CurType` | `[OBS]` | clignotement lu |
| 14 | `CloseOnExit=1` `[RUNTIME]` | RT | défaut `OnCleanExit` ✔ |
| 15 | `TCPNoDelay=1` `[RUNTIME]` | RT | nodelay défaut true ✔ |
| 16 | `ProxyPort=80` `[RUNTIME]` | RT | défaut port proxy 80 |
| 17 | `SerialLine=COM1`, `SerialSpeed=9600`, 8N1 `[RUNTIME]` | RT | défauts série ✔ |
| 18 | **Police : `Font=Courier New`** @1088111 ; `Consolas`/`Fixedsys` absents | scan | famille = Courier New ; **taille défaut doc 0.83 = 18 pt** (note `[RUNTIME]` REA_EVIDENCE §4 : « ce poste : 10 ; défaut doc : 18 ») → défaut 18 pt (conforme à la consigne) |
| 19 | `Colour0..21` : 0=fg, 1=fg gras, 2=bg, 3=bg gras, 4=texte curseur, 5=couleur curseur, **6..21 = ANSI par paires (normale,vive)** ; échantillons 8=`187,0,0`, 9=`255,85,85`, 20=`187,187,187`, 21=`255,255,255` | `[RUNTIME]` REA_EVIDENCE §6 | lecture `Colour0/2/5` + `Colour6..21` ; palette par défaut 187/85 (style VGA), slots non échantillonnés `[SUPPOSE]` |
| 20 | `unknown option "%s"`, `unexpected argument "%s"`, `option "%s" requires an argument` | scan + `[OBS]` §3 | messages d'erreur repris à l'identique |
| 21 | `You need to specify a destination address.` / `in the form "host.name:port"` | scan | message repris |
| 22 | `-pw`/`-sercfg` réservés (messages complets @1109483) | scan | contraintes implémentées |
| 23 | Chaînes CLI `[OBS]` : `-load`, `-pageant`, `-nopageant`, `-hostkey`, `-sshlog`, `-sshrawlog`, `-nc`, `-pwfile`, `-cleanup`, `-pgpfp`, `-host-ca`, `-demo-config-box`, `-sercfg`, `-ssh`(via table), `-telnet`, `-serial`, `-pw `, `-agent` | scan + §3 | toutes acceptées (valeurs lues ou ignorées) |
| 24 | Popup hôte inconnu : `The host key is not cached for this server:`, `Host key fingerprint is:`, `press "Accept".`, `WARNING - POTENTIAL SECURITY BREACH!` | scan + §2 | notre acceptation auto = `[SIMPLIF]` |
| 25 | `PuTTY-User-Key-File-` (PPK v1/v2/v3) | §5 | PPK non lu par russh `[LIMITATION]` |
| 26 | Prompts SSH : `login as: `, `%s@%s's password: ` | §5 | futures intégrations UI (pas de prompt console dans le squelette) |

## 2. Comportements `[SUPPOSE]` (à confronter aux observations REA)

### Ligne de commande (`cmdline.rs`)
1. `-title <t>` = alias de `-T <t>` (inventé, tolérant).
2. `-clear-avtcache` = désactive le titre dynamique distant (peut être lié à
   `RemoteQTitleAction` `[OBS]` dont la valeur par défaut `[RUNTIME]` = 1).
3. `-term <type>` : option non officiellement documentée (gardée tolérante).
4. `-cygterm`, `-no-antispoof`, `-old-geometry`, `-1/-2`, `-bc`, `-gssapi`,
   `-f` : acceptées-ignorées.
5. Deux formes `-opt` et `--opt` acceptées (mots-clés « réglage » stockés sans
   tiret dans le binaire ; parser CLI Ghidra = table sans strcmp → comportement
   exact à valider `[REA]`).
6. Aide : `--help/-h/-?` (équivalent fonctionnel de l'aide console réelle).

### Protocoles et ports
7. **raw = 10001** `[DOC]` ; chaîne « 10001 » absente (entier compilé) → `[REA]` GUI.
8. ssh=22 `[RUNTIME]` · telnet=23, rlogin=513 `[DOC]`.
9. Changement de protocole ⇒ reset port par défaut sauf `-P` explicite.
10. Série : le positionnel avec `-serial` = ligne COM ; `-sercfg` = config
    (débit…) — sémantique `[DOC]`, à valider en exécution comparée `[REA]`.

### Configuration / registre (`config.rs`)
11. Fusion `Default Settings` puis session `[DOC]` (non vérifiée ligne à ligne).
12. `FontHeight` : signe de la valeur (négatif = points ?) `[DOC][REA]` ; on
    accepte les deux.
13. Écrire ses propres valeurs seulement (`save`) sans reproduire tout le schéma
    PuTTY `[LIMITATION]`.
14. Titre fenêtre par défaut « host:port » (PuTTY : « user@host:port » via
    `WinNameAlways=1` `[RUNTIME]`) `[SUPPOSE]`.
15. Couleurs : slots ANSI 6..21 complets seulement partiellement échantillonnés
    → valeurs intermédiaires reconstituées `[SUPPOSE]`.

### Terminal (`terminal.rs`, `app.rs`)
16. ENQ reçu via `Callbacks::unhandled_control(b=5)` (mécanisme vt100 supposé ;
    l'association answerback↔^E est `[OBS]` par le libellé GUI).
17. Answerback renvoyé brut (CR/LF non interprétés) `[SUPPOSE]`.
18. BEL : flash jaune 300 ms `[SUPPOSE]` (PuTTY : FlashWindow + BellOverload
    N=5/T=2000/S=5000 `[RUNTIME]` — surcharge non implémentée `[LIMITATION]`).
19. Backspace ^H défaut `[DOC]` — mais session locale `[RUNTIME]`=DEL → arbitrage
    `[REA]`.
20. Shift+Backspace = DEL `[DOC]`.
21. Flèches normal/application via DECCKM ✔ ; Home/End en mode application
    (`ESC O H/F` vs `ESC [ 1~/4~`) `[SUPPOSE]` — chaînes `RXVTHomeEnd`/
    `LinuxFunctionKeys` `[OBS]` suggèrent plusieurs modes PuTTY.
22. F1-F12 codage xterm `[SUPPOSE]` (`NoApplicationKeys`, `NetHackKeypad` `[OBS]`
    = modes alternatifs non rendus).
23. Alt = préfixe ESC `[DOC partiel]` ; texte non ASCII en UTF-8 (PuTTY :
    `LineCodePage` `[OBS]`) `[SUPPOSE]`.
24. Entrée = CR `[DOC]` ; `TelnetRet=1` `[RUNTIME]` (spécificité telnet non
    rendue `[LIMITATION]`).
25. Collage : CR→LF, pas de dialogue d'avertissement CR/LF/TAB
    (`PasteControls` `[OBS]`) `[SUPPOSE]`.
26. Copier : relâchement souris = copie + trim trailing spaces `[DOC partiel]` ;
    **Entrée copie si sélection** = consigne, réel à confirmer `[REA]`
    (`MouseAutocopy=1` `[RUNTIME]` appuie le copier-automatique) ; Ctrl+C copie
    si sélection sinon ^C `[SUPPOSE]`.
27. Molette = scrollback, retour en bas à la frappe `[SUPPOSE]`
    (`ScrollOnKey`/`ScrollOnDisp` `[OBS]`, défauts `[REA]`).
28. Curseur bloc clignotant 500 ms `[SUPPOSE]` (`BlinkCur`/`CurType` `[OBS]`
    lus mais styles non rendus `[LIMITATION]`).
29. Resize demandé par le serveur (`CSI 8t`) journalisé sans effet `[SUPPOSE]`.

### Backends
30. **Telnet IAC** : WONT à tout DO (sauf STATUS), silence aux WILL, NAWS à
    notre initiative, `WILL NAWS` initial — tout `[SUPPOSE]`, trace réseau `[REA]`
    (PuTTY a `PassiveTelnet`, `TelnetKey` `[OBS]`).
31. NAWS RFC 1073 (col hi/lo, row hi/lo) ✔ structure ; sémantique = RFC.
32. **SSH** : acceptation auto clé d'hôte `[SIMPLIF]` (popup réelle `[OBS]` §24
    table 1) ; MOTD via bannière `[DOC]` ; username vide = `%USERNAME%`
    `[SUPPOSE]` (PuTTY : `login as:` + défaut Windows `[DOC]`) ; pas de Pageant
    (`TryAgent=1` `[RUNTIME]` → futur) ; PPK non lu `[LIMITATION]` ; tunnels/X11
    non câblés `[LIMITATION]` ; `SshBanner=1` `[RUNTIME]` = bannière affichée ✔.
33. **Raw** : TCP nu + `TCP_NODELAY` (`TCPNoDelay=1` `[RUNTIME]` ✔) ; resize
    ignoré `[SUPPOSE]`.
34. **Série** : `\\.\COMx` sans DCB ni timeouts `[LIMITATION]`.
35. rlogin = raw `[SUPPOSE]` (PoC : `Unrecognised protocol prefix` n'existe pas
    en rlogin).

## 3. Questions ouvertes pour la campagne REA

1. Aide console exacte (`putty.exe -?`) vs notre `usage()` (dont liste `--`).
2. `Default Settings` vierge : valeurs par défaut réellement écrites
   (`BackspaceIsDelete`, `Beep`, `BlinkCur`, `ScrollOnKey/OnDisp`, port Raw).
3. Trace réseau telnet : handshake réel (item 30).
4. ETW/strace `putty -serial` : DCB/timeouts (item 34).
5. Popup clé d'hôte : format d'empreinte exact (MD5 vs SHA256 ?) + contenu
   écrit dans `SshHostKeys`.
6. `RemoteQTitleAction=1` `[RUNTIME]` : mapping 0/1/2 et lien avec
   `-clear-avtcache` (item 2).
7. `TelnetTermType` réellement envoyé en telnet (`putty` supposé).
8. Sélection au clavier (shift+flèches) et « Entrée copie » (item 26).

---

*Généré le 2026-10-27. Les offsets byte-brut proviennent d'un scan ASCII du
fichier `C:\Program Files\PuTTY\putty.exe` (hachage vérifié par REA) ; les
références « § » renvoient à `docs/REA_EVIDENCE.md` (campagne REA
complémentaire : Ghidra + `reg query` [RUNTIME]).*
