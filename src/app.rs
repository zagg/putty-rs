//! Application eframe : fenêtre, grille terminal, clavier, souris.
//!
//! Équivalences PuTTY implémentées :
//!  * rendu grille 80×24 (redimensionnable) avec couleurs issues de vt100 ;
//!  * curseur clignotant (bloc) — clignotement 500 ms `[SUPPOSE]` ;
//!  * BEL « visuel » : flash jaune de la grille (équivalent VisualBell ;
//!    le type `BellType` du registre est respecté : Audible = beep futur,
//!    None = rien) ;
//!  * titre dynamique OSC (désactivable par `-T` / `-clear-avtcache`) ;
//!  * sélection souris : glisser = sélectionner, relâcher = COPIER
//!    [DOC PuTTY] ; Ctrl+Insert copie aussi ; Entrée copie si une sélection
//!    existe (comportement « copier=clic/Entrée » demandé par la consigne,
//!    `[SUPPOSE]` — à confronter à la GUI réelle [REA]) ;
//!  * coller = clic droit, Shift+Insert, Ctrl+V [DOC PuTTY : right-mouse et
//!    Shift-Insert] ;
//!  * Ctrl+C : copie si sélection, sinon envoie ^C `[SUPPOSE]` ;
//!  * molette : défilement du scrollback `[SUPPOSE]`.
//!
//! API egui 0.31 utilisée : CentralPanel + Frame::new, allocate_exact_size,
//! Painter::layout_job, Context::copy_text, send_viewport_cmd(Title),
//! input().events / raw_scroll_delta, FontData::from_named [API-À-VÉRIFIER].

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, Event, FontData, FontFamily, FontId, Frame, Key, Modifiers, Rect,
    Sense, TextFormat, Vec2,
};
// egui 0.31 : LayoutJob n'est PAS re-exporte a la racine (module text).
use eframe::egui::text::LayoutJob;

use crate::backend::{self, BackendCommand, BackendEvent, BackendHandle};
use crate::cmdline;
use crate::config::{self, BellType, Config};
use crate::terminal;
use crate::theme;
use crate::mcp::{self, ConfigOp, McpCommand, McpHandle, McpPublisher};
use crate::session::{self, Status};

/// Repaint max toutes les 60 ms (clignotement + polling mpsc).
const FRAME_PERIOD: Duration = Duration::from_millis(60);

/// Style d'un run de cellule (fg, bg, attributs).
#[derive(Clone, Copy, PartialEq, Eq)]
struct RowStyle {
    fg: Color32,
    bg: Color32,
    bold: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
}

/// Run horizontal de cellules de même style, prêt à peindre.
struct RowRun {
    col0: u16,
    text: String,
    style: RowStyle,
}

pub struct PuTTYApp {
    cfg: Config,
    /// État de session partagé (parseur vt100, statut, flux de sortie).
    /// Alimenté par le thread pompe (`session::Pump`), lu par l'UI *et* par le
    /// serveur MCP : les snapshots n'attendent plus un repaint eframe.
    live: Arc<Mutex<session::LiveSession>>,
    pump: Option<session::Pump>,
    backend: BackendHandle,
    /// Moitié publication MCP (pour les changements pilotés par l'UI).
    publisher: Option<McpPublisher>,

    // Rendu
    font_id: FontId,
    char_size: Vec2,
    metrics_ready: bool,
    fonts_installed: bool,
    started: Instant,

    // Sélection souris
    sel_start: Option<(u16, u16)>,
    sel_end: Option<(u16, u16)>,

    // Titre déjà appliqué à la fenêtre
    last_title: String,

    // Thème cosmétique (docs/THEME_DARK.md) : sombre « professional » par
    // défaut, legacy clair via AppTheme=light — [DESIGN], palette fidèle aux
    // 22 slots PuTTY (Colour0..21, layout RUNTIME validé).
    theme: theme::Theme,

    // --- Serveur MCP embarqué (docs/MCP_PILOT.md, bus src/mcp/bus.rs) ------
    mcp: Option<McpHandle>,
    /// id MCP de la session courante (chaîne uuid v4, FIXÉ 27/10).
    mcp_session_id: String,
    /// Redemande l'application du style egui à la frame suivante.
    style_dirty: bool,
    /// `resize` MCP en attente : redimensionne aussi la FENÊTRE (sinon
    /// l'auto-resize pixels→colonnes annule la demande à la frame suivante).
    mcp_pending_win: Option<(u16, u16)>,

    // Presse-papiers OS (clic droit / Shift+Insert), arboard = pur Rust.
    clipboard: Option<arboard::Clipboard>,
}

impl PuTTYApp {
    pub fn new(mut cfg: Config, backend: BackendHandle, events: Receiver<BackendEvent>, mcp: Option<McpHandle>) -> Self {
        // Thème sombre par défaut (demande utilisateur 27/10) ; si l'utilisateur
        // n'a jamais touché à la police (défaut registre = « Courier New »
        // [REA §4]), bascule cosmétique pro Consolas 11 pt [THEME_DARK §7],
        // sinon on respecte son choix.
        let t = theme::Theme::professional();
        if cfg.font.family.eq_ignore_ascii_case("Courier New") {
            cfg.font.family = theme::DEFAULT_FONT_DARK.to_owned();
            cfg.font.size_pt = theme::DEFAULT_FONT_DARK_PT;
        }
        cfg.fg = t.palette.fg();
        cfg.bg = t.palette.bg();
        cfg.selection_bg = t.palette.sel_bg();
        cfg.cursor_fg = t.palette.cursor_colour();
        let font_id = FontId::monospace(cfg.font.size_pt);
        let mcp_session_id =
            if mcp.is_some() { mcp::new_session_id() } else { String::new() };
        // État partagé + pompe backend → état → MCP (hors repaint eframe).
        let live = Arc::new(Mutex::new(session::LiveSession::new(&cfg, mcp_session_id.clone())));
        let publisher = mcp.as_ref().map(McpHandle::publisher);
        let pump = session::Pump::spawn(
            Arc::clone(&live),
            events,
            backend.sender(),
            publisher.clone(),
        );
        PuTTYApp {
            cfg,
            live,
            pump: Some(pump),
            backend,
            publisher,
            font_id,
            char_size: Vec2::new(8.0, 16.0), // recalculé au premier frame
            metrics_ready: false,
            fonts_installed: false,
            started: Instant::now(),
            sel_start: None,
            sel_end: None,
            last_title: String::new(),
            theme: t,
            mcp,
            mcp_session_id,
            style_dirty: false,
            mcp_pending_win: None,
            clipboard: arboard::Clipboard::new().ok(),
        }
    }

    /// Verrou bref sur l'état partagé (jamais paniquer sur poison).
    fn live(&self) -> std::sync::MutexGuard<'_, session::LiveSession> {
        self.live.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Publie immédiatement l'état courant vers le MCP (changements décidés
    /// par l'UI ou par une commande MCP : resize, fermeture, nouveau titre…).
    fn publish_now(&self) {
        if let Some(b) = self.mcp.as_ref() {
            let (snap, seq) = {
                let mut l = self.live();
                (vec![l.snapshot()], l.output_seq)
            };
            log::debug!("ui: publish seq={seq}");
            b.publish(snap, true);
        }
    }

    // =========================================================================
    // Aide : position de cellule sous le curseur souris
    // =========================================================================
    fn cell_at(&self, rect: Rect, pos: egui::Pos2) -> (u16, u16) {
        let (cols, rows) = {
            let l = self.live();
            (l.term.cols(), l.term.rows())
        };
        let col = ((pos.x - rect.min.x) / self.char_size.x)
            .clamp(0.0, f32::from(cols) - 1.0) as u16;
        let row = ((pos.y - rect.min.y) / self.char_size.y)
            .clamp(0.0, f32::from(rows) - 1.0) as u16;
        (row, col)
    }

    /// Copie la sélection vers le presse-papiers (relâchement souris,
    /// Ctrl+Insert, Ctrl+C, ou Entrée « copier » [SUPPOSE]).
    fn copy_selection(&mut self, ctx: &egui::Context) {
        if let (Some(a), Some(b)) = (self.sel_start, self.sel_end) {
            let text = self.live().term.selection_text(a, b);
            if !text.is_empty() {
                ctx.copy_text(text);
            }
        }
    }

    /// Collage façon PuTTY : clic droit / Shift+Insert / Ctrl+V.
    fn do_paste(&mut self, ctx: &egui::Context) {
        // 1) événement Paste déjà fourni par eframe (Ctrl+V plateforme)
        let evt_paste: Option<String> = ctx.input(|i| {
            i.events
                .iter()
                .find_map(|e| if let Event::Paste(s) = e { Some(s.clone()) } else { None })
        });
        let text = match evt_paste {
            Some(t) => Some(t),
            // 2) sinon lecture directe du presse-papiers (clic droit, Shift+Insert)
            None => self.clipboard.as_mut().and_then(|cb| cb.get_text().ok()),
        };
        if let Some(t) = text {
            // PuTTY refuse/avertit pour CR/LF/TAB non autorisés [DOC] ; ici on
            // normalise CR→LF et on envoie [SUPPOSE].
            let bytes = terminal::sanitize_paste(&t);
            if !bytes.is_empty() {
                self.backend.send(BackendCommand::Write(bytes));
                // Retour en bas du scrollback quand le serveur echoing.
                self.live().term.scroll_to_bottom();
            }
        }
    }

    // =========================================================================
    // Rendu de la grille : runs de style identique (vt100 → egui)
    // =========================================================================

    /// Copie une ligne de la grille sous verrou (aucun appel de peinture :
    /// le verrou de session n'est jamais tenu pendant le rendu GPU).
    fn collect_row(
        &self,
        screen: &vt100::Screen,
        row: u16,
        cols: u16,
        sel: Option<((u16, u16), (u16, u16))>,
    ) -> Vec<RowRun> {
        let mut runs: Vec<RowRun> = Vec::new();
        for col in 0..cols {
            let cell = match screen.cell(row, col) {
                Some(c) => c,
                None => continue,
            };
            let in_sel = sel.is_some_and(|((r1, c1), (r2, c2))| {
                (row > r1 || (row == r1 && col >= c1)) && (row < r2 || (row == r2 && col <= c2))
            });

            let fg = self.color_of(cell.fgcolor(), true, self.cfg.fg);
            let mut bg = self.color_of(cell.bgcolor(), false, self.cfg.bg);
            if in_sel {
                bg = self.cfg.selection_bg;
            }
            let style = RowStyle {
                fg,
                bg,
                bold: cell.bold(),
                italic: cell.italic(),
                underline: cell.underline(),
                inverse: cell.inverse(),
            };
            let ch = cell.contents();
            let ch: &str = if ch.is_empty() { " " } else { &ch };
            if let Some(last) = runs.last_mut() {
                if last.style == style {
                    last.text.push_str(ch);
                    continue;
                }
            }
            runs.push(RowRun { col0: col, text: ch.to_string(), style });
        }
        runs
    }

    /// Peint une ligne déjà copiée (hors verrou).
    fn paint_row(&self, painter: &egui::Painter, rect: Rect, row: u16, runs: &[RowRun]) {
        let (cw, ch_h) = (self.char_size.x, self.char_size.y);

        // 1) Fonds colorés des runs (hors fond par défaut).
        for run in runs {
            if run.style.bg != self.cfg.bg {
                let w = run.text.chars().count() as f32 * cw;
                let r = Rect::from_min_size(
                    rect.min + Vec2::new(f32::from(run.col0) * cw, f32::from(row) * ch_h),
                    Vec2::new(w, ch_h),
                )
                .intersect(rect);
                painter.rect_filled(r, 0.0, run.style.bg);
            }
        }

        // 2) Texte : un seul LayoutJob, runs consécutifs.
        let mut job = LayoutJob::default();
        for run in runs {
            let mut tf = TextFormat {
                font_id: self.font_id.clone(),
                color: if run.style.inverse { run.style.bg } else { run.style.fg },
                ..Default::default()
            };
            if run.style.bold {
                // egui n'a pas de gras synthétique en monospace : approximation
                // par teinte plus claire [SUPPOSE] ; PuTTY propose « BoldAsColour »
                // (clé OBSERVÉE `BoldAsColour`, offset ~1101343) [DOC].
                tf.color = tf.color.linear_multiply(1.35);
            }
            if run.style.italic {
                tf.italics = true;
            }
            if run.style.underline {
                tf.underline = egui::Stroke::new(1.0_f32, tf.color);
            }
            job.append(&run.text, 0.0, tf);
        }
        let galley = painter.layout_job(job);
        painter.galley(
            rect.min + Vec2::new(0.0, f32::from(row) * ch_h),
            galley,
            self.cfg.fg,
        );
    }

    /// Mapping vt100::Color → Color32 (palette 16 ANSI + RGB + défaut).
    fn color_of(&self, c: vt100::Color, foreground: bool, _fallback: Color32) -> Color32 {
        match c {
            vt100::Color::Default => {
                if foreground { self.cfg.fg } else { self.cfg.bg }
            }
            vt100::Color::Idx(i) => {
                // <16 : palette ANSI ; 16..255 : cube 6×6×6 + grisés, style
                // xterm [DOC xterm ; palette PuTTY réelle → REA].
                if (i as usize) < self.cfg.palette.len() {
                    self.cfg.palette[i as usize]
                } else if i >= 16 && i < 232 {
                    let i = i - 16;
                    let r = (i / 36) * 51;
                    let g = ((i / 6) % 6) * 51;
                    let b = (i % 6) * 51;
                    Color32::from_rgb(r, g, b)
                } else {
                    let v = 8 + (i.saturating_sub(232)) * 10;
                    Color32::from_gray(v.min(255))
                }
            }
            vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        }
    }

    // =========================================================================
    // Clavier
    // =========================================================================
    fn handle_keyboard(&mut self, ui: &egui::Ui, focused: bool) {
        if !focused {
            return;
        }
        let events: Vec<Event> = ui.input(|i| i.events.clone());
        let enc = {
            let l = self.live();
            terminal::KeyEncoder {
                backspace: self.cfg.backspace,
                application_cursor: l.term.screen().application_cursor(),
                application_keypad: l.term.screen().application_keypad(),
            }
        };
        for ev in events {
            match ev {
                Event::Key { key, modifiers, pressed, repeat, .. } => {
                    if !pressed && !repeat {
                        continue;
                    }
                    // --- Raccourcis presse-papiers ---
                    if self.is_copy_combo(key, &modifiers) {
                        self.copy_selection(ui.ctx());
                        continue;
                    }
                    if self.is_paste_combo(key, &modifiers) {
                        self.do_paste(ui.ctx());
                        continue;
                    }
                    // --- Ctrl+C avec sélection = copie [SUPPOSE], sinon ^C ---
                    if key == Key::C && modifiers.ctrl && !modifiers.alt {
                        if self.sel_start.is_some() && self.sel_end.is_some() {
                            self.copy_selection(ui.ctx());
                            continue;
                        }
                        // sans sélection : on tombe dans encode_special_key,
                        // qui renvoie desormais 0x03 (ctrl_code) — [DOC PuTTY].
                    }
                    // --- Entrée = copie si sélection [SUPPOSE] ---
                    if key == Key::Enter
                        && !modifiers.ctrl
                        && self.sel_start.is_some()
                        && self.sel_end.is_some()
                    {
                        self.copy_selection(ui.ctx());
                        self.sel_start = None;
                        self.sel_end = None;
                        continue;
                    }
                    let bytes = terminal::encode_special_key(key, &modifiers, &enc);
                    if !bytes.is_empty() {
                        self.backend.send(BackendCommand::Write(bytes));
                    }
                }
                Event::Text(text) => {
                    // egui 0.31 : Event::Text est un tuple (sans modifiers) ;
                    // on lit les modificateurs courants a la volee.
                    let mods = ui.input(|i| i.modifiers);
                    // Les octets de controle (CR, ^C…) sont deja emis par le
                    // chemin Key (encode_special_key + ctrl_code) : les
                    // renvoyer ici les dualiserait (double Entree, double ^C).
                    if text.chars().all(|c| (c as u32) < 0x20) {
                        continue;
                    }
                    let bytes = terminal::encode_text(&text, &mods);
                    if !bytes.is_empty() {
                        self.backend.send(BackendCommand::Write(bytes));
                    }
                }
                Event::Paste(s) => {
                    let bytes = terminal::sanitize_paste(&s);
                    self.backend.send(BackendCommand::Write(bytes));
                }
                _ => {}
            }
        }
    }

    fn is_copy_combo(&self, key: Key, m: &Modifiers) -> bool {
        key == Key::Insert && m.ctrl
    }

    fn is_paste_combo(&self, key: Key, m: &Modifiers) -> bool {
        (key == Key::Insert && m.shift) || (key == Key::V && (m.ctrl || m.command))
    }

    // =========================================================================
    // Installation des polices (première frame)
    // =========================================================================
    fn install_fonts(&mut self, ctx: &egui::Context) {
        if self.fonts_installed {
            return;
        }
        self.fonts_installed = true;
        let family = self.cfg.font.family.clone();
        let mut fonts = egui::FontDefinitions::default();
        // egui 0.31 n'a pas de FontData::from_named : on charge le TTF du
        // systeme (Courier New, OBSERVE dans putty.exe [REA]) depuis
        // C:\Windows\Fonts. Repli silencieuse : monospace embarque egui.
        let candidate: Option<String> = match family.as_str() {
            f if f.eq_ignore_ascii_case("courier new") => Some("cour.ttf".into()),
            f if f.eq_ignore_ascii_case("consolas") => Some("consola.ttf".into()),
            f if f.eq_ignore_ascii_case("cascadia mono") => Some("CascadiaMono.ttf".into()),
            f if f.eq_ignore_ascii_case("cascadia code") => Some("CascadiaCode.ttf".into()),
            f => Some(format!("{f}.ttf")),
        };
        let data = candidate
            .and_then(|f| std::fs::read(format!(r"C:\Windows\Fonts\{f}")).ok())
            .map(FontData::from_owned);
        if let Some(d) = data {
            fonts.font_data.insert("putty-rs-font".to_owned(), Arc::new(d));
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .insert(0, "putty-rs-font".into());
        } else {
            log::warn!("police « {family} » non trouvee en C:\\Windows\\Fonts — repli monospace egui");
        }
        ctx.set_fonts(fonts);
        // Style egui « Professional dark » (une seule fois, comme les fonts)
        // — docs/THEME_DARK.md §3 [DESIGN].
        theme::apply_egui_style(ctx, &self.theme);
        self.font_id = FontId::monospace(self.cfg.font.size_pt);
    }

    // =========================================================================
    // Intégration MCP (docs/MCP_PILOT.md — bus src/mcp/bus.rs)
    // =========================================================================
    // Les snapshots sont construits par `session::LiveSession::snapshot()` et
    // publiés par le thread pompe (indépendant des repaints). L'UI ne publie
    // que les changements qu'elle décide (resize, fermeture, thème…).

    /// Exécute une commande MCP (thread UI — comme un événement clavier).
    fn handle_mcp_command(&mut self, cmd: McpCommand) {
        match cmd {
            McpCommand::SendText { text, .. } => {
                // Pas de CR automatique (décision MCP_PILOT) : le client
                // envoie « user\r » lui-même.
                self.backend.send(BackendCommand::Write(text.into_bytes()));
                self.live().term.scroll_to_bottom();
            }
            McpCommand::SendKeys { keys, .. } => {
                let enc = {
                    let l = self.live();
                    terminal::KeyEncoder {
                        backspace: self.cfg.backspace,
                        application_cursor: l.term.screen().application_cursor(),
                        application_keypad: l.term.screen().application_keypad(),
                    }
                };
                for k in &keys {
                    let b = named_key_bytes(k, &enc);
                    if b.is_empty() {
                        log::warn!("touche MCP inconnue : {k:?}");
                    } else {
                        self.backend.send(BackendCommand::Write(b));
                    }
                }
            }
            McpCommand::Resize { cols, rows, .. } => {
                self.live().term.resize(cols, rows);
                self.backend.send(BackendCommand::Resize { cols, rows });
                // Comportement façon PuTTY : c'est la fenêtre qui s'adapte à
                // la grille demandée (l'auto-resize inverse sinon à la frame
                // suivante annulait la demande — corrigé en test live 27/10).
                self.mcp_pending_win = Some((cols, rows));
            }
            McpCommand::AcceptHostKey { once, .. } => {
                // v0.1 : l'acceptation est déjà automatique (ssh.rs [SIMPLIF]) ;
                // `once` = ne pas stocker — persistance SshHostKeys en v0.2.
                log::info!("accept_host_key(once={once}) : accepté (v0.1 auto)");
            }
            McpCommand::NewSession { host, port, proto } => self.reopen_session(&host, port, &proto),
            McpCommand::CloseSession { .. } => {
                self.backend.shutdown();
                self.live().status = Status::Closed;
            }
            McpCommand::SetTheme { dark } => {
                self.theme = if dark {
                    theme::Theme::professional()
                } else {
                    theme::Theme::legacy()
                };
                self.cfg.fg = self.theme.palette.fg();
                self.cfg.bg = self.theme.palette.bg();
                self.cfg.selection_bg = self.theme.palette.sel_bg();
                self.cfg.cursor_fg = self.theme.palette.cursor_colour();
                self.style_dirty = true;
            }
        }
        // Toute commande MCP change un état visible : publication immédiate.
        self.publish_now();
    }

    /// v0.1 mono-fenêtre : remplace la session courante. Un nouvel id uuid
    /// est publié ; le serveur MCP l'attend (`new_session` poll `world`).
    fn reopen_session(&mut self, host: &str, port: u16, proto: &str) {
        self.backend.shutdown();
        // L'ancienne pompe est détachée : son canal se ferme avec le backend,
        // et son drapeau `alive` l'empêche de publier après le reset.
        if let Some(mut p) = self.pump.take() {
            p.stop();
        }
        self.cfg.host = host.to_string();
        if port != 0 {
            self.cfg.port = port;
        }
        self.cfg.protocol = match proto {
            "telnet" => cmdline::Protocol::Telnet,
            "rlogin" => cmdline::Protocol::Rlogin,
            "raw" => cmdline::Protocol::Raw,
            "serial" => cmdline::Protocol::Serial,
            _ => cmdline::Protocol::Ssh,
        };
        let built = match backend::build(&self.cfg, None) {
            Ok(b) => b,
            Err(e) => {
                self.live().status = Status::Failed(e);
                self.publish_now();
                return;
            }
        };
        match BackendHandle::start(built) {
            Ok((handle, events)) => {
                self.backend = handle;
                self.sel_start = None;
                self.sel_end = None;
                self.last_title = String::new();
                self.mcp_session_id = mcp::new_session_id();
                // Même `Arc` : l'état est remis à zéro puis la pompe reprend.
                self.live().reset(&self.cfg, self.mcp_session_id.clone());
                self.pump = Some(session::Pump::spawn(
                    Arc::clone(&self.live),
                    events,
                    self.backend.sender(),
                    self.publisher.clone(),
                ));
            }
            Err(e) => self.live().status = Status::Failed(e),
        }
        self.publish_now();
    }

    fn config_get(&self, key: &str) -> (bool, String) {
        match config::registry_get(key) {
            Some(v) => (true, v),
            None => (false, format!("clé absente : {key}")),
        }
    }

    fn config_set(&self, key: &str, value: &str) -> (bool, String) {
        match config::registry_set(key, value) {
            Ok(()) => (true, "ok".into()),
            Err(e) => (false, e),
        }
    }
}

impl eframe::App for PuTTYApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Réveil périodique : blink, flash BEL, polling des canaux.
        ctx.request_repaint_after(FRAME_PERIOD);
        self.install_fonts(ctx);

        // --- 0) MCP : drain commandes + config -------------------------------
        // (Les snapshots sont publiés par la pompe de session ; l'UI ne publie
        //  que ce qu'elle décide elle-même.)
        let (cmds, cfgreqs) = match &self.mcp {
            Some(b) => (b.drain_commands(), b.drain_config_requests()),
            None => (Vec::new(), Vec::new()),
        };
        for cmd in cmds {
            self.handle_mcp_command(cmd);
        }
        if let Some((c, r)) = self.mcp_pending_win.take() {
            // Taille intérieure = grille exacte + marges du panneau.
            let size = egui::vec2(
                c as f32 * self.char_size.x + 16.0,
                r as f32 * self.char_size.y + 16.0,
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        for req in cfgreqs {
            let (ok, payload) = match &req.op {
                ConfigOp::Get { key } => self.config_get(key),
                ConfigOp::Set { key, value } => self.config_set(key, value),
            };
            // Jamais bloquant (file pleine = le client MCP a fait un timeout).
            let _ = req.respond.try_send(mcp::ConfigReply { ok, payload });
        }
        if self.style_dirty {
            theme::apply_egui_style(ctx, &self.theme);
            self.style_dirty = false;
        }

        // --- 2) Barre de statut ----------------------------------------------
        let (proto, port) = self.cfg.resolved();
        let (status_txt, cols, rows) = {
            let l = self.live();
            let status = match &l.status {
                Status::Connecting => "connexion…".to_string(),
                Status::Connected => "connecté".to_string(),
                Status::Closed => "fermé".to_string(),
                Status::Failed(e) => format!("erreur : {e}"),
            };
            (status, l.term.cols(), l.term.rows())
        };
        egui::TopBottomPanel::bottom("status")
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} {proto} {}:{} · {cols}×{rows} · {status_txt}",
                        env!("CARGO_PKG_NAME"),
                        self.cfg.host,
                        port,
                    ));
                });
            });

        // --- 3) Grille terminal ------------------------------------------------
        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(2.0).fill(self.cfg.bg))
            .show(ctx, |ui| {
                let avail = ui.available_size();
                let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
                if resp.clicked() || resp.drag_started() {
                    resp.request_focus();
                }

                // -- Métriques de cellule : fiables seulement (le premier
                // galley peut sortir avant que set_fonts soit résolu : on
                // reverdit tant que c'est incohérent, jamais de resize sur
                // des métriques folles — sinon la grille part en 732×2 !).
                if !self.metrics_ready || !cell_metrics_sane(self.char_size) {
                    let job = LayoutJob::simple(
                        "X".repeat(80),
                        self.font_id.clone(),
                        self.cfg.fg,
                        0.0,
                    );
                    let galley = ui.painter().layout_job(job);
                    let cand = Vec2::new(
                        (galley.size().x / 80.0).max(1.0),
                        galley.size().y.max(1.0),
                    );
                    if cell_metrics_sane(cand) {
                        self.char_size = cand;
                        self.metrics_ready = true;
                    }
                }

                // -- Redimensionnement grille → vt100 + backend --
                if self.metrics_ready && cell_metrics_sane(self.char_size) {
                    let cols = (rect.width() / self.char_size.x).floor().max(2.0) as u16;
                    let rows = (rect.height() / self.char_size.y).floor().max(2.0) as u16;
                    let (cur_cols, cur_rows) = {
                        let l = self.live();
                        (l.term.cols(), l.term.rows())
                    };
                    if cols != cur_cols || rows != cur_rows {
                        self.live().term.resize(cols, rows);
                        self.backend.send(BackendCommand::Resize { cols, rows });
                        // L'état visible a changé : snapshot MCP immédiat.
                        self.publish_now();
                    }
                }

                // -- Molette = défilement scrollback --
                let scroll = ui.input(|i| i.raw_scroll_delta.y);
                if scroll.abs() > 0.5 && (resp.contains_pointer() || ui.rect_contains_pointer(rect)) {
                    self.live().term.scroll_by((scroll / self.char_size.y) as i32);
                }

                // -- Souris : sélection --
                if let Some(p) = resp.interact_pointer_pos() {
                    if resp.drag_started() {
                        self.sel_start = Some(self.cell_at(rect, p));
                        self.sel_end = self.sel_start;
                    } else if resp.dragged() {
                        self.sel_end = Some(self.cell_at(rect, p));
                    }
                }
                if resp.drag_stopped() {
                    // Relâchement = copie automatique (PuTTY [DOC]).
                    self.copy_selection(ctx);
                }
                if resp.secondary_clicked() {
                    // Clic droit = collage (PuTTY [DOC]).
                    self.sel_start = None;
                    self.sel_end = None;
                    self.do_paste(ctx);
                }

                // -- Fond --
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, self.cfg.bg);

                // -- Texte + curseur + flash BEL : copie sous verrou, peinture
                //    hors verrou (un hoquet GPU ne doit jamais geler la pompe). --
                let live_arc = Arc::clone(&self.live);
                let (rows_data, cursor, bell_until, cols) = {
                    let l = live_arc.lock().unwrap_or_else(|p| p.into_inner());
                    let term = &l.term;
                    let sel = match (self.sel_start, self.sel_end) {
                        (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
                        _ => None,
                    };
                    let mut rows = Vec::with_capacity(term.rows() as usize);
                    for row in 0..term.rows() {
                        rows.push(self.collect_row(term.screen(), row, term.cols(), sel));
                    }
                    let cur = if term.at_bottom() && !term.screen().hide_cursor() {
                        Some(term.screen().cursor_position())
                    } else {
                        None
                    };
                    (rows, cur, term.bell_flash_until, term.cols())
                };

                for (row, runs) in rows_data.iter().enumerate() {
                    self.paint_row(&painter, rect, row as u16, runs);
                }

                // -- Curseur bloc clignotant [SUPPOSE 500 ms] --
                let blink_on = !self.cfg.blink_cursor
                    || (self.started.elapsed().as_millis() / 500) % 2 == 0;
                if let Some((cr, cc)) = cursor {
                    if blink_on && cr < rows_data.len() as u16 && cc < cols {
                        let r = Rect::from_min_size(
                            rect.min
                                + Vec2::new(
                                    f32::from(cc) * self.char_size.x,
                                    f32::from(cr) * self.char_size.y,
                                ),
                            self.char_size,
                        );
                        // Curseur bloc : fond couleur curseur [SUPPOSE].
                        painter.rect_filled(r, 0.0, self.cfg.cursor_fg.linear_multiply(0.35));
                        painter.rect_stroke(
                            r,
                            0.0,
                            egui::Stroke::new(1.0_f32, self.cfg.cursor_fg),
                            egui::StrokeKind::Inside,
                        );
                    }
                }

                // -- Flash BEL visuel (équivalent VisualBell PuTTY) --
                if matches!(self.cfg.bell_type, BellType::Visual | BellType::Audible) {
                    if let Some(until) = bell_until {
                        if Instant::now() < until {
                            painter.rect_filled(
                                rect,
                                0.0,
                                Color32::from_rgba_unmultiplied(255, 255, 0, 40),
                            );
                        }
                    }
                }

                // -- Clavier (hors verrou : handle_keyboard relit l'état) --
                self.handle_keyboard(ui, resp.has_focus());
            });

        // --- 4) Titre de fenêtre -------------------------------------------------
        let title = if self.cfg.disable_remote_title {
            self.cfg.static_title()
        } else {
            let remote = self.live().title.clone();
            if remote.is_empty() {
                self.cfg.static_title()
            } else {
                remote
            }
        };
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // --- 5) Fermeture : arrêt propre du backend -------------------------------
        if ctx.input(|i| i.viewport().close_requested()) {
            self.backend.shutdown();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(mut p) = self.pump.take() {
            p.stop();
        }
        self.backend.shutdown();
        self.backend.join();
    }
}

/// Seuil de crédibilité des métriques de cellule (px) : évite qu'un galley
/// calculé avant la résolution de la police ne casse la grille (732×2…).
fn cell_metrics_sane(v: egui::Vec2) -> bool {
    v.x.is_finite() && v.y.is_finite() && (3.0..=40.0).contains(&v.x) && (8.0..=60.0).contains(&v.y)
}

/// Mapping des noms envoyés par l'outil MCP `send_keys` → séquences d'octets
/// strictement identiques à celles du clavier physique (terminal.rs, modes
/// DECARM / application cursor keys — cf. docs/REA_EVIDENCE.md §5).
fn named_key_bytes(name: &str, enc: &terminal::KeyEncoder) -> Vec<u8> {
    use egui::Key;
    // Forme « ^x » = contrôle direct (même octet que la frappe physique).
    if let Some(rest) = name.trim().strip_prefix('^') {
        let b = rest.as_bytes();
        if b.len() == 1 && (b[0] == b'\\' || b[0] == b']' || b[0].is_ascii_alphabetic()) {
            return vec![b[0] & 0x1f];
        }
    }
    // « Ctrl+C » (forme usuelle avec +) doit marcher comme « ctrl-c » :
    // normalise le + en - pour les modificateurs (sinon envoi silencieux).
    let n = name.trim().to_ascii_lowercase().replace('+', "-");
    // Forme « ctrl-x » generique (toutes lettres + espace) : meme code que la
    // frappe physique ^X. La forme avec + (« Ctrl+C ») est normalisee plus bas.
    if let Some(rest) = n.strip_prefix("ctrl-") {
        let rb = rest.as_bytes();
        if rb.len() == 1 && rb[0].is_ascii_alphabetic() {
            return vec![rb[0].to_ascii_uppercase() & 0x1f];
        }
        if rest == "space" {
            return vec![0];
        }
    }
    // Caracteres imprimables seuls (« q », « Q », « / », « 7 »…) : octet
    // direct, comme si l'agent les tapait (on garde la casse d'origine).
    let raw = name.trim();
    let nb = raw.as_bytes();
    if nb.len() == 1 && (0x20..0x7f).contains(&nb[0]) {
        return vec![nb[0]];
    }
    let (key, m) = match n.as_str() {
        "enter" | "cr" | "return" => (Key::Enter, Modifiers::NONE),
        "up" | "↑" => (Key::ArrowUp, Modifiers::NONE),
        "down" | "↓" => (Key::ArrowDown, Modifiers::NONE),
        "left" | "←" => (Key::ArrowLeft, Modifiers::NONE),
        "right" | "→" => (Key::ArrowRight, Modifiers::NONE),
        "tab" => (Key::Tab, Modifiers::NONE),
        "esc" | "escape" => (Key::Escape, Modifiers::NONE),
        "backspace" => (Key::Backspace, Modifiers::NONE),
        "delete" | "del" => (Key::Delete, Modifiers::NONE),
        "insert" | "ins" => (Key::Insert, Modifiers::NONE),
        "home" => (Key::Home, Modifiers::NONE),
        "end" => (Key::End, Modifiers::NONE),
        "pageup" | "pgup" => (Key::PageUp, Modifiers::NONE),
        "pagedown" | "pgdn" => (Key::PageDown, Modifiers::NONE),
        "space" => (Key::Space, Modifiers::NONE),
        "f1" => (Key::F1, Modifiers::NONE),
        "f2" => (Key::F2, Modifiers::NONE),
        "f3" => (Key::F3, Modifiers::NONE),
        "f4" => (Key::F4, Modifiers::NONE),
        "f5" => (Key::F5, Modifiers::NONE),
        "f6" => (Key::F6, Modifiers::NONE),
        "f7" => (Key::F7, Modifiers::NONE),
        "f8" => (Key::F8, Modifiers::NONE),
        "f9" => (Key::F9, Modifiers::NONE),
        "f10" => (Key::F10, Modifiers::NONE),
        "f11" => (Key::F11, Modifiers::NONE),
        "f12" => (Key::F12, Modifiers::NONE),
        "ctrl-c" => (Key::C, Modifiers::CTRL),
        "ctrl-d" => (Key::D, Modifiers::CTRL),
        "ctrl-z" => (Key::Z, Modifiers::CTRL),
        "ctrl-l" => (Key::L, Modifiers::CTRL),
        _ => return Vec::new(),
    };
    terminal::encode_special_key(key, &m, enc)
}
