//! Backend Telnet avec négociateur IAC minimal.
//!
//! Numéros d'options conformes aux RFC :
//!  * RFC 854 (Telnet) : ECHO=1, SGA=3, STATUS=5, TIMING-MARK=6, RCP=7,
//!    RS=8, US=9, NAWS=31 (RFC 1073), TSPEED=32 (RFC 1079),
//!    X-Display-Location=35 (RFC 1091).
//!  * Commandes : IAC=255, DONT=254, DO=253, WILL=252, WONT=251, SB=250,
//!    SE=240, GA=249, EL=246, GM=245, SB/SE, NOP=241, DM=242, BRK=243, IP=244.
//!
//! Politique (squelette) `[SUPPOSE] — à confronter aux traces REA du
//! négotiateur réel de PuTTY (`telnet.c`, cf. ANALYSIS_NOTES.md) :
//!  * le client reste « passif » : il répond WONT à tout DO (sauf STATUS→SB
//!    réponse) et n'ignore que les WILL ECHO/SGA sans répondre (le serveur
//!    écho est accepté tel quel, comme PuTTY qui laisse le serveur échoer) ;
//!  * le client annonce NAWS (taille de fenêtre) au connect et à chaque
//!    resize ;
//!  * `IAC DO NAWS` du serveur → re-envoi immédiat de la taille.
//!  * EOR (35, binaire) est consommé silencieusement.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use super::{Backend, BackendCommand, BackendEvent};
use crate::config::Config;

// --- Commandes Telnet (RFC 854) ---------------------------------------------
const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WILL: u8 = 252;
const WONT: u8 = 251;
const SB: u8 = 250;
const GA: u8 = 249;
const EL: u8 = 246;
const GM: u8 = 245;
const NOP: u8 = 241;
const DM: u8 = 242;
const BRK: u8 = 243;
const IP: u8 = 244;
const SE: u8 = 240;

// --- Options (RFC 854 / 1073 / 1079 / 1091) ----------------------------------
pub mod opt {
    pub const ECHO: u8 = 1;
    pub const SGA: u8 = 3;
    pub const STATUS: u8 = 5;
    pub const TIMING_MARK: u8 = 6;
    #[allow(dead_code)]
    pub const NAWS: u8 = 31; // RFC 1073
    #[allow(dead_code)]
    pub const TSPEED: u8 = 32; // RFC 1079
    #[allow(dead_code)]
    pub const X_DISPLAY: u8 = 35; // RFC 1091
}

const POLL: Duration = Duration::from_millis(200);

/// Automate IAC : extrait les données utiles et génère les réponses.
#[derive(Default)]
struct IacMachine {
    /// État de l'automate.
    state: IacState,
    /// tampon de sous-négociation.
    sub: Vec<u8>,
    /// Options déjà refusées (anti-boucle de négociation) [SUPPOSE].
    refused: Vec<u8>,
    /// Options que NOUS avons annoncées WILL.
    we_will: Vec<u8>,
}

#[derive(Default, PartialEq)]
enum IacState {
    #[default]
    Data,
    /// Octet IAC reçu : le prochain octet est une commande.
    Command,
    /// Attend l'option de DO/DONT/WILL/WONT.
    CommandOpt { cmd: u8 },
    /// `IAC SB` reçu : le prochain octet est l'option de la sous-négociation.
    SubOpt,
    /// En sous-négociation (IAC SB <opt> … IAC SE).
    Sub { opt: u8 },
    /// IAC reçu à l'intérieur d'une sous-négociation.
    SubIac { opt: u8 },
}

impl IacMachine {
    /// Traite des octets du serveur ; retourne les données « écran » et les
    /// octets de réponse à envoyer.
    fn feed(&mut self, bytes: &[u8], cols: u16, rows: u16) -> (Vec<u8>, Vec<u8>) {
        let mut data = Vec::with_capacity(bytes.len());
        let mut reply = Vec::new();
        for &b in bytes {
            match self.state {
                IacState::Data => match b {
                    IAC => self.state = IacState::Command,
                    _ => data.push(b),
                },
                IacState::Command => match b {
                    IAC => { data.push(IAC); self.state = IacState::Data; } // 0xFF 0xFF → 0xFF
                    DO | DONT | WILL | WONT => self.state = IacState::CommandOpt { cmd: b },
                    SB => {
                        self.sub.clear();
                        self.state = IacState::SubOpt;
                    }
                    SE | GA | EL | GM | NOP | DM | BRK | IP => {
                        // commandes seules : ignorées.
                        self.state = IacState::Data;
                    }
                    _ => self.state = IacState::Data,
                },
                IacState::CommandOpt { cmd } => {
                    self.on_command(cmd, b, cols, rows, &mut reply);
                    self.state = IacState::Data;
                }
                IacState::SubOpt => {
                    self.state = IacState::Sub { opt: b };
                }
                IacState::Sub { opt } => match b {
                    IAC => self.state = IacState::SubIac { opt },
                    _ => self.sub.push(b),
                },
                IacState::SubIac { opt } => {
                    if b == SE {
                        // Hoist du tampon : evite deux emprunts mutables de
                        // self.sub (lecepteur + argument) sur la meme ligne.
                        let sub = std::mem::take(&mut self.sub);
                        self.on_subneg(opt, &sub, &mut reply);
                        self.state = IacState::Data;
                    } else {
                        // « IAC IAC » dans un SB : IAC littéral.
                        self.sub.push(IAC);
                        self.sub.push(b);
                        self.state = IacState::Sub { opt };
                    }
                }
            }
        }
        (data, reply)
    }

    fn on_command(&mut self, cmd: u8, o: u8, cols: u16, rows: u16, reply: &mut Vec<u8>) {
        match (cmd, o) {
            // Le serveur nous demande d'activer une option → on refuse (client
            // passif) [SUPPOSE], sauf NAWS où on renvoie la taille.
            (DO, opt::NAWS) => {
                reply.extend_from_slice(&naws(cols, rows));
            }
            (DO, opt::STATUS) => {
                // IAC SB STATUS (aucune option active WILL) IAC SE.
                reply.extend_from_slice(&[IAC, SB, opt::STATUS, IAC, SE]);
            }
            (DO, o) => {
                if !self.refused.contains(&o) {
                    self.refused.push(o);
                    reply.extend_from_slice(&[IAC, WONT, o]);
                }
            }
            // Le serveur active une option chez lui : on accepte en silence
            // (écho serveur, SGA…) [SUPPOSE — PuTTY répondrait peut-être DO ;
            // à vérifier en trace REA].
            (WILL, _) => {}
            // Refus du serveur ou retrait : rien à faire.
            (DONT, _) | (WONT, _) => {}
            _ => {}
        }
    }

    fn on_subneg(&mut self, o: u8, _body: &[u8], reply: &mut Vec<u8>) {
        match o {
            // Le serveur demande notre taille ? (certains serveurs sondent)
            opt::NAWS => {}
            // SEND STATUS : renvoyer nos WILL actifs.
            opt::STATUS => {
                reply.push(IAC);
                reply.push(SB);
                reply.push(opt::STATUS);
                for &w in &self.we_will {
                    reply.push(WILL);
                    reply.push(w);
                }
                reply.push(IAC);
                reply.push(SE);
            }
            _ => {}
        }
    }
}

/// Séquence complète IAC SB NAWS cols/rows IAC SE (RFC 1073 : col hi/lo puis
/// row hi/lo).
fn naws(cols: u16, rows: u16) -> Vec<u8> {
    vec![
        IAC, SB, opt::NAWS,
        (cols >> 8) as u8, (cols & 0xff) as u8,
        (rows >> 8) as u8, (rows & 0xff) as u8,
        IAC, SE,
    ]
}

pub struct TelnetBackend {
    host: String,
    port: u16,
    nodelay: bool,
    stream: Option<TcpStream>,
    cols: u16,
    rows: u16,
    iac: IacMachine,
    /// TERM annoncé (PuTTY envoie « TERM=putty » en telnet [DOC]).
    #[allow(dead_code)]
    term_type: String,
}

impl TelnetBackend {
    pub fn new(cfg: &Config, port: u16) -> Self {
        TelnetBackend {
            host: cfg.host.clone(),
            port,
            nodelay: cfg.nodelay,
            stream: None,
            cols: cfg.width as u16,
            rows: cfg.height as u16,
            iac: IacMachine::default(),
            term_type: cfg.telnet_term_type.clone(),
        }
    }
}

impl Backend for TelnetBackend {
    fn name(&self) -> &'static str { "telnet" }

    fn connect(&mut self) -> Result<(), String> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream = TcpStream::connect(&addr)
            .map_err(|e| format!("connexion telnet à {addr} impossible : {e}"))?;
        let _ = stream.set_nodelay(self.nodelay);
        self.stream = Some(stream);
        // Annonce initiale : NAWS (avec WILL NAWS une seule fois).
        let s = self.stream.as_mut().unwrap();
        s.write_all(&[IAC, WILL, opt::NAWS]).map_err(|e| e.to_string())?;
        s.write_all(&naws(self.cols, self.rows)).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn run(&mut self, rx: &Receiver<BackendCommand>, tx: &Sender<BackendEvent>) -> Result<(), String> {
        let stream = self.stream.as_mut().ok_or("telnet : non connecté")?;
        stream.set_read_timeout(Some(POLL)).map_err(|e| e.to_string())?;
        let mut buf = [0u8; 8192];
        loop {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    BackendCommand::Write(b) => {
                        // Échapement IAC dans les données clavier : 0xFF → 0xFFFF.
                        let mut esc = Vec::with_capacity(b.len() + 4);
                        for &x in &b {
                            if x == IAC {
                                esc.push(IAC);
                            }
                            esc.push(x);
                        }
                        stream.write_all(&esc).map_err(|e| format!("écriture telnet : {e}"))?;
                        stream.flush().ok();
                    }
                    BackendCommand::Resize { cols, rows } => {
                        self.cols = cols;
                        self.rows = rows;
                        stream
                            .write_all(&naws(cols, rows))
                            .map_err(|e| format!("NAWS : {e}"))?;
                        stream.flush().ok();
                    }
                    BackendCommand::Shutdown => return Ok(()),
                }
            }
            match stream.read(&mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    let (data, reply) = self.iac.feed(&buf[..n], self.cols, self.rows);
                    if !reply.is_empty() {
                        stream.write_all(&reply).map_err(|e| format!("réponse IAC : {e}"))?;
                        stream.flush().ok();
                    }
                    if !data.is_empty() {
                        if tx.send(BackendEvent::Received(data)).is_err() {
                            return Ok(());
                        }
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(e) => return Err(format!("lecture telnet : {e}")),
            }
        }
    }

    fn shutdown(&mut self) {
        if let Some(s) = &self.stream {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iac_stripping_and_replies() {
        let mut m = IacMachine::default();
        // Serveur : "AB" + DO ECHO + "C" + WILL ECHO + IAC SB NAWS … SE + "D"
        let mut input = b"AB".to_vec();
        input.extend_from_slice(&[IAC, DO, opt::ECHO]);
        input.push(b'C');
        input.extend_from_slice(&[IAC, WILL, opt::ECHO]);
        input.extend_from_slice(&[IAC, SB, opt::NAWS, 0, 80, 0, 24, IAC, SE]);
        input.push(b'D');
        let (data, reply) = m.feed(&input, 80, 24);
        assert_eq!(data, b"ABCD");
        // DO ECHO ⇒ WONT ECHO (une fois).
        assert_eq!(reply, vec![IAC, WONT, opt::ECHO]);
    }

    #[test]
    fn escaped_0xff_passes_through() {
        let mut m = IacMachine::default();
        let (data, reply) = m.feed(&[0x41, IAC, IAC, 0x42], 80, 24);
        assert_eq!(data, b"A\xFFB");
        assert!(reply.is_empty());
    }

    #[test]
    fn naws_wire_format() {
        assert_eq!(naws(300, 24), vec![IAC, SB, opt::NAWS, 1, 44, 0, 24, IAC, SE]);
    }
}
