//! Couche backend : connexions réseau des protocoles PuTTY.
//!
//! Architecture commune (choix assumé, pas de tokio sauf pour SSH) :
//!  * un **thread OS par connexion** ;
//!  * deux `std::sync::mpsc` :
//!      - `BackendCommand` (UI → backend) : `Write`, `Resize`, `Shutdown` ;
//!      - `BackendEvent`  (backend → UI) : `Received`, `Closed`, `Error` ;
//!  * le trait [`Backend`] est implémenté par `raw`, `telnet` et (feature
//!    `ssh`) `ssh` ; le rlogin est traité comme du raw [SUPPOSE — le protocole
//!    rlogin proprement dit n'est pas implémenté dans le squelette].
//!
//! `connect()` est appelé sur le thread principal : les erreurs de connexion
//! immédiates remontent avant l'ouverture de la fenêtre.

pub mod raw;
pub mod telnet;
#[cfg(feature = "ssh")]
pub mod ssh;

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::thread::JoinHandle;

use crate::cmdline::Protocol;
use crate::config::Config;

/// Événements backend → UI.
#[derive(Clone, Debug)]
pub enum BackendEvent {
    /// Données brutes à injecter dans le terminal.
    Received(Vec<u8>),
    /// Flux terminé proprement (EOF serveur, shell fermé…).
    Closed,
    /// Erreur fatale du backend (message lisible pour la barre de statut).
    Error(String),
}

/// Commandes UI → backend.
#[derive(Clone, Debug)]
pub enum BackendCommand {
    /// Octets clavier (et answerback) à envoyer au serveur.
    Write(Vec<u8>),
    /// La grille a changé de taille (NAWS en telnet, window-change en SSH).
    Resize { cols: u16, rows: u16 },
    /// Demande d'arrêt propre.
    Shutdown,
}

/// Le contract commun des backends, calqué sur l'énoncé :
/// `connect`, `write` (via `BackendCommand::Write` dans `run`), événements de
/// lecture (via `BackendEvent`), `shutdown`.
pub trait Backend: Send {
    /// Petit nom pour les logs / le thread (« raw », « telnet », « ssh »).
    fn name(&self) -> &'static str;

    /// Établit la connexion (bloquant, thread principal). Peut différer le
    /// travail réel pour `run` (cas de SSH async).
    fn connect(&mut self) -> Result<(), String>;

    /// Boucle principale du thread backend : lit les commandes et émet les
    /// événements. Doit retourner quand la connexion se termine.
    fn run(
        &mut self,
        rx: &Receiver<BackendCommand>,
        tx: &Sender<BackendEvent>,
    ) -> Result<(), String>;

    /// Nettoyage optionnel après la boucle.
    fn shutdown(&mut self) {}
}

/// Poignée tenue par l'UI pour piloter un backend déjà lancé en thread.
pub struct BackendHandle {
    cmd_tx: Sender<BackendCommand>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl BackendHandle {
    /// Connecte le backend (thread appelant) puis lance son thread de service.
    pub fn start(mut backend: Box<dyn Backend>) -> Result<(BackendHandle, Receiver<BackendEvent>), String> {
        backend.connect()?;
        let (ev_tx, ev_rx) = channel::<BackendEvent>();
        let (cmd_tx, cmd_rx) = channel::<BackendCommand>();
        let name = format!("putty-rs-{}", backend.name());
        let join = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                if let Err(e) = backend.run(&cmd_rx, &ev_tx) {
                    let _ = ev_tx.send(BackendEvent::Error(e));
                }
                // Toujours signaler la fin pour débloquer l'UI.
                let _ = ev_tx.send(BackendEvent::Closed);
            })
            .map_err(|e| format!("impossible de créer le thread backend : {e}"))?;
        Ok((BackendHandle { cmd_tx, join: Mutex::new(Some(join)) }, ev_rx))
    }

    /// Envoie une commande (non bloquant).
    pub fn send(&self, cmd: BackendCommand) {
        if self.cmd_tx.send(cmd).is_err() {
            log::debug!("backend déjà terminé, commande ignorée");
        }
    }

    /// Demande l'arrêt ; ne joint pas le thread (fait par `join_all`).
    pub fn shutdown(&self) {
        self.send(BackendCommand::Shutdown);
    }

    /// Attend la fin du thread (appelé à la fermeture de la fenêtre).
    pub fn join(&self) {
        if let Some(h) = self.join.lock().unwrap().take() {
            let _ = h.join();
        }
    }
}

/// Fabrique le backend correspondant à la configuration résolue.
/// `password` = valeur `-pw` de la ligne de commande (jamais persistée).
pub fn build(cfg: &Config, password: Option<String>) -> Result<Box<dyn Backend>, String> {
    let (proto, port) = cfg.resolved();
    match proto {
        Protocol::Raw => Ok(Box::new(raw::RawBackend::new(cfg, port))),
        Protocol::Telnet => Ok(Box::new(telnet::TelnetBackend::new(cfg, port))),
        Protocol::Rlogin => {
            // [SUPPOSE] rlogin = raw pour le squelette (le handshake rlogin
            // « \0user\0user\0term\0 » n'est pas implémenté).
            log::warn!("rlogin non implémenté : traité comme raw");
            Ok(Box::new(raw::RawBackend::new(cfg, port)))
        }
        Protocol::Serial => Ok(Box::new(raw::SerialBackend::new(cfg)?)),
        Protocol::Ssh => {
            #[cfg(feature = "ssh")]
            {
                Ok(Box::new(ssh::SshBackend::new(cfg).with_password(password)))
            }
            #[cfg(not(feature = "ssh"))]
            {
                let _ = password;
                Err("protocole SSH indisponible : binaire compilé sans la feature `ssh`".into())
            }
        }
    }
}
