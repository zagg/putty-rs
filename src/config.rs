//! Configuration de putty-rs, alignée sur le modèle de PuTTY 0.83 :
//! sessions stockées dans le registre, `HKCU\Software\SimonTatham\PuTTY\Sessions`,
//! avec un sous-arbre `Default Settings` servant de valeurs par défaut.
//!
//! PREUVES REA (putty.exe 0.83 x64, sha256 12498d4e…0366, scan byte-brut) :
//!   * offsets 1095242 / 0x14010c441 : chaîne `Software\SimonTatham\PuTTY\Sessions`
//!     → le chemin énoncé dans la consigne (`…\PuSessions`) est CORIGÉ ici par
//!     le chemin réellement observé.
//!   * offset 1095896 / 0x14010c6d8 : `Default Settings`.
//!   * clés observées en tant que chaînes : `TerminalType`, `Answerback`,
//!     `ScrollbackLines`, `WinTitle`, `PublicKeyFile`, `ProxyType`, `ProxyPort`,
//!     `SerialLine`, `SerialSpeed`, `TerminalSpeed`, `RemoteQTitleAction`,
//!     `LineCodePage`, `CloseOnExit`, `WarnOnClose`, `Font`,
//!     `Software\SimonTatham\PuTTY\SshHostKeys` (cache de clés d'hôte).
//!   * NON retrouvées dans les chaînes (probablement écrites via des tables
//!     générées) → marquées [DOC]/[SUPPOSE] : `BellType`, `WindowWidth`,
//!     `WindowHeight`, `FontHeight`, `BackspaceKey`, `Colour0`…
//!
//! Lecture : `Default Settings` puis la session demandée par-dessus.
//! Écriture : uniquement la session nommée (comme PuTTY).

use std::path::PathBuf;

use egui::Color32;

use crate::cmdline::{Invocation, Protocol, ProxyType, Tunnel};

/// Racine des sessions, OBSERVÉE dans putty.exe (voir en-tête du fichier).
pub const SESSIONS_ROOT: &str = r"Software\SimonTatham\PuTTY\Sessions";
/// Sous-arbre des valeurs par défaut, OBSERVÉ dans putty.exe.
pub const DEFAULT_SETTINGS: &str = "Default Settings";
/// Cache des clés d'hôte SSH, OBSERVÉ dans putty.exe (pour future parité).
pub const SSH_HOST_KEYS: &str = r"Software\SimonTatham\PuTTY\SshHostKeys";

/// Type de bell (`BellType` dans le registre PuTTY). [DOC] mapping 0/1/2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BellType {
    Audible,
    Visual,
    None,
}

/// Touche Backspace (`BackspaceKey` dans le registre PuTTY).
/// Comportement par défaut de PuTTY : Control-H [DOC — les deux chaînes
/// « Control-H » et « DELETE » existent dans le binaire, offsets 1132997 et
/// 1088740 ; le défaut ^H est documenté mais reste à confirmer par REA].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackspaceKey {
    CtrlH,
    Del,
}

/// Fermeture de fenêtre (`CloseOnExit`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CloseOnExit {
    Always,
    Never,
    /// « seulement après une sortie propre » — défaut supposé [DOC].
    OnCleanExit,
}

/// Description de police (PuTTY : valeur `Font` + `FontHeight`).
#[derive(Clone, Debug)]
pub struct FontSpec {
    pub family: String,
    pub size_pt: f32,
}

impl Default for FontSpec {
    fn default() -> Self {
        // Famille : « Courier New » OBSERVÉE dans putty.exe (offset 1088111 +
        // docs/REA_EVIDENCE.md §4) ; « Consolas » et « Fixedsys » ABSENTS.
        // Taille : défaut doc 0.83 = **18 pt** [docs/REA_EVIDENCE.md §4,
        // [RUNTIME] note « défaut doc 0.83 : 18 »] — conforme à la consigne.
        FontSpec { family: "Courier New".into(), size_pt: 18.0 }
    }
}

/// Configuration complète d'une session.
#[derive(Clone, Debug)]
pub struct Config {
    /// Nom de la session chargée (« Default Settings » si aucune).
    pub session_name: String,

    // --- Connexion -----------------------------------------------------------
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Nom d'utilisateur local (clé `LocalUserName`, non câblé).
    pub local_username: String,
    pub serial_line: String,   // `SerialLine` (OBSERVÉ), ex. « COM1 »
    pub serial_speed: u32,     // `SerialSpeed` (OBSERVÉ), défaut 9600 [DOC]

    // --- Terminal --------------------------------------------------------------
    /// `TerminalType` (OBSERVÉ) ; défaut « xterm » [DOC — chaîne « xterm »
    /// présente dans le binaire, offset 1101343].
    pub term_type: String,
    /// `Answerback` (OBSERVÉ ; libellé GUI « Answerback to ^E: », offset 1145268)
    /// : réponse envoyée au serveur quand il envoie ENQ (^E). Défaut historique
    /// « PuTTY » `[RUNTIME]`.
    pub answerback: String,
    pub bell_type: BellType,           // `Beep`/`BeepInd` [OBS] (legacy `BellType` [DOC])
    pub visual_flash_ms: u64,          // équivalent VisualBellDuration [SUPPOSE]
    pub scrollback: usize,             // `ScrollbackLines` (OBSERVÉ), défaut 2000 [RUNTIME]
    pub width: usize,                  // `TermWidth` [OBS]
    pub height: usize,                 // `TermHeight` [OBS]
    pub font: FontSpec,                // `Font`/`FontHeight` [OBS]
    pub backspace: BackspaceKey,       // `BackspaceIsDelete` [OBS] (défaut ^H [DOC], [REA])
    /// Flèches en mode application au démarrage (option GUI « Application
    /// cursor keys ») — défaut false [SUPPOSE].
    pub app_cursor_keys_default: bool,

    // --- Fenêtre ---------------------------------------------------------------
    /// `WinTitle` (OBSERVÉ). Vide = titre dynamique (OSC) autorisé.
    pub win_title: String,
    /// Titre dynamique verrouillé (via `-T` ou `-clear-avtcache` [SUPPOSE]).
    pub disable_remote_title: bool,
    pub close_on_exit: CloseOnExit,    // `CloseOnExit` (OBSERVÉ)
    pub warn_on_close: bool,           // `WarnOnClose` (OBSERVÉ)
    pub blink_cursor: bool,            // [SUPPOSE] défaut clignotant, 500 ms
    pub scroll_on_key: bool,           // `ScrollOnKey` [DOC] défaut true ? [REA]

    // --- Couleurs (palette 16 couleurs ANSI + défauts) --------------------------
    /// `Colour0` = foreground, `Colour1` = curseur [DOC ; noms non retrouvés
    /// dans le scan → [REA]].
    pub fg: Color32,
    pub cursor_fg: Color32,
    pub bg: Color32,
    /// Palette ANSI 0..15 (xterm defaults [DOC] — la palette exacte de PuTTY
    /// diffère légèrement : [REA] à relever depuis la GUI).
    pub palette: [Color32; 16],
    pub selection_bg: Color32,

    // --- SSH ---------------------------------------------------------------------
    pub try_agent: bool,               // `TryAgent` [DOC] défaut true
    pub public_key_file: Option<PathBuf>, // `PublicKeyFile` (OBSERVÉ) + libellé
                                          // « Private key file for authentication »
                                          // (OBSERVÉ, offset 1144389)
    pub x11_forward: bool,             // `X11Forward` [DOC] défaut false
    pub x11_display: String,           // `X11Display` [DOC]
    pub compression: bool,             // `Compression` [DOC] défaut false
    pub no_shell: bool,                // « Command» vide / -N [DOC]
    pub tunnels: Vec<Tunnel>,          // `PortForwardings` [DOC]
    pub accept_hostkey: bool,          // politique hôte inconnu (GUI : popup)
    pub reject_hostkey: bool,
    pub nodelay: bool,                 // `TCPNoDelay` [DOC] défaut true (PuTTY
                                       // active nodelay par défaut [SUPPOSE])
    /// Vitesse de ligne annoncée (`TerminalSpeed` OBSERVÉ) ; défaut 38400 [DOC].
    pub terminal_speed: u32,

    // --- Proxy ----------------------------------------------------------------------
    pub proxy_type: ProxyType,         // `ProxyType` (OBSERVÉ) défaut None
    pub proxy_host: String,            // `ProxyHost`
    pub proxy_port: u16,               // `ProxyPort` (OBSERVÉ) défaut 8080 [DOC]
    pub proxy_username: String,
    pub proxy_password: String,

    // --- Telnet ----------------------------------------------------------------------
    /// `TelnetTermType` [DOC] — type de terminal utilisé pour le protocole
    /// TELNET (PuTTY envoie « TERM=putty » par défaut en telnet [DOC]).
    pub telnet_term_type: String,
}

impl Default for Config {
    /// Valeurs par défaut « observables » documentées dans ANALYSIS_NOTES.md.
    fn default() -> Self {
        Config {
            session_name: DEFAULT_SETTINGS.into(),
            protocol: Protocol::Ssh,          // GUI PuTTY démarre sur SSH [DOC]
            host: String::new(),
            port: 22,                            // défaut ssh [DOC]
            username: String::new(),             // PuTTY : vide → nom Windows
            local_username: String::new(),
            serial_line: "COM1".into(),          // [DOC] défaut GUI
            serial_speed: 9600,                  // [DOC]

            term_type: "xterm".into(),           // [DOC + chaîne observée + RUNTIME]
            // Défaut historique « PuTTY » [DOC ; présent dans le dump RUNTIME
            // REA_EVIDENCE.md §4]. La consigne proposait vide → preuve en faveur
            // de « PuTTY » retenue, [REA] pour confirmer sur Default Settings.
            answerback: "PuTTY".into(),
            bell_type: BellType::Audible,        // [DOC ; clés OBS : Beep/BeepInd]
            visual_flash_ms: 300,                // [SUPPOSE]
            scrollback: 2000,                    // [RUNTIME ScrollbackLines=0x7d0]
            width: 80,                           // [RUNTIME TermWidth=0x50]
            height: 24,                          // [RUNTIME TermHeight=0x18]
            font: FontSpec::default(),           // [OBS nom][DOC taille 18]
            backspace: BackspaceKey::CtrlH,      // [DOC][REA session vue: 1=DEL]
            app_cursor_keys_default: false,      // [SUPPOSE]

            win_title: String::new(),            // [DOC : vide]
            disable_remote_title: false,
            close_on_exit: CloseOnExit::OnCleanExit, // [DOC]
            warn_on_close: true,                 // [DOC]
            blink_cursor: true,                  // [SUPPOSE]
            scroll_on_key: true,                 // [DOC][REA?]

            fg: Color32::from_rgb(187, 187, 187), // [REA §6 : pairs stock 187/x]
            cursor_fg: Color32::from_rgb(187, 187, 187),
            bg: Color32::BLACK,
            palette: putty_default_palette(),
            selection_bg: Color32::from_rgb(51, 153, 255), // [SUPPOSE]

            try_agent: true,                     // [DOC]
            public_key_file: None,
            x11_forward: false,
            x11_display: String::new(),
            compression: false,
            no_shell: false,
            tunnels: Vec::new(),
            accept_hostkey: false,
            reject_hostkey: false,
            nodelay: true,
            terminal_speed: 38400,               // [DOC]

            proxy_type: ProxyType::None,
            proxy_host: String::new(),
            proxy_port: 80,                      // [RUNTIME ProxyPort=80]
            proxy_username: String::new(),
            proxy_password: String::new(),

            telnet_term_type: "putty".into(),    // [DOC : PuTTY envoie « putty »
                                                 // en telnet — à confirmer [REA]]
        }
    }
}

impl Config {
    /// Charge `Default Settings`, puis la session `name` par-dessus si fournie.
    /// Sur non-Windows (ou erreur de registre), retombe sur les défauts.
    pub fn load(name: Option<&str>) -> Config {
        let mut cfg = Config::default();
        #[cfg(windows)]
        {
            if let Some(key) = open_session_key(DEFAULT_SETTINGS) {
                apply_registry_values(&key, &mut cfg);
            }
            if let Some(n) = name {
                if let Some(key) = open_session_key(n) {
                    apply_registry_values(&key, &mut cfg);
                    cfg.session_name = n.to_string();
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = name; // pas de registre hors Windows : défauts seulement
        }
        cfg
    }

    /// Sauvegarde la session courante dans le registre (session nommée).
    /// [SUPPOSE] squelette : écrit nos propres noms de clés (compatibles
    /// PuTTY quand observés) ; ne réécrit pas les clés que nous ignorons.
    pub fn save(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
            let (k, _disp) = winreg::RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey_with_flags(
                    format!(r"{SESSIONS_ROOT}\{}", self.session_name),
                    KEY_SET_VALUE | KEY_READ,
                )
                .map_err(|e| e.to_string())?;
            let s = |k: &winreg::RegKey, name: &str, v: &str| {
                k.set_value(name, &v.to_string()).map_err(|e| e.to_string())
            };
            let d = |k: &winreg::RegKey, name: &str, v: u32| {
                k.set_value(name, &v).map_err(|e| e.to_string())
            };
            // Noms de clés OBSERVÉS (REA_EVIDENCE.md §4).
            s(&k, "TerminalType", &self.term_type)?;
            s(&k, "Answerback", &self.answerback)?;
            s(&k, "WinTitle", &self.win_title)?;
            s(&k, "SerialLine", &self.serial_line)?;
            d(&k, "SerialSpeed", self.serial_speed)?;
            // `TerminalSpeed` OBSERVÉ au format chaîne « in,out » [RUNTIME].
            s(&k, "TerminalSpeed", &format!("{},{}", self.terminal_speed, self.terminal_speed))?;
            d(&k, "ScrollbackLines", self.scrollback as u32)?;
            d(&k, "TermWidth", self.width as u32)?;
            d(&k, "TermHeight", self.height as u32)?;
            s(&k, "Font", &self.font.family)?;
            d(&k, "FontHeight", self.font.size_pt as u32)?;
            d(&k, "BeepInd", match self.bell_type {
                BellType::Audible => 1,
                BellType::Visual | BellType::None => 0,
            })?;
            d(&k, "BackspaceIsDelete", match self.backspace {
                BackspaceKey::CtrlH => 0,
                BackspaceKey::Del => 1,
            })?;
            d(&k, "BlinkCur", self.blink_cursor as u32)?;
            d(&k, "ScrollOnKey", self.scroll_on_key as u32)?;
            s(&k, "Protocol", self.protocol.as_str())?;
            s(&k, "HostName", &self.host)?;
            d(&k, "PortNumber", self.port as u32)?;
            s(&k, "UserName", &self.username)?;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Ok(())
        }
    }

    /// Fusionne la ligne de commande par-dessus la session chargée
    /// (priorité : ligne de commande > session > défauts, comme PuTTY [DOC]).
    pub fn apply_invocation(&mut self, inv: &Invocation) {
        if let Some(p) = inv.protocol {
            self.protocol = p;
            // Comme PuTTY : changer de protocole change le port, SAUF si le
            // port a été forcé explicitement sur la ligne de commande.
            if inv.port.is_none() {
                if let Some(dp) = p.default_port() {
                    self.port = dp;
                }
            }
        }
        if let Some(h) = &inv.host {
            self.host = h.clone();
        }
        if let Some(p) = inv.port {
            self.port = p;
        }
        if let Some(u) = &inv.username {
            self.username = u.clone();
        }
        if let Some(s) = &inv.serial_line {
            self.serial_line = s.clone();
            self.protocol = Protocol::Serial;
        }
        if self.protocol == Protocol::Serial {
            // `-serial COM3` : le positionnel est la ligne série [DOC PuTTY].
            if let Some(h) = &inv.host {
                if !h.is_empty() {
                    self.serial_line = h.clone();
                }
            }
            // `-sercfg 9600,8,n,1` : premier champ = débit [DOC].
            if let Some(c) = &inv.serial_config {
                if let Some(speed) = c.split(',').next().and_then(|s| s.trim().parse::<u32>().ok()) {
                    self.serial_speed = speed;
                }
            }
        }
        if let Some(t) = &inv.window_title {
            self.win_title = t.clone();
            self.disable_remote_title = true; // -T fige le titre [DOC]
        }
        if inv.clear_title {
            // [SUPPOSE] -clear-avtcache désactive la réception du titre distant
            self.disable_remote_title = true;
        }
        if let Some(t) = &inv.term_type {
            self.term_type = t.clone();
        }
        if let Some(a) = inv.try_agent {
            self.try_agent = a;
        }
        if let Some(k) = &inv.key_file {
            self.public_key_file = Some(k.clone());
        }
        if let Some(x) = inv.x11_forward {
            self.x11_forward = x;
        }
        if let Some(c) = inv.compression {
            self.compression = c;
        }
        if inv.no_shell {
            self.no_shell = true;
        }
        if !inv.tunnels.is_empty() {
            self.tunnels = inv.tunnels.clone();
        }
        self.accept_hostkey |= inv.accept_hostkey;
        self.reject_hostkey |= inv.reject_hostkey;
        if inv.nodelay {
            self.nodelay = true;
        }
        if let Some(ty) = inv.proxy {
            self.proxy_type = ty;
        }
        if let Some(h) = &inv.proxy_host {
            self.proxy_host = h.clone();
        }
        if let Some(p) = inv.proxy_port {
            self.proxy_port = p;
        }
        if let Some(u) = &inv.proxy_user {
            self.proxy_username = u.clone();
        }
        if let Some(p) = &inv.proxy_pass {
            self.proxy_password = p.clone();
        }
    }

    /// Résout le protocole effectif + port par défaut si besoin.
    pub fn resolved(&self) -> (Protocol, u16) {
        let port = match self.protocol.default_port() {
            Some(dp) if self.port == 0 => dp,
            _ => self.port,
        };
        (self.protocol, port)
    }

    /// Titre effectif de la fenêtre (sans titre dynamique reçu).
    pub fn static_title(&self) -> String {
        if !self.win_title.is_empty() {
            return self.win_title.clone();
        }
        // Format de titre par défaut de PuTTY : « user@host: port » [DOC]
        // [SUPPOSE] : squelette = host seul si titre vide.
        if self.host.is_empty() {
            "PuTTY-rs".into()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Palette ANSI 0..15 par défaut de PuTTY, telle qu'OBSERVÉE dans le dump
/// `[RUNTIME]` de `Sessions\demo-session` (docs/REA_EVIDENCE.md §6) : normales
/// sur base 187, vives sur base 255/85 (style VGA). Les valeurs exactes des
/// slots non échantillonnés dans la preuve sont [SUPPOSE] (même logique).
fn putty_default_palette() -> [Color32; 16] {
    const fn c(r: u8, g: u8, b: u8) -> Color32 {
        Color32::from_rgb(r, g, b)
    }
    [
        // 0..7 normales (base 187)
        c(0, 0, 0), c(187, 0, 0), c(0, 187, 0), c(187, 187, 0),
        c(0, 0, 187), c(187, 0, 187), c(0, 187, 187), c(187, 187, 187),
        // 8..15 vives (base 255 / 85)
        c(85, 85, 85), c(255, 85, 85), c(85, 255, 85), c(255, 255, 85),
        c(85, 85, 255), c(255, 85, 255), c(85, 255, 255), c(255, 255, 255),
    ]
}

// =============================================================================
// Implémentation registre (Windows uniquement)
// =============================================================================
//
// NOMS DE VALEURS = OBSERVÉS (docs/REA_EVIDENCE.md §4, inventaire Ghidra +
// dump `reg query` [RUNTIME]) : `HostName`, `PortNumber`, `Protocol`,
// `TerminalType`, `TerminalSpeed` (chaîne « in,out »), `Answerback`,
// `ScrollbackLines`, `TermWidth`, `TermHeight`, `Font`, `FontHeight`,
// `BackspaceIsDelete`, `Beep`/`BeepInd`, `BellOverload{N,T,S}`,
// `ScrollOnKey`, `ScrollOnDisp`, `BlinkCur`, `CurType`, `WinTitle`,
// `WinNameAlways`, `RemoteQTitleAction`, `CloseOnExit`, `WarnOnClose`,
// `PublicKeyFile`, `TryAgent`, `X11Forward`/`X11Display`, `Compression`,
// `TCPNoDelay`, `Proxy{Type,Host,Port,Username,Password}`, `Serial{Line,Speed,…}`,
// `TelnetRet`, `Colour0..21` (disposition OBSERVÉE §6 : 0=fg, 1=fg gras,
// 2=bg, 3=bg gras, 4=texte curseur, 5=couleur curseur, 6..21=ANSI par paires
// (normale,vive)). Les anciens noms « WindowWidth/Height, Port, BellType,
// BackspaceKey » sont TOUJOURS lus en repli (compatibilité).

#[cfg(windows)]
fn open_session_key(name: &str) -> Option<winreg::RegKey> {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(format!(r"{SESSIONS_ROOT}\{name}"))
        .ok()
}

/// `McpPort` (DWORD, clé additionnelle putty-rs [DESIGN MCP_PILOT]) sous
/// `…\PuTTY\Default Settings` — 0 = désactivé. Chaîne acceptée aussi
/// (reg add ... /v McpPort /t REG_SZ /d 3927 pratique).
#[cfg(windows)]
pub fn registry_mcp_port() -> Option<u16> {
    let k = open_session_key("Default Settings")?;
    if let Ok(v) = k.get_value::<u32, _>("McpPort") {
        return Some(v as u16);
    }
    k.get_value::<String, _>("McpPort")
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
}

#[cfg(not(windows))]
pub fn registry_mcp_port() -> Option<u16> {
    None
}

/// Lecture générique clé→valeur (string, sinon dword/bool rendus en texte),
/// pour l'outil MCP `get_config` (docs/MCP_PILOT.md).
#[cfg(windows)]
pub fn registry_get(key: &str) -> Option<String> {
    let k = open_session_key("Default Settings")?;
    if let Ok(s) = k.get_value::<String, _>(key) {
        return Some(s);
    }
    if let Ok(v) = k.get_value::<u32, _>(key) {
        return Some(v.to_string());
    }
    None
}

#[cfg(not(windows))]
pub fn registry_get(_key: &str) -> Option<String> {
    None
}

/// Écriture simple (REG_SZ — l'outillage registre PuTTY réel tolère la
/// lecture défensive ; [DESIGN]) pour l'outil MCP `set_config`.
#[cfg(windows)]
pub fn registry_set(key: &str, value: &str) -> Result<(), String> {
    let (k, _d) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .create_subkey(format!(r"{SESSIONS_ROOT}\Default Settings"))
        .map_err(|e| e.to_string())?;
    k.set_value(key, &value.to_string()).map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn registry_set(_key: &str, _value: &str) -> Result<(), String> {
    Err("registre indisponible sur ce plateforme".into())
}

/// Applique les valeurs du registre connues sur une `Config`.
#[cfg(windows)]
fn apply_registry_values(key: &winreg::RegKey, cfg: &mut Config) {
    fn gs(key: &winreg::RegKey, v: &str) -> Option<String> {
        key.get_value::<String, _>(v).ok()
    }
    fn gd(key: &winreg::RegKey, v: &str) -> Option<u32> {
        key.get_value::<u32, _>(v).ok()
    }

    // --- Terminal (clés OBSERVÉES) ------------------------------------------
    if let Some(t) = gs(key, "TerminalType") { cfg.term_type = t; }
    if let Some(a) = gs(key, "Answerback") { cfg.answerback = a; }
    if let Some(w) = gs(key, "WinTitle") { cfg.win_title = w; }
    if let Some(l) = gs(key, "SerialLine") { cfg.serial_line = l; }
    if let Some(s) = gd(key, "SerialSpeed") { cfg.serial_speed = s; }
    if let Some(n) = gd(key, "ScrollbackLines") { cfg.scrollback = n as usize; }
    if let Some(f) = gs(key, "Font") { cfg.font.family = f; }
    // `TerminalSpeed` OBSERVÉ au format chaîne « 38400,38400 » [RUNTIME].
    if let Some(s) = gs(key, "TerminalSpeed") {
        if let Some(first) = s.split(',').next().and_then(|x| x.trim().parse::<u32>().ok()) {
            cfg.terminal_speed = first;
        }
    }

    // --- Dimensions / police : TermWidth/TermHeight/FontHeight OBSERVÉS ------
    if let Some(n) = gd(key, "TermWidth").or_else(|| gd(key, "WindowWidth")) {
        cfg.width = n.max(20) as usize;
    }
    if let Some(n) = gd(key, "TermHeight").or_else(|| gd(key, "WindowHeight")) {
        cfg.height = n.max(2) as usize;
    }
    if let Some(n) = gd(key, "FontHeight") {
        // [RUNTIME] valeur stockée positive ici (0xa=10 sur ce poste) ; la
        // convention « négatif = points » est [DOC][REA].
        cfg.font.size_pt = (n as i32).unsigned_abs().clamp(6, 72) as f32;
    }

    // --- Cloche : `Beep`/`BeepInd` OBSERVÉS [OBS] ; `BellType` = repli [DOC].
    if let Some(b) = gs(key, "Beep") {
        cfg.bell_type = match b.to_ascii_lowercase().as_str() {
            "none" => BellType::None,
            "visible" | "visual" => BellType::Visual,
            _ => BellType::Audible,
        };
    } else if let Some(b) = gd(key, "BeepInd") {
        cfg.bell_type = if b == 0 { BellType::None } else { BellType::Audible };
    } else if let Some(b) = gd(key, "BellType") {
        cfg.bell_type = match b { 1 => BellType::Visual, 2 => BellType::None, _ => BellType::Audible };
    }

    // --- Backspace : `BackspaceIsDelete` OBSERVÉ [OBS].
    if let Some(b) = gd(key, "BackspaceIsDelete").or_else(|| gd(key, "BackspaceKey")) {
        cfg.backspace = if b != 0 { BackspaceKey::Del } else { BackspaceKey::CtrlH };
    }

    // --- Curseur : `BlinkCur`, `CurType` OBSERVÉS [OBS].
    if let Some(b) = gd(key, "BlinkCur") { cfg.blink_cursor = b != 0; }

    // --- Scrollback : `ScrollOnKey`/`ScrollOnDisp` OBSERVÉS [OBS].
    if let Some(b) = gd(key, "ScrollOnKey") { cfg.scroll_on_key = b != 0; }

    if let Some(b) = gd(key, "WarnOnClose") { cfg.warn_on_close = b != 0; }
    if let Some(b) = gd(key, "CloseOnExit") {
        // [RUNTIME] CloseOnExit=1 par défaut = « seulement sortie propre ».
        cfg.close_on_exit = match b { 0 => CloseOnExit::Never, 2 => CloseOnExit::Always, _ => CloseOnExit::OnCleanExit };
    }

    // --- Connexion -------------------------------------------------------------
    if let Some(p) = gs(key, "Protocol") {
        cfg.protocol = match p.as_str() {
            "telnet" => Protocol::Telnet,
            "rlogin" => Protocol::Rlogin,
            "raw" => Protocol::Raw,
            "serial" => Protocol::Serial,
            _ => Protocol::Ssh,
        };
    }
    if let Some(h) = gs(key, "HostName") { cfg.host = h; }
    // `PortNumber` OBSERVÉ [OBS] (22 en [RUNTIME]) ; repli legacy « Port ».
    if let Some(n) = gd(key, "PortNumber").or_else(|| gd(key, "Port")) { cfg.port = n as u16; }
    if let Some(u) = gs(key, "UserName") { cfg.username = u; }

    // --- Couleurs — disposition OBSERVÉE (REA_EVIDENCE.md §6) :
    //     0=fg, 1=fg gras, 2=bg, 3=bg gras, 4=texte curseur, 5=couleur curseur,
    //     6..21 = 16 ANSI par paires (normale, vive).
    if let Some(c) = gs(key, "Colour0") { cfg.fg = parse_rgb(&c).unwrap_or(cfg.fg); }
    if let Some(c) = gs(key, "Colour2") { cfg.bg = parse_rgb(&c).unwrap_or(cfg.bg); }
    if let Some(c) = gs(key, "Colour5") { cfg.cursor_fg = parse_rgb(&c).unwrap_or(cfg.cursor_fg); }
    for i in 0..16usize {
        if let Some(c) = gs(key, &format!("Colour{}", 6 + i)) {
            if let Some(col) = parse_rgb(&c) {
                cfg.palette[i] = col;
            }
        }
    }

    // --- SSH / proxy [OBS/RUNTIME] ----------------------------------------------
    if let Some(k) = gs(key, "PublicKeyFile") {
        cfg.public_key_file = if k.is_empty() { None } else { Some(PathBuf::from(k)) };
    }
    if let Some(b) = gd(key, "TryAgent") { cfg.try_agent = b != 0; }
    if let Some(b) = gd(key, "X11Forward") { cfg.x11_forward = b != 0; }
    if let Some(d) = gs(key, "X11Display") { cfg.x11_display = d; }
    if let Some(b) = gd(key, "Compression") { cfg.compression = b != 0; }
    if let Some(b) = gd(key, "TCPNoDelay") { cfg.nodelay = b != 0; }
    if let Some(t) = gd(key, "ProxyType") {
        cfg.proxy_type = match t {
            1 => ProxyType::Http,
            2 => ProxyType::Socks4,
            3 => ProxyType::Socks5,
            4 => ProxyType::Local,
            5 => ProxyType::Telnet,
            _ => ProxyType::None,
        };
    }
    if let Some(h) = gs(key, "ProxyHost") { cfg.proxy_host = h; }
    if let Some(n) = gd(key, "ProxyPort") { cfg.proxy_port = n as u16; }
    if let Some(u) = gs(key, "ProxyUsername") { cfg.proxy_username = u; }
    if let Some(p) = gs(key, "ProxyPassword") { cfg.proxy_password = p; }
    if let Some(t) = gs(key, "TelnetTermType") { cfg.telnet_term_type = t; }
}

/// Parse « r,g,b » (format des valeurs ColourN du registre PuTTY [DOC]).
#[cfg(windows)]
fn parse_rgb(s: &str) -> Option<Color32> {
    let mut it = s.split(',').map(|x| x.trim().parse::<u8>().ok());
    Some(Color32::from_rgb(it.next()??, it.next()??, it.next()??))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_putty_documented() {
        let c = Config::default();
        assert_eq!(c.width, 80);
        assert_eq!(c.height, 24);
        assert_eq!(c.port, 22);
        assert_eq!(c.term_type, "xterm");
        assert_eq!(c.scrollback, 2000);
        // Défaut historique OBSERVÉ [RUNTIME] : answerback « PuTTY ».
        assert_eq!(c.answerback, "PuTTY");
    }

    #[test]
    fn protocol_switch_changes_port() {
        let mut c = Config::default();
        let mut inv = Invocation::default();
        inv.protocol = Some(Protocol::Telnet);
        c.apply_invocation(&inv);
        assert_eq!(c.port, 23);
        // Port forcé sur la ligne de commande ⇒ non écrasé.
        let mut c2 = Config::default();
        inv.port = Some(1234);
        c2.apply_invocation(&inv);
        assert_eq!(c2.port, 1234);
    }

    #[test]
    fn title_freezing() {
        let mut c = Config::default();
        let mut inv = Invocation::default();
        inv.window_title = Some("Mon titre".into());
        c.apply_invocation(&inv);
        assert!(c.disable_remote_title);
        assert_eq!(c.static_title(), "Mon titre");
    }
}
