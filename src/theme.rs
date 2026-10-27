//! src/theme.rs — Palettes, thèmes et pont egui pour putty-rs (clone PuTTY 0.83).
//!
//! Module **autonome** : ne dépend que d'`egui::Color32` (et de `std`). Aucune
//! référence vers les autres modules putty-rs ; il doit rester compilable seul,
//! indépendamment du squelette `main.rs`/`app.rs`/`config.rs`.
//!
//! Sources de vérité :
//!   - `docs/THEME_DARK.md` (spéc cosmétique) — palette ANSI 16 §1, slots
//!     système 16..21 §2, overrides egui §3.2, cloche/curseur §4, thème clair
//!     legacy §5, clés registre §6, police/padding §7.
//!   - `docs/REA_EVIDENCE.md` §6/§7 — noms réels `Colour0..Colour21` au format
//!     `"r,g,b"` (REG_SZ), `BellType`, `Beep`/`BeepInd`, `FontVTMode`,
//!     `BlinkCur`, `CurType`, `WinNameAlways`, menus (clic droit / système).
//!
//! Marqueur `[AMBIGU]` : point où les sources se contredisent (ou taisent) ;
//! un commentaire explique le choix retenu et l'alternative.

use egui::Color32;

// ═══════════════════════════════════════════════════════════════════════════
// 0. Constantes d'index et clés de registre
// ═══════════════════════════════════════════════════════════════════════════

/// Nombre total de slots couleur (Colour0..Colour21) — [REA].
pub const N_COLOUR_SLOTS: usize = 22;

// Mapping des slots — SOURCE : THEME_DARK.md §1-2 et table §5.
//   0..15  = ANSI (voir §1) ; 16..21 = 6 couleurs « système » (voir §2).
//
// [AMBIGU] Cette disposition N'est PAS celle du PuTTY amont : le PuTTY réel
// stocke 0=fg, 1=bold fg, 2=bg, 3=bold bg, 4=cursor text, 5=cursor colour,
// 6..21=ANSI (les slots « système » en tête). La ligne REA §6 donne encore une
// troisième variante (4-19 = ANSI). On suit THÈME_DARK.md §1-2/§5, décision
// d'archi du projet ; si un jour on veut relire un profil PuTTY natif, il
// faudra une table de transposition (toujours connue, ajouter dans config.rs).
pub const IDX_FG: usize = 16;           // avant-plan terminal par défaut
pub const IDX_BOLD_FG: usize = 17;      // avant-plan en gras (fonte bold)
pub const IDX_BG: usize = 18;           // arrière-plan terminal
pub const IDX_BOLD_BG: usize = 19;      // arrière-plan en gras
pub const IDX_CURSOR_TEXT: usize = 20;  // glyphe dessiné DANS le curseur plein
pub const IDX_CURSOR_COLOUR: usize = 21; // remplissage du curseur

/// Sélection du terminal : gris 50 %, identique dans les deux thèmes — §5.
pub const SEL_BG: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);

/// Cloche visuelle (VISBEL) : flash inverse 100 ms sur BEL — §4 [doc PuTTY].
pub const VISBEL_MS: u16 = 100;
/// Clignotement curseur : période 530 ms (≈ caret Windows) — §4 [DESIGN].
pub const DEFAULT_CURSOR_BLINK_MS: u16 = 530;
/// Marge intérieure de la grille de texte : 8 px tous côtés — §7 [DESIGN].
pub const DEFAULT_CELL_PAD: u8 = 8;
/// Espacement de ligne en % ajouté à la hauteur de cellule, borné 0..25 — §6.
pub const DEFAULT_LINE_SPACING_PCT: u16 = 0; // [AMBIGU] défaut absent du spec → neutre
pub const MAX_LINE_SPACING_PCT: u16 = 25;

/// Noms de valeurs registre cités par les specs — pour le recâblage config.rs.
pub mod regkeys {
    /// Préfixe des 22 slots couleur ; clé n°i = `format!("{COLOUR_PREFIX}{i}")`.
    pub const COLOUR_PREFIX: &str = "Colour"; // "r,g,b" REG_SZ — [REA §6 doc]
    pub const BELL_TYPE: &str = "BellType";   // int : 0=SYSTEM, 1=NONE, 2=VISBEL — [REA/THEME §4]
    pub const BEEP: &str = "Beep";            // bips système — [REA §4]
    pub const BEEP_IND: &str = "BeepInd";     // cloche visuelle par indice — [REA §4]
    pub const FONT_VT_MODE: &str = "FontVTMode"; // int ; THEME §6 : 1 = VT ; [REA RUNTIME] obs. 4
    pub const BLINK_CUR: &str = "BlinkCur";   // curseur clignotant — [REA §4]
    pub const CUR_TYPE: &str = "CurType";     // forme du curseur — [REA §4]
    pub const WIN_NAME_ALWAYS: &str = "WinNameAlways"; // titre — [REA §4, RUNTIME: 1]
    pub const APP_THEME: &str = "AppTheme";   // "dark" | "light" — [DESIGN putty-rs §6]
    pub const LINE_SPACING_PCT: &str = "LineSpacingPct"; // int 0..25 — [DESIGN putty-rs §6]
}

/// Nom de la clé registre du slot i (ex. `Colour0`).
pub fn colour_key(i: usize) -> String {
    format!("{}{}", regkeys::COLOUR_PREFIX, i)
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Palette (22 slots) — source : THEME_DARK.md §1-2 + §5
// ═══════════════════════════════════════════════════════════════════════════

/// Palette complète PuTTY : 22 slots `Colour0..Colour21` — [REA].
///
/// Données : `pub cols: [Color32; 22]` (index = n° de slot registre tel que
/// THÈME_DARK.md le définit, voir la note `[AMBIGU]` ci-dessus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Slots 0..21, alignés sur les clés registre `Colour0..Colour21`.
    pub cols: [Color32; N_COLOUR_SLOTS],
}

impl Palette {
    /// 16 couleurs ANSI xterm/VGA — §1, communes aux deux thèmes (fidélité :
    /// identiques dans LEGACY et PRO_DARK, chacune réinscriptible comme PuTTY).
    const ANSI16: [Color32; 16] = [
        Color32::from_rgb(0x00, 0x00, 0x00), // 0  ANSI Black
        Color32::from_rgb(0x80, 0x00, 0x00), // 1  ANSI Red
        Color32::from_rgb(0x00, 0x80, 0x00), // 2  ANSI Green
        Color32::from_rgb(0x80, 0x80, 0x00), // 3  ANSI Yellow
        Color32::from_rgb(0x00, 0x00, 0x80), // 4  ANSI Blue
        Color32::from_rgb(0x80, 0x00, 0x80), // 5  ANSI Magenta
        Color32::from_rgb(0x00, 0x80, 0x80), // 6  ANSI Cyan
        Color32::from_rgb(0xc0, 0xc0, 0xc0), // 7  ANSI White
        Color32::from_rgb(0x55, 0x55, 0x55), // 8  Bright Black
        Color32::from_rgb(0xff, 0x55, 0x55), // 9  Bright Red
        Color32::from_rgb(0x55, 0xff, 0x55), // 10 Bright Green
        Color32::from_rgb(0xff, 0xff, 0x55), // 11 Bright Yellow
        Color32::from_rgb(0x55, 0x55, 0xff), // 12 Bright Blue
        Color32::from_rgb(0xff, 0x55, 0xff), // 13 Bright Magenta
        Color32::from_rgb(0x55, 0xff, 0xff), // 14 Bright Cyan
        Color32::from_rgb(0xff, 0xff, 0xff), // 15 Bright White
    ];

    /// Thème clair « Legacy PuTTY » — table complète §5 (colonne Legacy).
    ///
    /// [AMBIGU] Rép. à la question « 0=fg 0,0,0 / bold-fg 187,187,187 … » :
    /// ces valeurs amont historiques (texte gris `187,187,187` sur fond noir)
    /// sont explicitement déclarées **hors portée** par la note de §5 ; on suit
    /// l'observation REA : thème clair = fond blanc / police noire. Les slots
    /// 16..19 valent donc `#000000` (fg, bold fg) et `#ffffff` (bg, bold bg),
    /// 20 = glyphe du curseur `#ffffff`, 21 = curseur `#000000` (inversion).
    pub const LEGACY_PUTTY: Palette = Palette {
        cols: [
            Self::ANSI16[0],  Self::ANSI16[1],  Self::ANSI16[2],  Self::ANSI16[3],
            Self::ANSI16[4],  Self::ANSI16[5],  Self::ANSI16[6],  Self::ANSI16[7],
            Self::ANSI16[8],  Self::ANSI16[9],  Self::ANSI16[10], Self::ANSI16[11],
            Self::ANSI16[12], Self::ANSI16[13], Self::ANSI16[14], Self::ANSI16[15],
            Color32::from_rgb(0x00, 0x00, 0x00), // 16 fg        #000000
            Color32::from_rgb(0x00, 0x00, 0x00), // 17 bold fg   #000000
            Color32::from_rgb(0xff, 0xff, 0xff), // 18 bg        #ffffff
            Color32::from_rgb(0xff, 0xff, 0xff), // 19 bold bg   #ffffff
            Color32::from_rgb(0xff, 0xff, 0xff), // 20 cursor text (inversé) #ffffff
            Color32::from_rgb(0x00, 0x00, 0x00), // 21 cursor colour #000000
        ],
    };

    /// Thème sombre « Professional » — table complète §5 (colonne Professional).
    ///
    /// Fond `#1e1e1e` (§3.1 [DESIGN] : référence VS Code Dark+, évite le
    /// « creusement » du noir pur), fg `#d4d4d4` (anti-halo), cursor `#d4d4d4`
    /// avec glyphe inversé `#1e1e1e` (principe d'inversion PuTTY).
    pub const PRO_DARK: Palette = Palette {
        cols: [
            Self::ANSI16[0],  Self::ANSI16[1],  Self::ANSI16[2],  Self::ANSI16[3],
            Self::ANSI16[4],  Self::ANSI16[5],  Self::ANSI16[6],  Self::ANSI16[7],
            Self::ANSI16[8],  Self::ANSI16[9],  Self::ANSI16[10], Self::ANSI16[11],
            Self::ANSI16[12], Self::ANSI16[13], Self::ANSI16[14], Self::ANSI16[15],
            Color32::from_rgb(0xd4, 0xd4, 0xd4), // 16 fg        #d4d4d4
            Color32::from_rgb(0xff, 0xff, 0xff), // 17 bold fg   #ffffff
            Color32::from_rgb(0x1e, 0x1e, 0x1e), // 18 bg        #1e1e1e
            Color32::from_rgb(0x1e, 0x1e, 0x1e), // 19 bold bg   #1e1e1e
            Color32::from_rgb(0x1e, 0x1e, 0x1e), // 20 cursor text #1e1e1e
            Color32::from_rgb(0xd4, 0xd4, 0xd4), // 21 cursor colour #d4d4d4
        ],
    };

    // --- Accesseurs sémantiques (slots 16..21) — §2 -------------------------

    /// Avant-plan terminal par défaut (slot 16).
    pub const fn fg(&self) -> Color32 { self.cols[IDX_FG] }
    /// Avant-plan en gras (slot 17).
    pub const fn bold_fg(&self) -> Color32 { self.cols[IDX_BOLD_FG] }
    /// Arrière-plan terminal (slot 18).
    pub const fn bg(&self) -> Color32 { self.cols[IDX_BG] }
    /// Arrière-plan en gras (slot 19).
    pub const fn bold_bg(&self) -> Color32 { self.cols[IDX_BOLD_BG] }
    /// *Cursor Text* — glyphe dessiné dans le curseur plein (slot 20).
    pub const fn cursor_text(&self) -> Color32 { self.cols[IDX_CURSOR_TEXT] }
    /// *Cursor Colour* — remplissage du curseur (slot 21).
    pub const fn cursor_colour(&self) -> Color32 { self.cols[IDX_CURSOR_COLOUR] }

    /// Vue sur les 16 couleurs ANSI (slots 0..15) — §1.
    pub fn ansi16(&self) -> &[Color32] { &self.cols[0..16] }

    /// Sélection du terminal : gris 50 % `#808080` — §5, identique partout,
    /// NE passe PAS par `egui::Visuals::selection` (celle-là est pour l'UI).
    pub const fn sel_bg(&self) -> Color32 { SEL_BG }

    // --- Parsing registre : format `"r,g,b"` (décimal) — [REA §6 doc] -------

    /// Analyse `"r,g,b"` (décimal, espaces/largeurs libres tolérées).
    /// `None` si format invalide ou composante hors 0..=255.
    pub fn parse_rgb(s: &str) -> Option<Color32> {
        let mut parts = [0u8; 3];
        let mut n = 0;
        for raw in s.split(',') {
            let v: u8 = raw.trim().parse().ok()?;
            // PuTTY n'écrit que 3 composantes ; on refuse au-delà (strict).
            match n {
                0 | 1 | 2 => parts[n] = v,
                _ => return None,
            }
            n += 1;
        }
        if n == 3 {
            Some(Color32::from_rgb(parts[0], parts[1], parts[2]))
        } else {
            None
        }
    }

    /// Sérialise vers le format registre `"r,g,b"` (les 22 slots, ordre 0..21).
    pub fn to_registry_values(&self) -> [String; N_COLOUR_SLOTS] {
        std::array::from_fn(|i| {
            let c = self.cols[i];
            format!("{},{},{}", c.r(), c.g(), c.b())
        })
    }

    /// Constructeur depuis les valeurs registre — surcharge « Option par slot ».
    /// `values[i] == Some("r,g,b")` remplace le slot i ; `None` garde le slot
    /// de `base` (chargement partiel d'un profil, comme PuTTY).
    /// Compte/contenu invalide → `Err(PaletteError)`.
    pub fn from_registry_values(
        base: &Palette,
        values: &[Option<&str>],
    ) -> Result<Palette, PaletteError> {
        if values.len() != N_COLOUR_SLOTS {
            return Err(PaletteError::WrongCount { got: values.len() });
        }
        let mut out = *base;
        for (i, slot) in values.iter().enumerate() {
            if let Some(raw) = slot {
                out.cols[i] = Self::parse_rgb(raw)
                    .ok_or_else(|| PaletteError::BadFormat { index: i, value: (*raw).to_owned() })?;
            }
        }
        Ok(out)
    }

    /// Constructeur depuis les valeurs registre — surcharge « 22 chaînes d'un
    /// coup » (lecture complète d'un profil déjà rempli).
    pub fn from_registry_strings(values: &[&str; N_COLOUR_SLOTS]) -> Result<Palette, PaletteError> {
        let opts: [Option<&str>; N_COLOUR_SLOTS] = std::array::from_fn(|i| Some(values[i]));
        Self::from_registry_values(&Palette::LEGACY_PUTTY, &opts)
    }
}

/// Erreurs de lecture registre pour la palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteError {
    /// Pas exactement 22 entrées.
    WrongCount { got: usize },
    /// Valeur `"r,g,b"` illisible au slot `index`.
    BadFormat { index: usize, value: String },
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Thèmes applicatifs + réglages « pros » (THEME_DARK.md §4/§6/§7)
// ═══════════════════════════════════════════════════════════════════════════

/// Thème d'application du chrome UI (le contenu terminal suit la palette).
/// Persistance : clé `AppTheme` = `"dark" | "light"` — §6 [DESIGN putty-rs].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTheme {
    /// Sombre « Professional » (défaut).
    Dark,
    /// Clair « Legacy PuTTY ».
    Light,
}

impl AppTheme {
    /// Chaîne stockée en registre (`AppTheme`) — §6.
    pub const fn registry_str(&self) -> &'static str {
        match self {
            AppTheme::Dark => "dark",
            AppTheme::Light => "light",
        }
    }
    /// Lecture tolérante : toute valeur ≠ `"light"` retombe sur Dark
    /// (défaut §6 snippet : `_ => Theme::professional()`).
    pub fn from_registry_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("light") { AppTheme::Light } else { AppTheme::Dark }
    }
}

/// Réglages cosmétiques de session (paint terminal + chrome UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Clair ou sombre.
    pub app: AppTheme,
    /// Les 22 slots couleur de la session.
    pub palette: Palette,
    /// Padding de la grille de texte, px (tous côtés) — 8 §7 [DESIGN].
    pub cell_pad: u8,
    /// Interligne ajouté en % de la hauteur de cellule, 0..25 — clé LineSpacingPct §6.
    pub line_spacing_pct: u16,
    /// Durée du flash visuel (VISBEL) en ms — 100 §4 [doc PuTTY].
    pub bell_visual_ms: u16,
    /// Période de clignotement du curseur en ms — 530 §4 [DESIGN].
    pub cursor_blink_ms: u16,
}

impl Theme {
    /// Thème sombre par défaut (palette PRO_DARK + réglages §7).
    pub const fn professional() -> Theme { DEFAULT_DARK }
    /// Thème clair legacy (palette LEGACY_PUTTY + police Courier New).
    pub const fn legacy() -> Theme { DEFAULT_LIGHT }

    /// Interligne borné au domaine clé LineSpacingPct (0..25) — §6.
    pub fn clamped_line_spacing(&self) -> u16 {
        self.line_spacing_pct.min(MAX_LINE_SPACING_PCT)
    }

    /// Facteur d'interligne à passer au rendu de cellules :
    /// `cell_h = (ascent + descent) * (1.0 + pct/100).round()` — §7.
    pub fn line_spacing_factor(&self) -> f32 {
        1.0 + self.clamped_line_spacing() as f32 / 100.0
    }

    // [AMBIGU] Le mode cloche n'est PAS un champ de Theme (contrat du
    // ticket : la clé registre `BellType` est encodée/décodée par BellMode
    // en section 5, et app/config.rs décident du mode à la connexion).
}

// --- Polices par défaut (§7) — appliquées via FontDefinitions au démarrage ---
// [AMBIGU] La structure Theme (contrat du ticket) ne porte pas de champs
// police ; ces constantes accompagnent DEFAULT_DARK/DEFAULT_LIGHT pour que
// app.rs puisse monter les fontes : consola.ttf (+ consolb.ttf pour le gras,
// repli Cascadia Mono), Courier New pour le clair legacy (clé `Font` [REA §4]).
/// Police par défaut du thème sombre : Consolas — §7 [DESIGN].
pub const DEFAULT_FONT_DARK: &str = "Consolas";
/// Corps de la police sombre : 11 pt (≈ 14,66 px @96 dpi) — §7.
pub const DEFAULT_FONT_DARK_PT: f32 = 11.0;
/// Police legacy (thème clair) observée [RUNTIME REA §4] : Courier New.
pub const DEFAULT_FONT_LIGHT: &str = "Courier New";
/// Corps legacy : obs. runtime FontHeight=10 (défaut doc 0.83 : 18) — [REA].
pub const DEFAULT_FONT_LIGHT_PT: f32 = 10.0;

/// Thème par défaut « Professional » — §7.
pub const DEFAULT_DARK: Theme = Theme {
    app: AppTheme::Dark,
    palette: Palette::PRO_DARK,
    cell_pad: DEFAULT_CELL_PAD,           // 8 px
    line_spacing_pct: DEFAULT_LINE_SPACING_PCT,
    bell_visual_ms: VISBEL_MS,            // 100 ms
    cursor_blink_ms: DEFAULT_CURSOR_BLINK_MS, // 530 ms
};

/// Thème par défaut « Legacy PuTTY » (clair).
pub const DEFAULT_LIGHT: Theme = Theme {
    app: AppTheme::Light,
    palette: Palette::LEGACY_PUTTY,
    cell_pad: DEFAULT_CELL_PAD,           // mêmes réglages que le sombre
    line_spacing_pct: DEFAULT_LINE_SPACING_PCT,
    bell_visual_ms: VISBEL_MS,
    cursor_blink_ms: DEFAULT_CURSOR_BLINK_MS,
};

// ═══════════════════════════════════════════════════════════════════════════
// 3. Pont egui — surcharge du style global (chrome) — THEME_DARK.md §3.2
// ═══════════════════════════════════════════════════════════════════════════

/// Applique le style egui global correspondant au thème `t`.
///
/// Portée : le **chrome de l'app** (panneaux, widgets, scroll). Le contenu du
/// terminal est peint à part, cellule par cellule, avec `Theme.palette` et
/// `SEL_BG` (la sélection UI ≠ sélection terminal — §3.2 note).
/// Chaque override forme un micro-bloc commenté → correction humaine facile.
pub fn apply_egui_style(ctx: &egui::Context, t: &Theme) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;

    match t.app {
        AppTheme::Dark => apply_dark(v, t),
        AppTheme::Light => apply_light(v, t),
    }

    // Scrollbar : fine, discrète, épaisseur « pro » — §3.2. (Les marges
    // bar_inner/bar_outer peuvent être réduites ici si besoin : ScrollStyle
    // expose bar_inner_margin etc. — à affiner avec app.rs.)
    if let AppTheme::Dark = t.app {
        // [AMBIGU] spec §3.2 : `ScrollStyle::thin()` (+ bar_inner_margin
        // réduits, valeur non fixée) — appliqué seulement au sombre.
        // NB egui 0.31 : ScrollStyle n'est pas à la racine, module style.
        style.spacing.scroll = egui::style::ScrollStyle::thin();
    }

    ctx.set_style(style);
}

/// Overrides du thème sombre — §3.2 (snippets du spec, commentés bloc par bloc).
fn apply_dark(v: &mut egui::Visuals, _t: &Theme) {
    use egui::{Rounding, Stroke};

    v.dark_mode = true;

    // Fond de fenêtre/panneaux : un cran plus sombre que le terminal (#1e1e1e)
    // pour le détacher — §3.2 [DESIGN #171717].
    v.window_fill = Color32::from_rgb(0x17, 0x17, 0x17);
    v.panel_fill = Color32::from_rgb(0x17, 0x17, 0x17); // barre titre + barre d'état

    // Debug / code / piste de scrollbar — §3.2.
    v.extreme_bg_color = Color32::from_rgb(0x0f, 0x0f, 0x0f);
    // Alternance « zebra » — §3.2.
    v.faint_bg_color = Color32::from_rgb(0x1f, 0x1f, 0x1f);

    // Sélection (UI) : bleu VS Code sobre — §3.2 [DESIGN]. NB : egui 0.31 ne
    // comporte pas de `selection.text_color` (mentionné au pseudo-code spec) ;
    // le texte garde sa couleur → cf. `sel_text_keep: true` (§3.3).
    v.selection.bg_fill = Color32::from_rgb(0x26, 0x4f, 0x78);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x3f, 0x63, 0x8b));

    // Indications discrètes : egui 0.31 ne connait pas de champ « hint »
    // réglable — `Visuals::weak_text_color()` est dérivé ; on laisse le
    // sombre par défaut [FIX-BUILD 27/10, le champ hint_text_color n'existe
    // pas dans cette version].
    // Liens hypertexte — §3.2.
    v.hyperlink_color = Color32::from_rgb(0x56, 0x9c, 0xd6);

    // Widgets : resserrer les visuels standard — §3.2.
    // (inactive = au repos, hovered = survol, active = pressé/actif ; egui n'a
    // pas de état « pressed » séparé, `active` le couvre.)
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x22, 0x22, 0x22);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x2a, 0x2a, 0x2a);
    v.widgets.active.bg_fill = Color32::from_rgb(0x33, 0x33, 0x33);

    // Séparateurs discrets sur widgets non interactifs — §3.2.
    // [AMBIGU] Le ticket mentionnait `noninteractive.bg_contrast_panels`,
    // champ inexistant en egui 0.31 ; le spec §3.2 utilise `bg_stroke` ci-dessous.
    v.widgets.noninteractive.bg_stroke =
        Stroke::new(0.6_f32, Color32::from_rgb(0x33, 0x36, 0x3b));

    // Contour de fenêtre 1 px `#2b2b2b`, pas d'ombre — §3.2/§3.3.
    // (egui 0.31 n'a pas de `panel_stroke` : le trait de fenêtre suffit
    // [FIX-BUILD 27/10].)
    v.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x2b, 0x2b, 0x2b));
    v.window_shadow = egui::epaint::Shadow::NONE;

    // Arrondi de fenêtres discret (2 px) — cohérent badges §3.5.
    // egui 0.31 : `Visuals::window_corner_radius` (CornerRadius::same(u8)).
    v.window_corner_radius = Rounding::same(2);

    // Curseur texte des champs d'édition : gris du terminal — §3.2
    // (0.31 : text_cursor.stroke, pas de champ .color [FIX-BUILD]).
    v.text_cursor.stroke = Stroke::new(2.0_f32, Color32::from_rgb(0xd4, 0xd4, 0xd4));
}

/// Overrides du thème clair legacy — [AMBIGU] NON SPÉCIFIÉ dans THEME_DARK.md
/// (la §3 ne code que le sombre). Choix minimal, basé §5 : fond blanc,
/// chrome discret, pas d'ombre ; à ajuster librement par un humain.
fn apply_light(v: &mut egui::Visuals, _t: &Theme) {
    use egui::{Rounding, Stroke};

    v.dark_mode = false;

    // Chromage blanc « PuTTY historique » — §5 (fond blanc).
    v.window_fill = Color32::WHITE;
    v.panel_fill = Color32::WHITE;
    v.extreme_bg_color = Color32::from_rgb(0xe0, 0xe0, 0xe0); // pistes/debug
    v.faint_bg_color = Color32::from_rgb(0xf0, 0xf0, 0xf0);   // zebra

    // Sélection UI : bleu clair neutre [DESIGN, hors spec].
    v.selection.bg_fill = Color32::from_rgb(0xa8, 0xc0, 0xe8);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x7a, 0x96, 0xc8));

    // Indications discrètes : pas de champ « hint » réglable en egui 0.31
    // [FIX-BUILD] ; liens hypertexte bleus Win classique — §5.
    v.hyperlink_color = Color32::from_rgb(0x06, 0x44, 0xc9); // bleu Win classique

    // Widgets : gardés egui defaults (clairs), pas d'override [DESIGN].
    // Contours : gris faible, pas d'ombre (fenêtre PuTTY : liseré simple).
    v.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xaa, 0xaa, 0xaa));
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.window_corner_radius = Rounding::same(0); // PuTTY : angles droits

    // Curseur texte : noir comme l'ancienne police — [DESIGN, 0.31 stroke].
    v.text_cursor.stroke = Stroke::new(2.0_f32, Color32::from_rgb(0x00, 0x00, 0x00));
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Couleurs étendues xterm 256 — THEME_DARK.md §1 (rendu 256 couleurs)
// ═══════════════════════════════════════════════════════════════════════════

/// Table 256 couleurs xterm, construite depuis un `Palette`.
///
/// Mapping documenté (§1) :
///   - 0..15   → slots ANSI du `Palette` (0..15) ;
///   - 16..231 → cube 6×6×6 : `index = 16 + 36r + 6g + b`, niveaux
///     `CUBE_LEVELS = [0, 95, 135, 175, 215, 255]` (« pas de 0/255/0x33 » :
///     c'est bien 95/135/175/215 entre les deux) — standard xterm [REA].
///   - 232..255 → rampe de 24 gris `8 + 10*n` (232→#080808 … 255→#e8e8e8).
///
/// Les slots 16..21 du `Palette` (fg/bg/curseur…) **n'entrent PAS** dans la
/// table 256 : ce sont des couleurs de configuration, pas des indices ANSI.
/// Tout indice ≥ 256 est clampé (`color()`), jamais paniqué.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ansi256(pub [Color32; 256]);

impl Ansi256 {
    /// Niveaux de composante du cube 6×6×6 xterm.
    pub const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    /// Construit la table complète à partir de la palette courante.
    pub fn from_palette(p: &Palette) -> Ansi256 {
        let mut cols = [Color32::BLACK; 256];

        // 0..15 : copie des slots ANSI (0..15) de la palette.
        for i in 0..16 {
            cols[i] = p.cols[i];
        }

        // 16..231 : cube 6×6×6 (formule xterm standard).
        for r in 0..6u16 {
            for g in 0..6u16 {
                for b in 0..6u16 {
                    let idx = 16 + 36 * r + 6 * g + b;
                    cols[idx as usize] = Color32::from_rgb(
                        Self::CUBE_LEVELS[r as usize],
                        Self::CUBE_LEVELS[g as usize],
                        Self::CUBE_LEVELS[b as usize],
                    );
                }
            }
        }

        // 232..255 : rampe de 24 gris « 8 + 10*n » (formule xterm).
        for n in 0..24u16 {
            let v = (8 + 10 * n).min(255) as u8; // 232+n ≤ 255 → jamais > 248
            cols[(232 + n) as usize] = Color32::from_rgb(v, v, v);
        }

        Ansi256(cols)
    }

    /// Couleur d'un indice ANSI, avec clamp (jamais de panic si ≥ 256).
    pub fn color(&self, index: u16) -> Color32 {
        self.0[(index as usize).min(255)]
    }
}

impl core::ops::Index<u16> for Ansi256 {
    type Output = Color32;
    fn index(&self, index: u16) -> &Color32 { &self.0[(index as usize).min(255)] }
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Mode cloche (BellType) — THEME_DARK.md §4, clés [REA §4]
// ═══════════════════════════════════════════════════════════════════════════

/// Trois modes de cloche, stockés en int à la clé `BellType` — §4.
/// Note REA : `Beep` (bip système) et `BeepInd` (flash visuel par indice) sont
/// aussi présentes dans le binaire 0.83 ; l'interface registre retenue reste
/// `BellType` (décision §4 note).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BellMode {
    /// Bip système `MessageBeep` (FlashWindow au BEL côté REA).
    System,
    /// Rien.
    None,
    /// Flash inverse `bell_visual_ms` ms (§4 : vrai échange fg/bg, pas d'alpha).
    Visual,
}

impl BellMode {
    /// int registre (`BellType`) : 0=SYSTEM, 1=NONE, 2=VISBEL — §4.
    pub const fn to_bell_type(self) -> u32 {
        match self {
            BellMode::System => 0,
            BellMode::None => 1,
            BellMode::Visual => 2,
        }
    }
    /// Lecture tolérante du registre (hors-domaine → System, défaut 0.83).
    pub const fn from_bell_type(v: u32) -> Self {
        match v {
            1 => BellMode::None,
            2 => BellMode::Visual,
            _ => BellMode::System,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Tests unitaires
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Color32 → hex #rrggbb (assertion lisible).
    fn hex(c: Color32) -> String {
        format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
    }

    #[test]
    fn ansi16_reads_the_theme_dark_table() {
        // Table §1 recopiée telle quelle (ANSI 0..7 puis Bright 8..15).
        let std16 = [
            "#000000", "#800000", "#008000", "#808000",
            "#000080", "#800080", "#008080", "#c0c0c0",
            "#555555", "#ff5555", "#55ff55", "#ffff55",
            "#5555ff", "#ff55ff", "#55ffff", "#ffffff",
        ];
        for pal_name in [("LEGACY", Palette::LEGACY_PUTTY), ("PRO_DARK", Palette::PRO_DARK)] {
            for (i, want) in std16.iter().enumerate() {
                assert_eq!(&hex(pal_name.1.cols[i]), want, "{} slot {i}", pal_name.0);
            }
        }
    }

    #[test]
    fn ansi16_identical_in_both_themes() {
        // Fidélité §1 : les 16 couleurs sont communes aux deux thèmes.
        assert_eq!(Palette::LEGACY_PUTTY.ansi16(), Palette::PRO_DARK.ansi16());
    }

    #[test]
    fn system_slots_match_theme_table() {
        // Table §5 (slots 16..21) :
        let legacy = [
            "#000000", "#000000", "#ffffff", "#ffffff", "#ffffff", "#000000",
        ];
        let dark = [
            "#d4d4d4", "#ffffff", "#1e1e1e", "#1e1e1e", "#1e1e1e", "#d4d4d4",
        ];
        for i in 0..6 {
            assert_eq!(hex(Palette::LEGACY_PUTTY.cols[16 + i]), legacy[i as usize].to_string());
            assert_eq!(hex(Palette::PRO_DARK.cols[16 + i]), dark[i as usize].to_string());
        }
        // Sélection terminal : gris 50 % partout — §5.
        assert_eq!(hex(SEL_BG), "#808080");
        assert_eq!(hex(Palette::PRO_DARK.sel_bg()), "#808080");
    }

    #[test]
    fn registry_round_trip() {
        for base in [Palette::LEGACY_PUTTY, Palette::PRO_DARK] {
            let vals = base.to_registry_values();
            let refs: [&str; N_COLOUR_SLOTS] =
                vals.iter().map(|s| s.as_str()).collect::<Vec<_>>().try_into().unwrap();
            let back = Palette::from_registry_strings(&refs).expect("round-trip");
            assert_eq!(back, base, "round-trip registre");
        }
    }

    #[test]
    fn partial_registry_load_keeps_base_slots() {
        // Surcharge d'un seul slot (le fg sombre), le reste intact.
        let values: [Option<&str>; N_COLOUR_SLOTS] = {
            let mut v = [None; N_COLOUR_SLOTS];
            v[IDX_FG] = Some(" 30 , 30 , 30 "); // espaces libres tolérés
            v
        };
        let p = Palette::from_registry_values(&Palette::PRO_DARK, &values).unwrap();
        // Le slot surchargé devient "30,30,30" = #1e1e1e (ex. fond sombre
        // injecté en fg) ; le reste de la base est conservé tel quel.
        assert_eq!(hex(p.fg()), "#1e1e1e");
        assert_eq!(p.cols[IDX_FG], Color32::from_rgb(30, 30, 30));
        // Slots non fournis : inchangés (bold fg blanc).
        assert_eq!(p.bold_fg(), Color32::WHITE);
    }

    #[test]
    fn registry_rejects_garbage() {
        let values: [Option<&str>; N_COLOUR_SLOTS] = {
            let mut v = [None; N_COLOUR_SLOTS];
            v[3] = Some("255,0"); // incomplet
            v
        };
        assert_eq!(
            Palette::from_registry_values(&Palette::LEGACY_PUTTY, &values),
            Err(PaletteError::BadFormat { index: 3, value: "255,0".to_owned() })
        );
        let values: [Option<&str>; N_COLOUR_SLOTS] = {
            let mut v = [None; N_COLOUR_SLOTS];
            v[5] = Some("999,0,0"); // hors 0..=255
            v
        };
        assert!(Palette::from_registry_values(&Palette::LEGACY_PUTTY, &values).is_err());
        assert_eq!(
            Palette::from_registry_values(&Palette::LEGACY_PUTTY, &[]),
            Err(PaletteError::WrongCount { got: 0 })
        );
    }

    #[test]
    fn ansi256_cube_196_is_red() {
        // xterm-196 : 16 + 36*5 + 0 + 0 → niveaux (5,0,0) = #ff0000.
        let a = Ansi256::from_palette(&Palette::PRO_DARK);
        assert_eq!(hex(a[196]), "#ff0000");
        // Sanity : coin opposé 16 + 0 + 0 + 0 ? -> 16 = noir ; 231 = blanc cubique.
        assert_eq!(hex(a[16]), "#000000");
        assert_eq!(hex(a[231]), "#ffffff");
        // Rampe : 232=#080808, 255=#eeeeee (formule xterm 8 + 10·(n-232),
        // n=255 → 238 = 0xee [FIX-BUILD: l'attendu précédent #e8 confusion
        // index/valeur]).
        assert_eq!(hex(a[232]), "#080808");
        assert_eq!(hex(a[255]), "#eeeeee");
    }

    #[test]
    fn ansi256_gray_ramp_is_sorted_and_increasing() {
        let a = Ansi256::from_palette(&Palette::LEGACY_PUTTY);
        for n in 232..255usize {
            assert_less(a.0[n].r(), a.0[n + 1].r(), "rampe grise strictement croissante");
            assert_eq!(a.0[n].r(), a.0[n].g());
            assert_eq!(a.0[n].r(), a.0[n].b());
        }
    }

    #[test]
    fn ansi256_slots_0_15_follow_palette() {
        let mut p = Palette::PRO_DARK;
        p.cols[1] = Color32::from_rgb(1, 2, 3); // simulation de reconfig OSC
        let a = Ansi256::from_palette(&p);
        assert_eq!(a[1], Color32::from_rgb(1, 2, 3));
        // Clamp hors domaine : aucun panic.
        assert_eq!(a.color(9999), a.0[255]);
    }

    #[test]
    fn apptheme_registry_round_trip() {
        assert_eq!(AppTheme::Dark.registry_str(), "dark");
        assert_eq!(AppTheme::Light.registry_str(), "light");
        assert_eq!(AppTheme::from_registry_str("light"), AppTheme::Light);
        assert_eq!(AppTheme::from_registry_str("DARK"), AppTheme::Dark);
        assert_eq!(AppTheme::from_registry_str("nimporte"), AppTheme::Dark); // défaut §6
    }

    #[test]
    fn bell_type_round_trip() {
        assert_eq!(BellMode::System.to_bell_type(), 0);
        assert_eq!(BellMode::None.to_bell_type(), 1);
        assert_eq!(BellMode::Visual.to_bell_type(), 2);
        assert_eq!(BellMode::from_bell_type(2), BellMode::Visual);
        assert_eq!(BellMode::from_bell_type(42), BellMode::System); // défaut
    }

    #[test]
    fn theme_defaults_match_spec() {
        // §7 : padding 8 px, blink 530 ms, visual bell 100 ms.
        assert_eq!(DEFAULT_DARK.cell_pad, 8);
        assert_eq!(DEFAULT_DARK.cursor_blink_ms, 530);
        assert_eq!(DEFAULT_DARK.bell_visual_ms, 100);
        // Polices §7 / [RUNTIME REA].
        assert_eq!(DEFAULT_FONT_DARK, "Consolas");
        assert_eq!(DEFAULT_FONT_LIGHT, "Courier New");
        // Interligne borné.
        assert_eq!(MAX_LINE_SPACING_PCT, 25);
        assert_eq!(DEFAULT_LIGHT.clamped_line_spacing(), 0);
    }

    // Petit helper d'assert strictement croissant (évite un panique interne).
    fn assert_less(a: u8, b: u8, msg: &'static str) {
        assert!(a < b, "{msg}: {a} !< {b}");
    }
}
