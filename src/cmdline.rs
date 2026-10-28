//! Parseur de ligne de commande compatible avec les options observables de
//! PuTTY 0.83 (`putty.exe [options] [user@]host[:port]`).
//!
//! Sources :
//!  * Options observées dans les chaînes de putty.exe 0.83
//!    (sha256 12498d4e…0366) : `-ssh`, `-telnet`, `-serial`, `-load`, `-pw `,
//!    `-agent`, `-sercfg`, message `unknown option "%s"`, etc.
//!    (cf. ANALYSIS_NOTES.md, section « Preuves REA »).
//!  * Documentation PuTTY 0.83 (using.html) pour le reste.
//!
//! Conventions PuTTY reprises ici :
//!  * Les options « simples » ont un seul tiret (`-ssh`, `-l`, `-P`…).
//!  * Les options qui mappent un réglage de session (proxy, sessgroup…) sont
//!    documentées par PuTTY avec deux tirets (`--proxyhost`) ; le binaire
//!    contient ces noms SANS tiret préfixe (`proxyhost`). Nous acceptons les
//!    trois formes (`-x`, `--x`) pour être tolérants.  [REA] à confirmer.
//!  * Une option inconnue provoque l'erreur `unknown option "%s"` (format
//!    OBSERVÉ dans le binaire).

use std::path::PathBuf;

/// Protocole (« connection type » dans la GUI PuTTY).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    Ssh,
    Telnet,
    Rlogin,
    Raw,
    Serial,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Protocol {
    /// Ports par défaut par protocole.
    ///
    /// * ssh=22, telnet=23, rlogin=513 : valeurs classiques, `[DOC PuTTY 0.83]`.
    /// * raw=10001 : documented par PuTTY comme port par défaut du protocole
    ///   Raw `[DOC]` — la valeur brute « 10001 » n'a PAS été retrouvée dans
    ///   les chaînes du binaire (c'est un entier compilé) : `[REA]` à confirmer
    ///   en observant le champ Port de la GUI avec « Raw » sélectionné.
    /// * serial : pas de port (ligne série `COMx`).
    pub fn default_port(self) -> Option<u16> {
        match self {
            Protocol::Ssh => Some(22),
            Protocol::Telnet => Some(23),
            Protocol::Rlogin => Some(513),
            Protocol::Raw => Some(10_001), // [SUPPOSE/DOC] à confirmer [REA]
            Protocol::Serial => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Ssh => "ssh",
            Protocol::Telnet => "telnet",
            Protocol::Rlogin => "rlogin",
            Protocol::Raw => "raw",
            Protocol::Serial => "serial",
        }
    }
}

/// Type de proxy (`-proxy <type>`), comme la GUI PuTTY.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProxyType {
    None,
    Http,
    Socks4,
    Socks5,
    Local,   // « Local (skip proxy settings) »
    Telnet,  // Old-style telnet proxy
}

impl ProxyType {
    fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(ProxyType::None),
            "http" => Ok(ProxyType::Http),
            "socks4" | "socks4a" => Ok(ProxyType::Socks4),
            "socks5" => Ok(ProxyType::Socks5),
            "local" => Ok(ProxyType::Local),
            "telnet" => Ok(ProxyType::Telnet),
            _ => Err(format!("unknown proxy type \"{s}\"")),
        }
    }
}

/// Sens d'un tunnel (`-L`, `-R`, `-D`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TunnelKind {
    Local,
    Remote,
    Dynamic,
}

/// Un tunnel demandé sur la ligne de commande (`-L 8080:host:80` etc.).
#[derive(Clone, Debug)]
pub struct Tunnel {
    pub kind: TunnelKind,
    /// Spécification brute, dans le format PuTTY (`source,port,dest,destport`).
    pub spec: String,
}

/// Résultat du parsing de la ligne de commande.
///
/// `None` partout = « non fourni sur la ligne de commande » ; la fusion avec
/// la session sauvegardée/le registre a lieu dans `Config::apply_invocation`.
#[derive(Clone, Debug, Default)]
pub struct Invocation {
    /// `--help` demandé.
    pub help: bool,

    /// Protocole explicitement forcé (`-ssh`, `-telnet`, …).
    pub protocol: Option<Protocol>,

    // --- Cible -------------------------------------------------------------
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Type de cible série (`-sercfg speed,data,parity,stop[,flow]`).
    pub serial_config: Option<String>,
    /// Ligne série (`COMx`) : fournie en positionnel avec `-serial` [DOC].
    pub serial_line: Option<String>,

    // --- Authentification --------------------------------------------------
    pub username: Option<String>,   // `-l` / `-user`
    pub password: Option<String>,   // `-pw`
    pub key_file: Option<PathBuf>,  // `-i`
    pub try_agent: Option<bool>,    // `-agent` / `-noagent`

    // --- Sessions ----------------------------------------------------------
    pub load_session: Option<String>,  // `-load <session>`
    pub saved_session: Option<String>, // `--savedsession <session>`
    pub sess_group: Option<String>,    // `--sessgroup <groupe>`
    pub no_session: bool,              // `-nosession` : ignorer le registre

    // --- Fenêtre / terminal -------------------------------------------------
    pub window_title: Option<String>, // `-T <titre>` / alias `-title <titre>`
    pub clear_title: bool,            // efface le titre dynamique distant
    pub term_type: Option<String>,    // `-term <type>` [SUPPOSE : PuTTY n'a pas
                                      // d'option -term officielle ; à confirmer]
    pub old_geometry: bool,           // `-old-geometry`

    // --- SSH ----------------------------------------------------------------
    pub x11_forward: Option<bool>,    // `-X` / `-noX11` / `-x`
    pub compression: Option<bool>,    // `-C`
    pub no_shell: bool,               // `-N` (pas de shell, tunnels seuls)
    pub tunnels: Vec<Tunnel>,         // `-L` `-R` `-D`
    pub local_ip: Option<String>,     // `--localip`
    pub accept_hostkey: bool,         // `-accept-hostkey`
    pub reject_hostkey: bool,         // `-reject-hostkey`
    pub nodelay: bool,                // `-nodelay`

    // --- putty-rs : serveur MCP embarqué [DESIGN] (docs/MCP_PILOT.md) ------
    pub mcpport: Option<u16>,         // `-mcpport <n>` (0 = désactivé)
    pub nomcp: bool,                  // `-nomcp` : désactive complètement

    // --- Proxy --------------------------------------------------------------
    pub proxy: Option<ProxyType>,     // `-proxy <type>`
    pub proxy_host: Option<String>,   // `--proxyhost`
    pub proxy_port: Option<u16>,      // `--proxyport`
    pub proxy_user: Option<String>,   // `--proxyusername`
    pub proxy_pass: Option<String>,   // `--proxypassword`

    // --- Divers acceptés mais ignorés dans le squelette ---------------------
    /// Options reconnues mais sans effet (documentées dans README/notes).
    pub ignored: Vec<String>,
}

impl Invocation {
    /// Le `-pw` n'est accepté par PuTTY qu'avec SSH :
    /// « the -pw option can only be used with the SSH protocol. » (OBSERVÉ).
    fn check_consistency(&self) -> Result<(), String> {
        if self.password.is_some()
            && matches!(self.protocol, Some(p) if p != Protocol::Ssh)
        {
            return Err("the -pw option can only be used with the SSH protocol.".into());
        }
        if self.serial_config.is_some()
            && matches!(self.protocol, Some(p) if p != Protocol::Serial)
        {
            // Message OBSERVÉ dans putty.exe (offset ~1109483).
            return Err("the -sercfg option can only be used with the serial protocol.".into());
        }
        Ok(())
    }
}

/// Normalise une option : retire un éventuel préfixe `-` ou `--`.
/// (PuTTY stocke les options de type « réglage » sans tiret dans le binaire.)
fn normalize_flag(arg: &str) -> &str {
    arg.trim_start_matches('-')
}

/// Parse `argv` (sans le nom du programme).
///
/// Retourne `Ok(None)` si l'aide a été demandée (`--help`, `-h`, `-?`).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Option<Invocation>, String> {
    let mut inv = Invocation::default();
    let args: Vec<String> = args.into_iter().collect();
    let mut i = 0usize;

    // Valeur suivante obligatoire pour une option donnée. Format d'erreur
    // OBSERVÉ dans putty.exe : « option "%s" requires an argument ».
    macro_rules! next_value {
        ($name:expr) => {{
            i += 1;
            match args.get(i) {
                Some(v) => v.clone(),
                None => return Err(format!("option \"{}\" requires an argument", $name)),
            }
        }};
    }

    while i < args.len() {
        let raw = args[i].clone();
        // Positionnel = cible `[user@]host[:port]` (et `-` seul : interdit).
        if !raw.starts_with('-') {
            let (user, host, port) = parse_target(&raw)?;
            if inv.host.is_some() {
                return Err(format!("unexpected argument \"{raw}\"")); // format OBSERVÉ
            }
            if user.is_some() {
                inv.username = user;
            }
            inv.host = Some(host);
            inv.port = port.or(inv.port);
            i += 1;
            continue;
        }

        match normalize_flag(&raw) {
            // --- Aide -------------------------------------------------------
            "help" | "h" | "?" => inv.help = true,

            // --- MCP [DESIGN, docs/MCP_PILOT.md] -----------------------------
            "mcpport" => {
                let v = next_value!(raw);
                inv.mcpport = Some(v.parse::<u16>().map_err(|_| format!("invalid port \"{v}\""))?);
            }
            "nomcp" => inv.nomcp = true,

            // --- Protocoles ---------------------------------------------------
            "ssh" => inv.protocol = Some(Protocol::Ssh),
            "telnet" => inv.protocol = Some(Protocol::Telnet),
            "rlogin" => inv.protocol = Some(Protocol::Rlogin),
            "raw" => inv.protocol = Some(Protocol::Raw),
            "serial" => inv.protocol = Some(Protocol::Serial),

            // --- Cible / ports ------------------------------------------------
            "P" | "port" => {
                let v = next_value!(raw);
                inv.port = Some(v.parse::<u16>().map_err(|_| format!("invalid port \"{v}\""))?);
            }
            "l" | "user" | "username" => inv.username = Some(next_value!(raw)),
            "pw" => inv.password = Some(next_value!(raw)),

            // --- Sessions -------------------------------------------------------
            "load" => inv.load_session = Some(next_value!(raw)),
            "savedsession" => inv.saved_session = Some(next_value!(raw)),
            "sessgroup" => inv.sess_group = Some(next_value!(raw)),
            "nosession" => inv.no_session = true,

            // --- Fenêtre / terminal ------------------------------------------
            "T" | "title" => inv.window_title = Some(next_value!(raw)), // [SUPPOSE] -title alias de -T
            "clear-avtcache" => inv.clear_title = true, // [SUPPOSE] sémantique à confirmer [REA]
            "term" => inv.term_type = Some(next_value!(raw)),
            "old-geometry" => inv.old_geometry = true,
            "cygterm" => inv.ignored.push(raw.clone()), // TERM spécifique cygwin [SUPPOSE]
            "no-antispoof" => inv.ignored.push(raw.clone()),
            "1" | "2" | "ssh1" | "ssh2" | "gssapi" | "f" | "owner" | "bc" => {
                inv.ignored.push(raw.clone())
            }

            // --- Agent / clés ---------------------------------------------------
            // -pageant / -nopageant : chaînes OBSERVées @0x14010b2ac
            // (docs/REA_EVIDENCE.md §3).
            "agent" | "pageant" => inv.try_agent = Some(true),
            "noagent" | "nopageant" => inv.try_agent = Some(false),
            "i" => inv.key_file = Some(PathBuf::from(next_value!(raw))),
            "pwfile" => {
                // -pwfile <fichier> : OBSERVÉ ; on lit la première ligne [DOC].
                let f = next_value!(raw);
                let content = std::fs::read_to_string(&f)
                    .map_err(|e| format!("unable to open password file '{f}': {e}"))?;
                inv.password = content.lines().next().map(str::to_string);
            }
            "hostkey" => {
                let _ = next_value!(raw); // empreinte imposée : stockée, non câblée
                inv.ignored.push(raw.clone());
            }
            "accept-hostkey" => inv.accept_hostkey = true,
            "reject-hostkey" => inv.reject_hostkey = true,

            // --- SSH divers -------------------------------------------------------
            "X" => inv.x11_forward = Some(true),
            "noX11" | "x" => inv.x11_forward = Some(false),
            "C" => inv.compression = Some(true),
            "N" => inv.no_shell = true,
            "L" => inv.tunnels.push(Tunnel { kind: TunnelKind::Local, spec: next_value!(raw) }),
            "R" => inv.tunnels.push(Tunnel { kind: TunnelKind::Remote, spec: next_value!(raw) }),
            "D" => inv.tunnels.push(Tunnel { kind: TunnelKind::Dynamic, spec: next_value!(raw) }),
            "localip" => inv.local_ip = Some(next_value!(raw)),
            "localport" => inv.ignored.push(raw.clone()), // accepté, non câblé [SUPPOSE]
            "nodelay" => inv.nodelay = true,

            // --- Proxy ---------------------------------------------------------
            "proxy" => inv.proxy = Some(ProxyType::parse(&next_value!(raw))?),
            "proxyhost" => inv.proxy_host = Some(next_value!(raw)),
            "proxyport" => {
                let v = next_value!(raw);
                inv.proxy_port = Some(v.parse::<u16>().map_err(|_| format!("invalid proxy port \"{v}\""))?);
            }
            "proxyusername" => inv.proxy_user = Some(next_value!(raw)),
            "proxypassword" => inv.proxy_pass = Some(next_value!(raw)),

            // --- Série -----------------------------------------------------------
            "sercfg" => {
                // Format PuTTY : speed,data,parity,stop[,flow] ex. « 9600,8,n,1 »
                inv.serial_config = Some(next_value!(raw));
            }

            // --- Divers OBSERVés dans le binaire, acceptés et ignorés ----------
            // (docs/REA_EVIDENCE.md §3 : -cleanup, -pgpfp, -host-ca, -sshlog,
            //  -sshrawlog, -nc, -demo-config-box…)
            "cleanup" | "pgpfp" | "demo-config-box" | "demo-terminal" => {
                inv.ignored.push(raw.clone())
            }
            "host-ca" | "sshlog" | "sshrawlog" | "nc" => {
                let _ = next_value!(raw);
                inv.ignored.push(raw.clone());
            }

            _ => return Err(format!("unknown option \"{raw}\"")), // format OBSERVÉ
        }
        i += 1;
    }

    inv.check_consistency()?;
    if inv.help {
        return Ok(None);
    }
    Ok(Some(inv))
}

/// Parse une cible positionnelle : `[user@]host[:port]`, y compris IPv6
/// entre crochets (`[::1]:22`).
fn parse_target(tok: &str) -> Result<(Option<String>, String, Option<u16>), String> {
    // user@host — les hôtes (y compris IPv6 entre crochets) ne contiennent
    // jamais d''@' : on peut séparer sur le dernier '@' en toute sécurité.
    let (user, rest) = match tok.rfind('@') {
        Some(at) => (Some(tok[..at].to_string()), &tok[at + 1..]),
        None => (None, tok),
    };

    // host:port — Attention IPv6 : « [::1]:22 » → port après le ']'.
    let (host, port) = if rest.starts_with('[') {
        let close = rest.find(']').ok_or_else(|| format!("malformed address \"{tok}\""))?;
        let h = &rest[1..close];
        let tail = &rest[close + 1..];
        let p = if let Some(p) = tail.strip_prefix(':') {
            if p.is_empty() {
                None
            } else {
                Some(p.parse::<u16>().map_err(|_| format!("invalid port \"{p}\""))?)
            }
        } else if tail.is_empty() {
            None
        } else {
            return Err(format!("malformed address \"{tok}\""));
        };
        (h.to_string(), p)
    } else if let Some(colon) = rest.rfind(':') {
        // Un seul ':', sinon ambiguïté IPv6 non crochétée → erreur claire.
        if rest[..colon].contains(':') {
            return Err(format!(
                "IPv6 address \"{rest}\" must be enclosed in square brackets"
            ));
        }
        let h = &rest[..colon];
        let p = &rest[colon + 1..];
        let port = if p.is_empty() {
            None
        } else {
            Some(p.parse::<u16>().map_err(|_| format!("invalid port \"{p}\""))?)
        };
        (h.to_string(), port)
    } else {
        (rest.to_string(), None)
    };

    if host.is_empty() {
        // PuTTY : « You need to specify a destination address. » (OBSERVÉ)
        return Err("You need to specify a destination address.".into());
    }
    Ok((user, host, port))
}

/// Texte d'aide (`--help`), généré à partir des options acceptées.
pub fn usage() -> String {
    let mut s = String::new();
    s.push_str("putty-rs 0.1.0 - clone de PuTTY 0.83 en Rust pur\n\n");
    s.push_str("Usage: putty-rs.exe [options] [[user@]host | host:port | [user@]host:port]\n\n");
    s.push_str("Protocoles (choix mutuellement exclusifs) :\n  \
        -ssh  -telnet  -rlogin  -raw  -serial\n");
    s.push_str("Ports par défaut : ssh=22  telnet=23  rlogin=513  raw=10001 [SUPPOSE]  serial=-\n\n");
    s.push_str("Cible et authentification :\n  \
        -P <n> | -port <n>     port\n  \
        -l <user>              utilisateur  (alias: -user)\n  \
        -pw <pass>             mot de passe (SSH uniquement, comme PuTTY)\n  \
        -i <ppk>               fichier de clé privée\n  \
        -agent | -noagent      forcer/désactiver l'agent d'authentification\n\n");
    s.push_str("Sessions (registre HKCU\\Software\\SimonTatham\\PuTTY\\Sessions) :\n  \
        -load <session>        charger une session sauvegardée\n  \
        --savedsession <s>     identique à -load (forme « réglage »)\n  \
        --sessgroup <groupe>   groupe de sessions\n  \
        -nosession             ne charger aucune session du registre\n\n");
    s.push_str("Fenêtre / terminal :\n  \
        -T <titre>             titre de fenêtre (alias: -title)\n  \
        -clear-avtcache        désactive le titre dynamique distant [SUPPOSE]\n  \
        -term <type>           type de terminal annoncé [SUPPOSE]\n  \
        -cygterm  -no-antispoof  -old-geometry  (acceptées, ignorées)\n\n");
    s.push_str("SSH :\n  \
        -X | -noX11            forwarding X11\n  \
        -C                     compression\n  \
        -N                     pas de shell (tunnels seuls)\n  \
        -L <spec>  -R <spec>  -D <port>   tunnels locaux/distants/dynamiques\n  \
        --localip <ip>         interface locale\n  \
        -accept-hostkey | -reject-hostkey  politique hôte inconnu\n  \
        -nodelay               TCP_NODELAY\n\n");
    s.push_str("Proxy (acceptées, câblage minimal) :\n  \
        -proxy none|http|socks4|socks5|local|telnet\n  \
        --proxyhost <h> --proxyport <n> --proxyusername <u> --proxypassword <p>\n\n");
    s.push_str("Série :\n  -sercfg <config>   ex. 9600,8,n,1\n\n");
    s.push_str("Divers : --help | -h | -?\n");
    s
}

// =============================================================================
// Tests unitaires
// =============================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Invocation {
        parse(args.iter().map(|s| s.to_string()))
            .expect("parse ok")
            .expect("not help")
    }

    #[test]
    fn target_user_host_port() {
        let inv = p(&["alice@example.com:2222"]);
        assert_eq!(inv.username.as_deref(), Some("alice"));
        assert_eq!(inv.host.as_deref(), Some("example.com"));
        assert_eq!(inv.port, Some(2222));
    }

    #[test]
    fn target_ipv6_brackets() {
        let inv = p(&["[::1]:23"]);
        assert_eq!(inv.host.as_deref(), Some("::1"));
        assert_eq!(inv.port, Some(23));
    }

    #[test]
    fn protocol_and_options() {
        let inv = p(&["-telnet", "-P", "2323", "-l", "bob", "-load", "monserveur", "h"]);
        assert_eq!(inv.protocol, Some(Protocol::Telnet));
        assert_eq!(inv.port, Some(2323));
        assert_eq!(inv.username.as_deref(), Some("bob"));
        assert_eq!(inv.load_session.as_deref(), Some("monserveur"));
        assert_eq!(inv.host.as_deref(), Some("h"));
    }

    #[test]
    fn double_dash_forms_accepted() {
        let inv = p(&["--proxyhost", "p.example", "--proxyport", "1080", "-proxy", "socks5", "h"]);
        assert_eq!(inv.proxy, Some(ProxyType::Socks5));
        assert_eq!(inv.proxy_host.as_deref(), Some("p.example"));
        assert_eq!(inv.proxy_port, Some(1080));
    }

    #[test]
    fn pw_only_with_ssh() {
        // -pw autorisé sans protocole explicite…
        assert!(p(&["-pw", "s3cret", "h"]).password.as_deref() == Some("s3cret"));
        // …refusé explicitement avec telnet (message PuTTY observé).
        let e = parse(["-telnet".into(), "-pw".into(), "x".into(), "h".into()]).unwrap_err();
        assert!(e.contains("-pw option can only be used with the SSH protocol"));
    }

    #[test]
    fn unknown_option_message() {
        let e = parse(["-zzz".into()]).unwrap_err();
        assert_eq!(e, "unknown option \"-zzz\"");
    }

    #[test]
    fn help_returns_none() {
        assert!(parse(["--help".into()]).unwrap().is_none());
    }
}
