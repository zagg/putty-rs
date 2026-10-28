//! putty-rs — clone de PuTTY 0.83 en Rust pur.
//!
//! Enchaînement du démarrage :
//!  1. parsing de la ligne de commande (`cmdline`) ;
//!  2. chargement de la session depuis le registre (`config`, façon PuTTY :
//!     `HKCU\Software\SimonTatham\PuTTY\Sessions` + `Default Settings`) ;
//!  3. fusion ligne de commande / session (priorité à la ligne de commande) ;
//!  4. fabrication du backend (`backend::build`) puis connexion sur le thread
//!     principal (erreurs immédiates visibles avant la fenêtre) ;
//!  5. ouverture de la fenêtre eframe (`app::PuTTYApp`).
//!
//! Déviation assumée : PuTTY sans cible ouvre la boîte de configuration
//! graphique ; le squelette affiche une erreur + l'aide [SUPPOSE/limite].

use std::process::ExitCode;

mod app;
mod backend;
mod cmdline;
mod config;
mod mcp;
mod session;
mod terminal;
mod theme;

fn main() -> ExitCode {
    // Un panic dans un thread auxiliaire (pompe, backend) ne doit pas passer
    // inaperçu : on le consigne avant que le thread ne meure.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC (thread) : {info}");
        eprintln!("{msg}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("putty_rs_panic.log"))
        {
            use std::io::Write;
            let _ = writeln!(f, "{msg}");
        }
    }));

    // Logs via env_logger : variable d'environnement PUTTY_RS_LOG (ou RUST_LOG)
    // ex. `set PUTTY_RS_LOG=putty_rs=debug` avant de lancer.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,putty_rs=info"),
    )
    .init();

    // --- 1) Ligne de commande ------------------------------------------------
    let invocation = match cmdline::parse(std::env::args().skip(1)) {
        Ok(Some(inv)) => inv,
        Ok(None) => {
            // --help : on imprime l'aide et on sort comme PuTTY.
            print!("{}", cmdline::usage());
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("putty-rs: {e}");
            eprintln!();
            eprint!("{}", cmdline::usage());
            return ExitCode::from(2);
        }
    };

    // --- 2) Session depuis le registre ----------------------------------------
    let session = invocation
        .load_session
        .as_deref()
        .or(invocation.saved_session.as_deref());
    let mut cfg = if invocation.no_session {
        config::Config::default()
    } else {
        config::Config::load(session)
    };

    // --- 3) Fusion ligne de commande --------------------------------------------
    cfg.apply_invocation(&invocation);

    // --- 4) Backend : vérification cible + connexion -----------------------------
    let (proto, port) = cfg.resolved();
    if proto != cmdline::Protocol::Serial && cfg.host.is_empty() {
        // PuTTY ouvrirait la boîte de configuration ; ici : message + aide.
        eprintln!(
            "putty-rs: You need to specify a destination address.\n\
             (en la forme \"host.name:port\" — comme putty.exe 0.83)\n"
        );
        eprint!("{}", cmdline::usage());
        return ExitCode::from(2);
    }

    let backend = match backend::build(&cfg, invocation.password.clone()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("putty-rs: {e}");
            return ExitCode::from(1);
        }
    };
    log::info!(
        "démarrage : {proto} → {}:{port} (session « {} », TERM={})",
        cfg.host,
        cfg.session_name,
        cfg.term_type,
    );
    let (handle, events) = match backend::BackendHandle::start(backend) {
        Ok(pair) => pair,
        Err(e) => {
            // Échec de connexion : message façon « PuTTY Fatal Error » [SIMPLIF :
            // pas de popup, juste stderr + code de sortie].
            eprintln!("putty-rs: {e}");
            return ExitCode::from(1);
        }
    };

    // --- 4b) Serveur MCP embarqué (docs/MCP_PILOT.md) -------------------------
    // Priorité FIXÉ 27/10 : -nomcp > -mcpport > registre McpPort > 3927.
    let mcp_port: u16 = if invocation.nomcp {
        0
    } else if let Some(p) = invocation.mcpport {
        p
    } else {
        config::registry_mcp_port().unwrap_or(mcp::DEFAULT_MCP_PORT)
    };
    let mcp_ui = if mcp_port != 0 {
        let (ui_half, server_half) = mcp::McpBus::new();
        let _mcp_thread = mcp::spawn(server_half, mcp_port);
        Some(ui_half)
    } else {
        None
    };

    // --- 5) Fenêtre eframe ---------------------------------------------------------
    let app = app::PuTTYApp::new(cfg.clone(), handle, events, mcp_ui);
    let viewport = egui::ViewportBuilder::default()
        // Taille initiale : grille + marge + barre de statut [SUPPOSE].
        .with_inner_size([
            (cfg.width as f32 * 9.0) + 16.0,
            (cfg.height as f32 * 18.0) + 48.0,
        ])
        .with_title(cfg.static_title());
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let app_name = format!("{} — putty-rs", cfg.session_name);
    match eframe::run_native(
        &app_name,
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("putty-rs: erreur graphique : {e}");
            ExitCode::from(1)
        }
    }
}
