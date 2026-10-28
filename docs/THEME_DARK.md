# THÈME SOMBRE « Professional » — Spécification cosmétique (egui/eframe 0.31)

Statut : prêt-à-coder (snippets **plausibles, non compilés**). Portée : cosmétique uniquement.
Base fidélité : PuTTY 0.83 — palette ANSI stockée en registre sous `Colour0..Colour21`,
support `Xterm256Colour`, jeu de 6 couleurs supplémentaires (index 16-21).
Conventions : les valeurs `[REA]` proviennent de l'analyse du binaire 0.83 ; `[doc PuTTY]`
suit la table ANSI/les défauts documentés de PuTTY ; `[DESIGN]` = choix libres du client.

## 1. Palette ANSI 16 (index 0..15) — commune aux deux thèmes

Fidélité stricte : les 16 couleurs sont **identiques dans les deux thèmes** (configurables
individuellement, comme PuTTY).

| Idx | Nom            | Hex       | Idx | Nom (bold)      | Hex       |
|----:|----------------|-----------|----:|-----------------|-----------|
|  0  | ANSI Black     | `#000000` |  8  | Bright Black    | `#555555` |
|  1  | ANSI Red       | `#800000` |  9  | Bright Red      | `#ff5555` |
|  2  | ANSI Green     | `#008000` | 10  | Bright Green    | `#55ff55` |
|  3  | ANSI Yellow    | `#808000` | 11  | Bright Yellow   | `#ffff55` |
|  4  | ANSI Blue      | `#000080` | 12  | Bright Blue     | `#5555ff` |
|  5  | ANSI Magenta   | `#800080` | 13  | Bright Magenta  | `#ff55ff` |
|  6  | ANSI Cyan      | `#008080` | 14  | Bright Cyan     | `#55ffff` |
|  7  | ANSI White     | `#c0c0c0` | 15  | Bright White    | `#ffffff` |

```rust
pub const ANSI16: [egui::Color32; 16] = [
    Color32::from_rgb(0x00, 0x00, 0x00), Color32::from_rgb(0x80, 0x00, 0x00),
    Color32::from_rgb(0x00, 0x80, 0x00), Color32::from_rgb(0x80, 0x80, 0x00),
    Color32::from_rgb(0x00, 0x00, 0x80), Color32::from_rgb(0x80, 0x00, 0x80),
    Color32::from_rgb(0x00, 0x80, 0x80), Color32::from_rgb(0xc0, 0xc0, 0xc0),
    Color32::from_rgb(0x55, 0x55, 0x55), Color32::from_rgb(0xff, 0x55, 0x55),
    Color32::from_rgb(0x55, 0xff, 0x55), Color32::from_rgb(0xff, 0xff, 0x55),
    Color32::from_rgb(0x55, 0x55, 0xff), Color32::from_rgb(0xff, 0x55, 0xff),
    Color32::from_rgb(0x55, 0xff, 0xff), Color32::from_rgb(0xff, 0xff, 0xff),
];
```

Le rendu 256 couleurs (`Xterm256Colour` `[REA]`) reste standard : 0..15 → table ci-dessus,
16..231 → cube 6×6×6 (séquence `16 + 36r + 6g + b`, pas de 0/255/0x33), 232..255 → rampes de gris.

## 2. Index 16..21 — les 6 couleurs « système » PuTTY

Sémantique `[bin PuTTY 0.83 / doc PuTTY]` ; les **valeurs** dépendent du thème courant :

| Idx | Registre    | Sémantique                                            |
|----:|-------------|--------------------------------------------------------|
| 16  | `Colour16`  | Avant-plan par défaut (texte « normal »)              |
| 17  | `Colour17`  | Avant-plan par défaut en **gras** (fonte bold)        |
| 18  | `Colour18`  | Arrière-plan par défaut (fond du terminal)            |
| 19  | `Colour19`  | Arrière-plan par défaut en gras                       |
| 20  | `Colour20`  | *Cursor Text* — glyphe dessiné DANS le curseur plein  |
| 21  | `Colour21`  | *Cursor Colour* — remplissage du curseur              |

Ces 6 couleurs sont réinscriptibles par l'hôte (`OSC 10/11/17/19`, `OSC 12`) si l'option
« Allow terminal re-colouring » (doc PuTTY) est activée.

## 3. Thème sombre « Professional »

### 3.1 Choix du fond `[DESIGN]`

- **Fond terminal `#1e1e1e`** (RGB 30,30,30). Justification : avec la palette
  ANSI fidéliste ci-dessus, un noir pur fait « creuser » l'écran et brutalise les contrastes ;
  `#1e1e1e` est la référence d'industrie des terminals « pro » (VS Code Dark+), fatigue moins
  à long terme et garde un bon rendu du noir ANSI pur sans effet de halo.
- Avant-plan `#d4d4d4` : gris haut-contraste mais pas blanc pur (anti-halo, confort APCA).
- Curseur plein `#d4d4d4` + glyphe inversé `#1e1e1e` (fidèle au principe d'inversion PuTTY).

```rust
pub struct TerminalPaint {
    pub fg:   Color32, pub bg:   Color32,
    pub bold_fg: Color32, pub bold_bg: Color32,
    pub cursor_fg: Color32, pub cursor_color: Color32, // 20/21
}
pub const PROF: TerminalPaint = TerminalPaint {
    fg: Color32::from_rgb(0xd4, 0xd4, 0xd4),  bg: Color32::from_rgb(0x1e, 0x1e, 0x1e),
    bold_fg: Color32::from_rgb(0xff, 0xff, 0xff), bold_bg: Color32::from_rgb(0x1e, 0x1e, 0x1e),
    cursor_fg: Color32::from_rgb(0x1e, 0x1e, 0x1e), cursor_color: Color32::from_rgb(0xd4, 0xd4, 0xd4),
};
```

### 3.2 Overrides `egui::Visuals` ( Style global, chrome de l'app ≠ contenu du terminal )

```rust
fn apply_professional_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    // Fond de fenêtre/panneaux : un cran plus sombre que le terminal (le détache)
    v.window_fill  = Color32::from_rgb(0x17, 0x17, 0x17); // [DESIGN] #171717
    v.panel_fill   = Color32::from_rgb(0x17, 0x17, 0x17); // barre titre + barre d'état
    v.extreme_bg_color = Color32::from_rgb(0x0f, 0x0f, 0x0f); // debug/code/track scrollbar
    v.faint_bg_color   = Color32::from_rgb(0x1f, 0x1f, 0x1f); // alternance « zebra »
    // Sélection (UI) : bleu vs-code sobre [DESIGN]
    v.selection.bg          = Color32::from_rgb(0x26, 0x4f, 0x78);
    v.selection.stroke      = egui::Stroke::new(1.0, Color32::from_rgb(0x3f, 0x63, 0x8b));
    v.selection.text_color  = Color32::from_rgb(0xd4, 0xd4, 0xd4);
    // Indications discrètes
    v.hint_text_color = Color32::from_rgb(0x60, 0x64, 0x6a); // placeholders / hint widgets
    v.hyperlink_color = Color32::from_rgb(0x56, 0x9c, 0xd6);
    // Widgets : resserrer les visuels standard
    v.widgets.inactive.bg_fill    = Color32::from_rgb(0x22, 0x22, 0x22);
    v.widgets.hovered.bg_fill     = Color32::from_rgb(0x2a, 0x2a, 0x2a);
    v.widgets.active.bg_fill      = Color32::from_rgb(0x33, 0x33, 0x33);
    v.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(0.6, Color32::from_rgb(0x33, 0x36, 0x3b)); // séparateurs discrets
    v.window_stroke = egui::Stroke::new(1.0, Color32::from_rgb(0x2b, 0x2b, 0x2b));
    v.panel_stroke  = egui::Stroke::NONE;                 // transition sans trait entre panneaux
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.text_cursor.color = Color32::from_rgb(0xd4, 0xd4, 0xd4);
    ctx.set_style(style);
    // Scrollbar : fine, discrète (épaisseur « pro »)
    style.spacing.scroll = egui::style::ScrollStyle::thin(); // + bar_inner_margin réduits
    ctx.set_style(style);
}
```

> La **sélection de texte du terminal** (bloc rouge-vert de PuTTY) NE passe PAS par
> `v.selection` : elle est peinte dans la grille avec la couleur de sélection terminal
> dédiée (§3.3) — fidélité PuTTY (sélection grise) vs notre variante sombre.

### 3.3 Bordures, scroll, sélection terminal `[DESIGN]`

```rust
pub const PROF_WINDOW: WindowDefn = WindowDefn {
    border:        Color32::from_rgb(0x2b, 0x2b, 0x2b), // contour fenêtre 1px
    border_width:  1.0,
    splitter:      Color32::from_rgb(0x2b, 0x2b, 0x2b),
    scrollbar_w:   10.0,
    scrollbar_fg:  Color32::from_rgb(0x3c, 0x3c, 0x3c), // pouce
    scrollbar_bg:  Color32::from_rgb(0x17, 0x17, 0x17), // rail
    sel_bg:        Color32::from_rgb(0x26, 0x4f, 0x78), // sélection terminal sombre
    sel_text_keep: true, // le texte garde sa couleur de cellule (comportement egui/Web)
};
```

### 3.4 Fenêtre sans bordure + barre de titre custom

`[DESIGN]` — remplace le chrome système par une barre 28 px, drag natif :

```rust
// eframe (au lancement) : deco désactivée
let win = egui::ViewportBuilder::default().with_decorations(false).with_inner_size([900.0, 600.0]);
eframe::run_native("putty-rs", eframe::NativeOptions { viewport: win, ..Default::default() }, ...);

// à chaque frame :
egui::TopBottomPanel::new(TopBottomSide::Top, "titlebar").exact_height(28.0).show(ctx, |ui| {
    ui.style_mut().visuals.panel_fill = panel_fill;
    ui.horizontal_centered(|ui| {
        // zone gauche : icône + titre de session ; hitzone centrale = drag natif
        if let Some(tab_label) = current_label { ui.add(Label::new(RichText::new(tab_label))); }
        let rest = ui.available_width() - 90.0;
        let (_rect, resp) = ui.allocate_exact_size(
            egui::Vec2::new(rest, ui.available_height()), egui::Sense::click_and_drag());
        if resp.double_clicked() { // double-clic = maximiser (convention Windows)
            let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
        }
        if resp.drag_started() { ctx.send_viewport_cmd(ViewportCommand::StartDrag); }
        // boutons – □ ✕
        if ui.small_button("–").clicked() { ctx.send_viewport_cmd(ViewportCommand::Minimized(true)); }
        if ui.small_button("□").clicked() {
            let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
        }
        if ui.small_button("✕").clicked() { ctx.send_viewport_cmd(ViewportCommand::Close); }
    });
});
```

### 3.5 Badges d'état de connexion (barre d'état, §7)

| Badge        | Label exemple              | Couleur de fond | Contour            |
|--------------|---------------------------|-----------------|--------------------|
| État         | `CONNECTÉ` / `CONNECTION…` / `FERMÉ` | `#2d5a3d` / `#5c4a1e` / `#6b2c2c` | arrondi 2 px |
| Chiffrement  | `aes256-gcm@openssh.com`  | `#1f4d7a`       |                    |
| Échange clés | `curve25519-sha256`       | `#1f4d7a`       |                    |
| Clé hôte     | `host-key ✓ connue`       | vert ✓/jaune ⚠/rouge ✕ selon known_hosts | |

```rust
fn badge(ui: &mut egui::Ui, text: &str, fill: egui::Color32) -> egui::Response {
    egui::Frame::none().fill(fill).rounding(2.0)
        .inner_margin(egui::Margin::symmetric(5.0, 1.0))
        .show(ui, |ui| {
            ui.add(Label::new(RichText::new(text).size(10.0)
                .color(Color32::from_rgb(0xdd, 0xdd, 0xdd)).weak()));
        }).response
}
```

## 4. Modes visuels PuTTY à émuler

Mapping `[bin 0.83 / doc PuTTY]` : trois modes de cloche —
`SYSTEM` (bip système), `NONE` (rien), `VISBEL` (flash visuel). Clé de registre `BellType`.

```rust
pub enum BellMode { System = 0, None = 1, Visual = 2 } // stocké en int (BellType)
pub const VISBEL_MS: u64 = 100;   // [doc PuTTY] flash inverse 100 ms sur BEL
```

```rust
// 1) VISBEL — flash inverse : dans le rendu de la grille, si bell_flash_until > Instant::now(),
//    on échange fg/bg GLOBALEMENT (vrai inverse, pas un voile alpha) — fidélité PuTTY.
pub struct BellState { pub flash_until: Option<std::time::Instant> }
fn render_grid(ctx: &egui::Context, st: &mut BellState, term_fg: Color32, term_bg: Color32) {
    let invert = st.flash_until.map_or(false, |u| std::time::Instant::now() < u);
    let (fg, bg) = if invert { (term_bg, term_fg) } else { (term_fg, term_bg) };
    // ... dessiner la grille avec (fg, bg) ...
    if invert { ctx.request_repaint_after(std::time::Duration::from_millis(10)); }
}
fn on_bel(ctx: &egui::Context, mode: BellMode, st: &mut BellState) {
    match mode {
        BellMode::Visual => { st.flash_until = Some(Instant::now() + Duration::from_millis(VISBEL_MS)); },
        BellMode::System => unsafe { winapi::um::winuser::MessageBeep(winapi::um::winuser::MB_ICONASTERISK); }, // bips système
        BellMode::None   => {},
    }
}
```

```rust
// 2) Clignotement du curseur — timer explicite, période [DESIGN] 530 ms (≈ caret Windows)
pub struct CursorBlink { pub phase: bool, pub next_flip: Instant }
fn tick_cursor(ctx: &egui::Context, cb: &mut CursorBlink) -> bool {
    let now = Instant::now();
    if now >= cb.next_flip { cb.phase = !cb.phase; cb.next_flip = now + Duration::from_millis(530); }
    ctx.request_repaint_after(cb.next_flip - now.min(cb.next_flip));
    cb.phase
}
```

> `Belloff/SYSTEM/VISBEL` (énoncé REA) ↔ `BellMode{System,None,Visual}` : la clé
> `BellType` reste l'interface registre (int) ; l'intitulé des 3 modes reste fidèle.

## 5. Thème clair « Legacy PuTTY » + table complète

Défauts `[REA — binaire 0.83 : thème clair par défaut]` : fond blanc, police noire,
**curseur en inversion** : au rendu, le curseur plein (Colour21) recouvre la cellule et le
glyphe est redessiné avec la couleur inverse (Cursor Text), comme dans PuTTY.

| Idx | Registre  | Legacy (clair)  | Professional (sombre) |
|----:|-----------|-----------------|------------------------|
|  0  | Colour0   | `#000000`      | `#000000` |
|  1  | Colour1   | `#800000`      | `#800000` |
|  2  | Colour2   | `#008000`      | `#008000` |
|  3  | Colour3   | `#808000`      | `#808000` |
|  4  | Colour4   | `#000080`      | `#000080` |
|  5  | Colour5   | `#800080`      | `#800080` |
|  6  | Colour6   | `#008080`      | `#008080` |
|  7  | Colour7   | `#c0c0c0`      | `#c0c0c0` |
|  8  | Colour8   | `#555555`      | `#555555` |
|  9  | Colour9   | `#ff5555`      | `#ff5555` |
| 10  | Colour10  | `#55ff55`      | `#55ff55` |
| 11  | Colour11  | `#ffff55`      | `#ffff55` |
| 12  | Colour12  | `#5555ff`      | `#5555ff` |
| 13  | Colour13  | `#ff55ff`      | `#ff55ff` |
| 14  | Colour14  | `#55ffff`      | `#55ffff` |
| 15  | Colour15  | `#ffffff`      | `#ffffff` |
| 16  | Colour16  | `#000000`      | `#d4d4d4` (fg)     |
| 17  | Colour17  | `#000000`      | `#ffffff` (bold fg)|
| 18  | Colour18  | `#ffffff`      | `#1e1e1e` (bg)     |
| 19  | Colour19  | `#ffffff`      | `#1e1e1e` (bold bg)|
| 20  | Colour20  | `#ffffff`      | `#1e1e1e` (cursor text) |
| 21  | Colour21  | `#000000`      | `#d4d4d4` (cursor colour) |

Couleur de **sélection du terminal** : gris 50 % `#808080` (128,128,128) `[doc PuTTY]`,
identique dans les deux thèmes.

```rust
pub struct Theme {
    pub name: String,                       // "dark" | "light"
    pub colours: [Color32; 22],             // index 0..21
    pub sel_bg: Color32,                    // #808080
}
```

> PuTTY « historique » (défauts amont : 187,187,187 / 0,0,0 etc.) est **hors portée** ici :
> on suit l'observation REA (thème clair) et on laisse les 22 valeurs entièrement configurables.

## 6. Configuration (clés registre, style PuTTY)

Arbre `[bin 0.83]` : `HKCU\Software\SimonTatham\PuTTYSessions\<Profil>` — putty-rs lit ce
même arbre et écrit en miroir (fidélité binaire), clés par session :

| Clé         | Type | Valeurs / format                                    | Portée |
|-------------|------|-----------------------------------------------------|--------|
| `Colour0`..`Colour21` | `REG_SZ` | `"r,g,b"` (décimal, ex. `"30,30,30"`)   | [REA] |
| `BellType`  | int  | 0=SYSTEM, 1=NONE, 2=VISBEL                          | [REA] |
| `FontVTMode`| int  | 1 = police VT100/rendu terminal natif (lisible par 0.83) | [REA] |
| `AppTheme`  | `REG_SZ` | `"dark"` \| `"light"`                           | [DESIGN putty-rs] |
| `LineSpacingPct` | int | 0..25 ( % de hauteur de cellule ajoutée )        | [DESIGN putty-rs] |

```rust
fn load_theme(reg: &winreg::RegKey, s: &SessionSettings) -> Theme { // crate winreg (à valider)
    let mut t = match s.app_theme.as_str() {
        "light" => Theme::legacy(), _ => Theme::professional(), // défauts §5
    };
    for (i, c) in t.colours.iter_mut().enumerate() {
        if let Some(raw) = reg.get_value::<String, _>(&format!("Colour{i}")).ok() {
            let p: Vec<u8> = raw.split(',').filter_map(|x| x.trim().parse().ok()).take(3).collect();
            if p.len() == 3 { *c = Color32::from_rgb(p[0], p[1], p[2]); }
        }
    }
    t
} // BellType / FontVTMode : reg.get_value::<u32,_>("BellType").unwrap_or(0)
```

## 7. Micro-cosmétiques « pro » `[DESIGN]` (jusqu'à mention contraire)

```rust
// Police : Consolas 11 pt (≈ 14.66 px @96 dpi), fallback monospace système
let mut fonts = egui::FontDefinitions::default();
if let Ok(data) = std::fs::read(r"C:\Windows\Fonts\consola.ttf") {
    // (+ consolb.ttf pour le gras) ; repli : Cascadia Mono
    fonts.font_data.insert("Consolas".into(), data.into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "Consolas".into());
}
ctx.set_fonts(fonts);
```

- **Antialiasing** : gardé par défaut (feathering epaint activé, MSAA quelconque windows) —
  plus lisible en petit corps qu'un rendu « nearest ».
- **Line spacing configurable** : le texte terminal est rendu par cellules, pas par Label :
  `cell_h = (ascent + descent) * (1.0 + LineSpacingPct/100.0).round();`.
- **Padding** : grille de texte inset de **8 px** dans son panneau (tous côtés).
- **Barre d'état** (bas, 22 px) : `profil | @host:port | badges §3.5 | latence optionnelle`
  (`LE 12 ms` gris, cachée si non mesurée) ; séparateurs `·` en `hint_text_color`.
  - Statut texte secondaire : fond `panel_fill` `#171717`, texte `#9ca0a6`.

```rust
egui::TopBottomPanel::new(TopBottomSide::Bottom, "status").exact_height(22.0)
  .show(ctx, |ui| ui.horizontal(|ui| { /* profil, @host:port, badges, latence */ }));
```

- **Onglets multi-sessions** : **BONUS — hors-portée PuTTY** (fournit une UX « plus pro »
  mais introduit une sémantique de multiplexer absent de PuTTY 0.83 ; à livrer en option
  derrière un flag `MultiSessions=true`, jamais par défaut).
- **Redimensionnement par grille** (pas de scaling bitmap) : la taille de fenêtre impose
  `cols = floor((inner_w - 16 - scrollbar) / cell_w)` et `rows = floor((inner_h - 22 - 28 - 16)/cell_h)` ;
  la rasterisation reste en pixels entiers : jamais `pixels_per_point` ≠ entier pour le
  le texte terminal (le chrome UI peut, lui, suivre le scaling OS).

```rust
use egui::vec2;
fn layout_grid(inner: egui::Vec2, cell: egui::Vec2) -> (usize, usize) {
    let pad = 8.0; let sb = 10.0; let chrome = 28.0 + 22.0;
    let w = ((inner.x - pad * 2.0 - sb) / cell.x).max(1.0) as usize;
    let h = ((inner.y - pad * 2.0 - chrome) / cell.y).max(1.0) as usize;
    (w, h)
}
```

## Annexe — traçabilité des choix

- `[REA / doc PuTTY]` (fidélité imposée) : palette ANSI 16 xterm/VGA + stockage `Colour0..Colour21`
  (§1/§6) ; index 16..21 = fg, bold fg, bg, bold bg, cursor text, cursor colour (§2) ; modes
  SYSTEM/NONE/VISBEL, flash inverse 100 ms, clés `BellType`/`FontVTMode` (§4/§6) ; `Xterm256Colour`
  cube 6³ + rampes gris (§1) ; thème clair Legacy fond blanc/police noire/curseur inversion (§5) ;
  sélection terminal gris 50 % (§5).
- `[DESIGN]` (choix libres, justifiés) : fond `#1e1e1e`, fg `#d4d4d4`, chrome `#171717`/`#0f0f0f`,
  sélection UI `#264f78`, hint gris, badges colorés, barre de titre custom (§3) ; clés putty-rs
  `AppTheme`/`LineSpacingPct` (§6) ; Consolas 11 pt, padding 8 px, blink 530 ms, barre d'état,
  grille entière (§7) ; BONUS onglets multi-sessions (off par défaut).
