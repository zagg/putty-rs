//! Surcouche « façon PuTTY » au-dessus du parseur `vt100`.
//!
//! Rôle :
//!  * alimenter le parseur avec les octets reçus du backend ;
//!  * gérer les événements qui ne sont pas dans le modèle d'écran via
//!    `vt100::Callbacks` : BEL (visuel), titre de fenêtre OSC, resize demandé
//!    par le serveur, et ENQ (^E) → réponse Answerback ;
//!  * encoder le clavier egui en octets à envoyer au backend, avec les
//!    particularités PuTTY (Backspace=^H par défaut, flèches en mode
//!    application quand DECCKM est actif, Alt = préfixe ESC…).
//!
//! API vt100 validée sur docs.rs (vt100 0.18.1) :
//!  * `Parser::new_with_callbacks(rows, cols, scrollback, cb)`
//!  * `parser.process(bytes)`, `parser.set_size(rows, cols)`
//!  * `screen().cell(r, c)`, `contents_between(...)`, `cursor_position()`,
//!    `application_cursor()`, `hide_cursor()`, `set_scrollback(off)`…
//!  * `Callbacks::unhandled_control(&mut self, &mut Screen, u8)` : c'est PAR
//!    CETTE VOIE que l'ENQ (0x05) nous parvient [SUPPOSE : ENQ non listé comme
//!    méthode dédiée dans Callbacks ; à confirmer à l'exécution].

use std::collections::VecDeque;
use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use crate::config::{BackspaceKey, Config};

/// Événements émis par le parseur vt100 vers la surcouche.
#[derive(Clone, Debug)]
pub enum TermEvent {
    /// BEL (^G) — rendu visuel façon PuTTY (flash), pas de beep système
    /// [SUPPOSE : le beep Windows sera branché si BellType=Audible].
    Bell,
    /// Titre de fenêtre demandé par OSC 0/2.
    Title(String),
    /// Le serveur demande un resize (CSI 8 ; rows ; cols t).
    ResizeRequest(u16, u16),
    /// ENQ (^E) reçu : il faut renvoyer l'Answerback.
    Enq,
}

/// Implémentation des callbacks vt100 : on ne fait que mettre en file.
#[derive(Default)]
pub struct PuTTYCallbacks {
    pub events: Vec<TermEvent>,
}

impl vt100::Callbacks for PuTTYCallbacks {
    fn audible_bell(&mut self, _screen: &mut vt100::Screen) {
        self.events.push(TermEvent::Bell);
    }
    fn visual_bell(&mut self, _screen: &mut vt100::Screen) {
        self.events.push(TermEvent::Bell);
    }
    fn set_window_title(&mut self, _screen: &mut vt100::Screen, title: &[u8]) {
        self.events.push(TermEvent::Title(String::from_utf8_lossy(title).into_owned()));
    }
    fn resize(&mut self, _screen: &mut vt100::Screen, request: (u16, u16)) {
        self.events.push(TermEvent::ResizeRequest(request.0, request.1));
    }
    fn unhandled_control(&mut self, _screen: &mut vt100::Screen, b: u8) {
        // 0x05 = ENQ : PuTTY répond la chaîne « Answerback » [DOC + libellé
        // GUI « Answerback to ^E: » OBSERVÉ dans putty.exe offset 1145268].
        if b == 0x05 {
            self.events.push(TermEvent::Enq);
        } else {
            log::trace!("control non géré reçu : 0x{b:02x}");
        }
    }
}

/// Le terminal : parseur vt100 + file d'octets à renvoyer au backend
/// (answerback, réponses diverses).
pub struct Terminal {
    parser: vt100::Parser<PuTTYCallbacks>,
    answerback: Vec<u8>,
    /// Octets à envoyer au backend (drainé par l'UI à chaque frame).
    outbox: VecDeque<Vec<u8>>,
    /// Dernier titre reçu dynamiquement (OSC), si autorisé.
    title: Option<String>,
    /// Fin du flash BEL en cours (BEL visuel « façon PuTTY »).
    pub bell_flash_until: Option<Instant>,
    /// Position de défilement (0 = en bas, n = n lignes dans le scrollback).
    pub scroll_offset: usize,
    cols: u16,
    rows: u16,
}

impl Terminal {
    pub fn new(cfg: &Config) -> Terminal {
        let (cols, rows) = (cfg.width as u16, cfg.height as u16);
        let cb = PuTTYCallbacks::default();
        let parser = vt100::Parser::new_with_callbacks(
            nz(rows),
            nz(cols),
            cfg.scrollback,
            cb,
        );
        Terminal {
            parser,
            answerback: cfg.answerback.as_bytes().to_vec(),
            outbox: VecDeque::new(),
            title: None,
            bell_flash_until: None,
            scroll_offset: 0,
            cols: cols.max(2),
            rows: rows.max(2),
        }
    }

    /// Injecte des octets provenant du backend (flux serveur → écran).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        // Draine les événements générés par le parseur.
        let events = std::mem::take(&mut self.parser.callbacks_mut().events);
        for ev in events {
            match ev {
                TermEvent::Bell => {
                    self.bell_flash_until = Some(Instant::now() + Duration::from_millis(300));
                }
                TermEvent::Title(t) => self.title = Some(t),
                TermEvent::ResizeRequest(_r, _c) => {
                    // [SUPPOSE] PuTTY ne redimensionne pas la fenêtre sur
                    // CSI 8 t sans autorisation ; on journalise seulement.
                    log::debug!("le serveur demande un resize {0}x{1} (ignoré)", _r, _c);
                }
                TermEvent::Enq => {
                    if !self.answerback.is_empty() {
                        self.outbox.push_back(self.answerback.clone());
                    }
                }
            }
        }
    }

    /// Redimensionne l'écran (UI → vt100 ; le backend sera averti par l'appelant).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols.max(2);
        self.rows = rows.max(2);
        self.parser.set_size(nz(self.rows), nz(self.cols));
    }

    pub fn cols(&self) -> u16 { self.cols }
    pub fn rows(&self) -> u16 { self.rows }
    pub fn screen(&self) -> &vt100::Screen { self.parser.screen() }

    /// Octets à envoyer au backend (answerback suite à ENQ, etc.).
    pub fn drain_outbox(&mut self) -> Vec<Vec<u8>> {
        self.outbox.drain(..).collect()
    }

    /// Récupère le titre dynamique s'il a changé (None sinon).
    pub fn take_title(&mut self) -> Option<String> {
        self.title.clone()
    }

    /// Le défilement est-il actuellement « en bas » (flux live) ?
    pub fn at_bottom(&self) -> bool {
        self.scroll_offset == 0
    }

    /// Décale la vue dans le scrollback (borné par la taille réelle).
    pub fn scroll_by(&mut self, delta_rows: i32) {
        let max = self.screen().scrollback_len() as i32;
        let next = (self.scroll_offset as i32 - delta_rows).clamp(0, max);
        self.scroll_offset = next as usize;
        // vt100 : set_scrollback(offset depuis le HAUT de l'écran). On mappe
        // notre décalage « depuis le bas » en décalage vt100.
        let vt_offset = (self.screen().scrollback_len() - self.scroll_offset).min(self.screen().scrollback_len());
        self.parser.screen_mut().set_scrollback(vt_offset);
    }

    // ---------------------------------------------------------------------
    // Sélection (copier = relâchement du clic, comme PuTTY)
    // ---------------------------------------------------------------------

    /// Extrait le texte sélectionné entre deux cellules (triées), en
    /// enlevant les espaces de fin de ligne (comportement copie de PuTTY
    /// [DOC — à confirmer REA]) et en joignant les lignes par \n.
    pub fn selection_text(&self, a: (u16, u16), b: (u16, u16)) -> String {
        let (r1, c1, r2, c2) = if a <= b { (a.0, a.1, b.0, b.1) } else { (b.0, b.1, a.0, a.1) };
        let raw = self.screen().contents_between(r1, c1, r2, c2);
        raw.lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Conversion sûre en NonZeroU16 bornée (1..=4096 : garde-fou UI).
fn nz(v: u16) -> NonZeroU16 {
    NonZeroU16::new(v.clamp(1, 4096)).unwrap_or(NonZeroU16::MIN)
}

// =============================================================================
// Encodeur clavier egui → octets, avec les particularités PuTTY.
// =============================================================================

/// Contexte d'encodage : modes dépendant de l'état (reçu du serveur).
#[derive(Clone, Copy, Debug)]
pub struct KeyEncoder {
    /// `BackspaceKey` : ^H par défaut comme PuTTY [DOC][REA].
    pub backspace: BackspaceKey,
    /// DECCKM reçu du serveur (`ESC [ ? 1 h`) — lu depuis vt100.
    pub application_cursor: bool,
    /// DECCKM pavé numérique (`ESC [ ? 66 h`) — lu depuis vt100.
    pub application_keypad: bool,
}

/// Code de contrôle ASCII pour Ctrl+X, à la PuTTY [DOC : Ctrl+C=^C=0x03] :
/// A→0x01 … Z→0x1A, Espace→0x00 (NUL [SUPPOSE]). Aucun autre Key ne produit
/// de code (la ponctuation Ctrl+dépend du clavier, laissée vide comme avant).
pub fn ctrl_code(key: Key) -> Option<u8> {
    let c: u8 = match key {
        Key::A => b'A',
        Key::B => b'B',
        Key::C => b'C',
        Key::D => b'D',
        Key::E => b'E',
        Key::F => b'F',
        Key::G => b'G',
        Key::H => b'H',
        Key::I => b'I',
        Key::J => b'J',
        Key::K => b'K',
        Key::L => b'L',
        Key::M => b'M',
        Key::N => b'N',
        Key::O => b'O',
        Key::P => b'P',
        Key::Q => b'Q',
        Key::R => b'R',
        Key::S => b'S',
        Key::T => b'T',
        Key::U => b'U',
        Key::V => b'V',
        Key::W => b'W',
        Key::X => b'X',
        Key::Y => b'Y',
        Key::Z => b'Z',
        Key::Space => b' ',
        _ => return None,
    };
    Some(c & 0x1f)
}

/// Encode un événement clavier « spécial » (non textuel) en octets.
/// Renvoie une vec vide si la touche ne doit rien envoyer.
pub fn encode_special_key(key: Key, mods: &Modifiers, enc: &KeyEncoder) -> Vec<u8> {
    // Préfixe Alt = ESC (comportement PuTTY par défaut [DOC]).
    let alt_prefix: &[u8] = if mods.alt { b"\x1b" } else { b"" };

    // Ctrl+lettre/espace : le code de contrôle prime (Key::C+CTRL → 0x03).
    // (egui pose `command` en miroir de Ctrl sur Windows : ne pas l'exiger.)
    if mods.ctrl && !mods.alt {
        if let Some(b) = ctrl_code(key) {
            return vec![b];
        }
    }

    let body: &[u8] = match key {
        Key::Enter => b"\r", // PuTTY envoie CR [DOC]
        Key::Tab => b"\t",
        Key::Escape => b"\x1b",
        // Backspace : ^H par défaut ; Shift+Backspace envoie DEL [DOC PuTTY].
        Key::Backspace => {
            if mods.shift || enc.backspace == BackspaceKey::Del {
                b"\x7f"
            } else {
                b"\x08"
            }
        }
        Key::ArrowUp => {
            if enc.application_cursor { b"\x1bOA" } else { b"\x1b[A" }
        }
        Key::ArrowDown => {
            if enc.application_cursor { b"\x1bOB" } else { b"\x1b[B" }
        }
        Key::ArrowRight => {
            if enc.application_cursor { b"\x1bOC" } else { b"\x1b[C" }
        }
        Key::ArrowLeft => {
            if enc.application_cursor { b"\x1bOD" } else { b"\x1b[D" }
        }
        // Touched par le pavé applicatif ? [SUPPOSE : on utilise les codes
        // xterm standards ; le mapping exact PuTTY est à relever REA.]
        Key::Home => if enc.application_cursor { b"\x1bOH" } else { b"\x1b[H" },
        Key::End => if enc.application_cursor { b"\x1bOF" } else { b"\x1b[F" },
        Key::Insert => b"\x1b[2~",
        Key::Delete => b"\x1b[3~",
        Key::PageUp => b"\x1b[5~",
        Key::PageDown => b"\x1b[6~",
        Key::F1 => b"\x1bOP",
        Key::F2 => b"\x1bOQ",
        Key::F3 => b"\x1bOR",
        Key::F4 => b"\x1bOS",
        Key::F5 => b"\x1b[15~",
        Key::F6 => b"\x1b[17~",
        Key::F7 => b"\x1b[18~",
        Key::F8 => b"\x1b[19~",
        Key::F9 => b"\x1b[20~",
        Key::F10 => b"\x1b[21~",
        Key::F11 => b"\x1b[23~",
        Key::F12 => b"\x1b[24~",
        _ => return Vec::new(),
    };

    let mut out = alt_prefix.to_vec();
    out.extend_from_slice(body);
    out
}

/// Encode un événement texte egui (caractères imprimables).
///
/// * Ctrl+lettre → code de contrôle (`A`→0x01 …). [DOC PuTTY : Ctrl+C=^C etc.]
/// * Ctrl+espace → NUL [SUPPOSE].
/// * Alt → préfixe ESC puis les octets UTF-8 du texte. [SUPPOSE : PuTTY en
///   code page 1252 envoie des octets Latin-1 ; notre squelette envoie UTF-8,
///   le choix de la page (`LineCodePage`, clé OBSERVÉE) est une limite connue.]
pub fn encode_text(text: &str, mods: &Modifiers) -> Vec<u8> {
    if text.is_empty() {
        return Vec::new();
    }
    if mods.ctrl && !mods.alt {
        let b0 = text.as_bytes()[0];
        // Ctrl+lettre : masque 0x1f sur la majuscule ASCII.
        if b0.is_ascii_alphabetic() {
            return vec![b0 & 0x1f];
        }
        if b0 == b' ' {
            return vec![0];
        }
        // Ctrl+autre (ponctuation) : comportement dépend de la touche ;
        // le squelette n'envoie rien (Ctrl+C/V sont gérés comme presse-papiers).
        return Vec::new();
    }
    let mut out = Vec::with_capacity(text.len() + 1);
    if mods.alt {
        out.push(0x1b);
    }
    out.extend_from_slice(text.as_bytes());
    out
}

/// L'éditeur de PuTTY ouvre les URL Ctrl+clic etc. ; ici on prévoit au moins
/// le collage : PuTTY refuse par défaut le collage contenant CR/LF/TAB sauf
/// confirmation (« Attempt to paste ... » [DOC]). On filtre par sécurité.
/// [SUPPOSE : le dialogue d'avertissement n'est pas implémenté, on nettoie.]
pub fn sanitize_paste(text: &str) -> Vec<u8> {
    text.replace('\r', "\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc() -> KeyEncoder {
        KeyEncoder { backspace: BackspaceKey::CtrlH, application_cursor: false, application_keypad: false }
    }

    #[test]
    fn arrows_normal_vs_application() {
        assert_eq!(encode_special_key(Key::ArrowUp, &Modifiers::NONE, &enc()), b"\x1b[A");
        let app = KeyEncoder { application_cursor: true, ..enc() };
        assert_eq!(encode_special_key(Key::ArrowUp, &Modifiers::NONE, &app), b"\x1bOA");
    }

    #[test]
    fn backspace_default_ctrlh() {
        assert_eq!(encode_special_key(Key::Backspace, &Modifiers::NONE, &enc()), b"\x08");
        assert_eq!(
            encode_special_key(Key::Backspace, &Modifiers::SHIFT, &enc()),
            b"\x7f"
        );
    }

    #[test]
    fn ctrl_letter_special_key_events() {
        // Regression 27/10 : KeyPressed(C)+CTRL ne renvoyait rien (heredoc
        // MCP impossible a interrompre). Desormais 0x03, comme la frappe.
        let m = Modifiers { ctrl: true, ..Default::default() };
        assert_eq!(encode_special_key(Key::C, &m, &enc()), vec![3]);
        assert_eq!(encode_special_key(Key::D, &m, &enc()), vec![4]);
        assert_eq!(encode_special_key(Key::Space, &m, &enc()), vec![0]);
        assert_eq!(ctrl_code(Key::Z), Some(26));
        assert_eq!(ctrl_code(Key::Enter), None);
    }

    #[test]
    fn ctrl_letter_control_codes() {
        let m = Modifiers { ctrl: true, ..Default::default() };
        assert_eq!(encode_text("c", &m), vec![3]);
        assert_eq!(encode_text("C", &m), vec![3]);
    }

    #[test]
    fn enter_is_cr() {
        assert_eq!(encode_special_key(Key::Enter, &Modifiers::NONE, &enc()), b"\r");
    }

    #[test]
    fn answerback_on_enq() {
        let mut cfg = Config::default();
        cfg.answerback = "putty-rs\r".into();
        let mut term = Terminal::new(&cfg);
        term.feed(&[0x05]); // ENQ
        let out = term.drain_outbox();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], b"putty-rs\r");
    }
}
