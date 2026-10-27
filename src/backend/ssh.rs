//! Backend SSH via `russh` (async, nécessite tokio — confinés ici derrière
//! le feature `ssh`).
//!
//! API validée sur docs.rs (russh 0.45.0) :
//!  * `client::Handler::check_server_key(&PublicKey) -> Result<bool>` (le
//!    défaut REJETTE toute clé : on doit l'implémenter) ;
//!  * `client::Handler::auth_banner(&str, &mut Session)` ;
//!  * `Handle::channel_open_session() -> Channel` ;
//!  * `Channel::request_pty(want_reply, term, col_width, row_height,
//!    pix_width, pix_height, terminal_modes)` ;
//!  * `Channel::request_shell(want_reply)` ;
//!  * `Channel::window_change(col_width, row_height, pix_width, pix_height)` ;
//!  * `Channel::data<R: AsyncRead + Unpin>(R)` ;
//!  * `Channel::wait() -> Option<ChannelMsg>` : flux entrant (Data,
//!    ExtendedData, Eof, Close, ExitStatus…).
//!
//! Simplifications assumées (à confronter à PuTTY, voir ANALYSIS_NOTES.md) :
//!  * **Vérification de clé d'hôte interactive SIMPLIFIÉE** : toute clé est
//!    acceptée automatiquement et journalisée, sans popup « The server's host
//!    key is not cached » et sans écriture dans
//!    `HKCU\Software\SimonTatham\PuTTY\SshHostKeys` (clé OBSERVÉE dans
//!    putty.exe, VA 0x14010bad0) `[SIMPLIF]`.
//!  * MOTD : la bannière d'authentification (SSH_MSG_USERAUTH_BANNER) est
//!    injectée dans le terminal, comme le fait PuTTY [DOC].
//!  * Le format .ppk de PuTTY n'est pas lu par russh : convertir la clé avec
//!    puttygen → « Export OpenSSH key » `[LIMITATION]`.
//!  * Agent Pageant non tenté `[SUPPOSE]`.
//!  * Tunnels -L/-R/-D et X11 acceptés au parsing mais non câblés
//!    `[LIMITATION]`.

use std::io::Cursor;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use async_trait::async_trait;

use super::{Backend, BackendCommand, BackendEvent};
use crate::config::Config;

pub struct SshBackend {
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    key_file: Option<std::path::PathBuf>,
    term_type: String,
    compression: bool,
    no_shell: bool,
    accept_hostkey: bool,
    reject_hostkey: bool,
    cols: u16,
    rows: u16,
}

impl SshBackend {
    pub fn new(cfg: &Config) -> Self {
        // [SUPPOSE] PuTTY : username vide = nom de l'utilisateur Windows.
        let username = if cfg.username.is_empty() {
            std::env::var("USERNAME").unwrap_or_default()
        } else {
            cfg.username.clone()
        };
        SshBackend {
            host: cfg.host.clone(),
            port: cfg.port,
            username,
            password: None, // -pw porté par Invocation (non persisté en registre)
            key_file: cfg.public_key_file.clone(),
            term_type: cfg.term_type.clone(),
            compression: cfg.compression,
            no_shell: cfg.no_shell,
            accept_hostkey: cfg.accept_hostkey,
            reject_hostkey: cfg.reject_hostkey,
            cols: cfg.width as u16,
            rows: cfg.height as u16,
        }
    }

    /// Injecte le mot de passe `-pw` (Config ne le persiste pas, comme PuTTY).
    pub fn with_password(mut self, pw: Option<String>) -> Self {
        self.password = pw;
        self
    }
}

/// Handler russh : bannière + politique de clé d'hôte.
struct Handler {
    tx: Sender<BackendEvent>,
    accept_hostkey: bool,
    reject_hostkey: bool,
}

#[async_trait]
impl russh::client::Handler for Handler {
    type Error = russh::Error;

    /// Vérification de clé d'hôte `[SIMPLIF]` : acceptation automatique
    /// (sauf -reject-hostkey qui refuse, comme l'option PuTTY).
    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Empreinte SHA-256 base64 ; russh 0.45 renvoie le b64 sans préfixe,
        // on formate « SHA256:... » comme les outils PuTTY [REA §2 dialogues].
        let fp = server_public_key.fingerprint();
        let algo = server_public_key.name();
        if self.reject_hostkey {
            log::warn!("clé d'hôte {algo} SHA256:{fp} REFUSÉE (-reject-hostkey)");
            return Ok(false);
        }
        log::info!(
            "clé d'hôte {algo} SHA256:{fp} acceptée automatiquement (accept_hostkey={})",
            self.accept_hostkey
        );
        // NOTE REA : PuTTY affiche une popup avec l'empreinte (observed :
        // « WARNING - POTENTIAL SECURITY BREACH! », « press "Accept". », etc.)
        // et persiste la clé dans HKCU\...\SshHostKeys.
        Ok(true)
    }

    /// Bannière d'authentification (MOTD pré-session) : affichée dans le
    /// terminal, comme PuTTY [DOC].
    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        let mut bytes = banner.as_bytes().to_vec();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let _ = self.tx.send(BackendEvent::Received(bytes));
        Ok(())
    }
}

impl Backend for SshBackend {
    fn name(&self) -> &'static str { "ssh" }

    fn connect(&mut self) -> Result<(), String> {
        // La connexion réelle a lieu dans `run` (async) ; validation rapide.
        if self.host.is_empty() {
            // Message OBSERVÉ dans putty.exe (« You need to specify a
            // destination address. », offset ~1164400).
            return Err("You need to specify a destination address.".into());
        }
        Ok(())
    }

    fn run(&mut self, rx: &Receiver<BackendCommand>, tx: &Sender<BackendEvent>) -> Result<(), String> {
        // russh exige tokio : runtime multi-thread dédié, créé ICI et
        // détruit à la fin de la connexion (le reste du projet reste
        // threads-OS + mpsc).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("runtime tokio : {e}"))?;
        rt.block_on(serve(self, rx, tx))
    }
}

/// Boucle SSH asynchrone complète.
async fn serve(
    cfg: &SshBackend,
    rx: &Receiver<BackendCommand>,
    tx: &Sender<BackendEvent>,
) -> Result<(), String> {
    use russh::ChannelMsg;

    // Compression : russh 0.45 la configure dans `preferred` (négociation),
    // pas dans un champ `compression` de Config. zlib@openssh.com d'abord,
    // puis zlib, puis « none » [DOC RFC].
    let mut preferred = russh::Preferred::default();
    preferred.compression = std::borrow::Cow::from(if cfg.compression {
        vec![
            russh::compression::ZLIB_LEGACY,
            russh::compression::ZLIB,
            russh::compression::NONE,
        ]
    } else {
        vec![russh::compression::NONE]
    });
    let config = Arc::new(russh::client::Config {
        preferred,
        ..Default::default()
    });

    let handler = Handler { tx: tx.clone(), accept_hostkey: cfg.accept_hostkey, reject_hostkey: cfg.reject_hostkey };
    let mut handle = russh::client::connect(config, (cfg.host.as_str(), cfg.port), handler)
        .await
        .map_err(|e| format!("connexion SSH à {}:{} : {e}", cfg.host, cfg.port))?;

    // --- Authentification -----------------------------------------------------
    let mut authed = false;
    if let Some(pw) = &cfg.password {
        authed = handle
            .authenticate_password(cfg.username.as_str(), pw.as_str())
            .await
            .is_ok();
    }
    if !authed {
        if let Some(kf) = &cfg.key_file {
            let key = load_key(kf).map_err(|e| format!("clé privée {} : {e}", kf.display()))?;
            // russh 0.45 : authenticate_publickey attend Arc<key::KeyPair>.
            authed = handle
                .authenticate_publickey(cfg.username.as_str(), Arc::new(key))
                .await
                .is_ok();
        }
    }
    // [SUPPOSE] agent Pageant non tenté (russh le ferait via russh-agent).
    if !authed {
        // Écho du comportement PuTTY « Sent username "%s" » puis refus (OBSERVÉ).
        return Err(format!("authentification refusée pour \"{}\"", cfg.username));
    }

    // --- Canal, PTY, shell -------------------------------------------------------
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("ouverture canal : {e}"))?;

    channel
        .request_pty(
            true,
            cfg.term_type.as_str(),
            cfg.cols as u32,
            cfg.rows as u32,
            0,
            0,
            &[],
        )
        .await
        .map_err(|e| format!("request_pty : {e}"))?;

    if !cfg.no_shell {
        // Sans commande, PuTTY lance le shell de login [DOC].
        channel
            .request_shell(true)
            .await
            .map_err(|e| format!("request_shell : {e}"))?;
    }

    // --- Boucle : messages serveur + commandes UI ----------------------------------
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(50));
    loop {
        tokio::select! {
            biased;

            // Commandes de l'UI (polling non bloquant toutes les 50 ms).
            _ = ticker.tick() => {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        BackendCommand::Write(bytes) => {
                            // Channel::data attend un AsyncRead : Cursor<&[u8]>
                            // l'implémente (tokio).
                            if channel.data(Cursor::new(bytes)).await.is_err() {
                                return Ok(());
                            }
                        }
                        BackendCommand::Resize { cols, rows } => {
                            let _ = channel
                                .window_change(cols as u32, rows as u32, 0, 0)
                                .await;
                        }
                        BackendCommand::Shutdown => {
                            let _ = channel.close().await;
                            let _ = handle
                                .disconnect(russh::Disconnect::ByApplication, "", "en")
                                .await;
                            return Ok(());
                        }
                    }
                }
            }

            // Messages entrants du canal.
            msg = channel.wait() => {
                match msg {
                    None => return Ok(()),                       // canal fermé
                    Some(ChannelMsg::Data { data }) => {
                        if tx.send(BackendEvent::Received(data.to_vec())).is_err() {
                            return Ok(());                        // UI partie
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        // stderr mélangé au terminal, comme PuTTY [SUPPOSE].
                        let _ = tx.send(BackendEvent::Received(data.to_vec()));
                    }
                    Some(ChannelMsg::Eof { .. }) | Some(ChannelMsg::Close { .. }) => {
                        return Ok(());
                    }
                    Some(ChannelMsg::ExitStatus { .. }) => {
                        return Ok(());
                    }
                    _ => { /* Success/Failure/WindowAdjust… : rien à faire */ }
                }
            }
        }
    }
}

/// Charge une clé privée OpenSSH depuis le disque.
/// NOTE : le format `.ppk` de PuTTY/puttygen N'EST PAS supporté par russh —
/// refus explicite et clair `[LIMITATION v0.1]` ; convertir la clé avec
/// puttygen → « Export OpenSSH key » (ou lire la clé OpenSSH directement).
fn load_key(path: &std::path::Path) -> Result<russh::keys::key::KeyPair, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.starts_with(b"PuTTY-User-Key-File") {
        return Err(
            "format PPK non supporté en v0.1 — utiliser une clé OpenSSH \
             (puttygen : « Export OpenSSH key »)"
                .into(),
        );
    }
    // OpenSSH nouveau format + PEM ancien format gérés par russh-keys ;
    // None = sans passphrase (les clés chiffrées échoueront clairement).
    russh::keys::load_secret_key(path, None).map_err(|e| e.to_string())
}
