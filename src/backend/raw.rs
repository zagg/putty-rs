//! Backend « Raw » : flux TCP nu, et son cousin « Serial » (ligne série).
//!
//! Raw PuTTY = connexion TCP sans négociation [DOC]. Le redimensionnement
//! n'a pas de sens côté raw (pas de NAWS) : la commande `Resize` est ignorée
//! [SUPPOSE : PuTTY raw ne négocie pas la taille, comme ici].
//!
//! Socket créé via `std::net` ; `socket2` est réservé pour les réglages
//! fins (family, keepalive) quand le proxy sera câblé — voir ANALYSIS_NOTES.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use super::{Backend, BackendCommand, BackendEvent};
use crate::config::Config;

/// Latence max de réaction aux commandes (shutdown) : le read expire toutes
/// les 200 ms et on retente try_recv.
const POLL: Duration = Duration::from_millis(200);

// =============================================================================
// Raw TCP
// =============================================================================

pub struct RawBackend {
    host: String,
    port: u16,
    nodelay: bool,
    stream: Option<TcpStream>,
}

impl RawBackend {
    pub fn new(cfg: &Config, port: u16) -> Self {
        RawBackend { host: cfg.host.clone(), port, nodelay: cfg.nodelay, stream: None }
    }
}

impl Backend for RawBackend {
    fn name(&self) -> &'static str { "raw" }

    fn connect(&mut self) -> Result<(), String> {
        let addr = format!("{}:{}", self.host, self.port);
        // socket2 : choix explicite de la famille (IPv4/IPv6) + nodelay,
        // comme le fait PuTTY via ses options réseau.
        let socket_addr = std::net::ToSocketAddrs::to_socket_addrs(&addr)
            .map_err(|e| format!("résolution de {addr} impossible : {e}"))?
            .next()
            .ok_or_else(|| format!("adresse réseau introuvable : {addr}"))?;
        let sock = socket2::SockAddr::from(socket_addr);
        let socket = socket2::Socket::new(
            socket2::Domain::for_address(socket_addr), // 0.5 : family → Domain
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .map_err(|e| format!("création socket raw : {e}"))?;
        socket
            .connect(&sock)
            .map_err(|e| format!("connexion raw à {addr} impossible : {e}"))?;
        let stream: TcpStream = socket.into();
        // PuTTY applique TCP_NODELAY (`TCPNoDelay`) [DOC, défaut à confirmer].
        let _ = stream.set_nodelay(self.nodelay);
        self.stream = Some(stream);
        Ok(())
    }

    fn run(&mut self, rx: &Receiver<BackendCommand>, tx: &Sender<BackendEvent>) -> Result<(), String> {
        let stream = self.stream.as_mut().ok_or("raw : non connecté")?;
        stream.set_read_timeout(Some(POLL)).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 8192];
        loop {
            // 1) commandes en attente
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    BackendCommand::Write(b) => {
                        stream.write_all(&b).map_err(|e| format!("écriture raw : {e}"))?;
                        stream.flush().ok();
                    }
                    BackendCommand::Resize { .. } => { /* pas de NAWS en raw [SUPPOSE] */ }
                    BackendCommand::Shutdown => return Ok(()),
                }
            }
            // 2) lecture non bloquante « par timeout »
            match stream.read(&mut buf) {
                Ok(0) => return Ok(()), // EOF serveur
                Ok(n) => {
                    if tx.send(BackendEvent::Received(buf[..n].to_vec())).is_err() {
                        return Ok(()); // UI partie
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => return Err(format!("lecture raw : {e}")),
            }
        }
    }

    fn shutdown(&mut self) {
        if let Some(s) = &self.stream {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

// =============================================================================
// Serial (ligne série) — approximation squelette
// =============================================================================
//
// Sous Windows, ouvrir `\\.\COM1` avec `std::fs` donne un handle dont les
// lectures sont bloquantes SANS timeout (les timeouts COM nécessitent
// SetCommTimeouts, API Win32 non liée ici). Conséquence [SUPPOSE] :
// `Shutdown` peut attendre la prochaine activité sur le port pour être
// traité. Documenté comme limite connue.

pub struct SerialBackend {
    path: PathBuf,
    file: Option<File>,
}

impl SerialBackend {
    pub fn new(cfg: &Config) -> Result<Self, String> {
        let line = cfg.serial_line.trim();
        if line.is_empty() {
            return Err("aucune ligne série configurée (SerialLine / -sercfg)".into());
        }
        // Format Win32 des ports > COM9 et ports « présents » : \\.\COMx.
        let path = PathBuf::from(format!(r"\\.\{line}"));
        Ok(SerialBackend { path, file: None })
    }
}

impl Backend for SerialBackend {
    fn name(&self) -> &'static str { "serial" }

    fn connect(&mut self) -> Result<(), String> {
        // Débit/parité (SerialSpeed OBSERVÉ, 9600/8/N/1 [DOC]) : la configuration
        // DCB nécessite l'API Win32 (BuildCommDCB+SetCommState) non liée ici :
        // le port garde sa configuration système [SUPPOSE / limite connue].
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .map_err(|e| format!("ouverture série {} impossible : {e}", self.path.display()))?;
        self.file = Some(f);
        Ok(())
    }

    fn run(&mut self, rx: &Receiver<BackendCommand>, tx: &Sender<BackendEvent>) -> Result<(), String> {
        let file = self.file.as_mut().ok_or("série : non ouverte")?;
        let mut buf = [0u8; 4096];
        loop {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    BackendCommand::Write(b) => {
                        file.write_all(&b).map_err(|e| format!("écriture série : {e}"))?;
                        file.flush().ok();
                    }
                    BackendCommand::Resize { .. } => {}
                    BackendCommand::Shutdown => return Ok(()),
                }
            }
            match file.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    if tx.send(BackendEvent::Received(buf[..n].to_vec())).is_err() {
                        return Ok(());
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => return Err(format!("lecture série : {e}")),
            }
        }
    }
}
