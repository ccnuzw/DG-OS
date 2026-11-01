//! Interface choice and the listening socket (technical design §1).

use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// D107: default preferred port when `lan.port` is unset.
pub const DEFAULT_PORT: u16 = 8635;
/// D107: the preferred port and the 19 after it.
pub const FALLBACK_PORTS: u16 = 19;

/// Fetch standard "bad ports": browsers refuse to connect to them (D107).
const FETCH_BAD_PORTS: [u16; 83] = [
    0, 1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101,
    102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427,
    465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990,
    993, 995, 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667,
    6668, 6669, 6679, 6697, 10080,
];

/// `set_lan_port` validation (B10-69): 1024..=65535 and not a Fetch bad port.
pub fn valid_port(port: i64) -> Option<u16> {
    let port = u16::try_from(port).ok()?;
    (port >= 1024 && !FETCH_BAD_PORTS.contains(&port)).then_some(port)
}

/// 10/8, 172.16/12, 192.168/16 (D088). Loopback, link-local, CGNAT and public are refused.
pub fn is_private_lan(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// The host's view of its interfaces. Production is [`SystemNetwork`]; tests
/// substitute addresses they can actually bind.
pub trait Network: Send + Sync + 'static {
    /// Addresses the service may bind, in enumeration order.
    fn candidate_ipv4(&self) -> Vec<Ipv4Addr>;
    /// The source address of the default route, if there is one.
    fn default_route_ipv4(&self) -> Option<Ipv4Addr>;
}

/// The default route's address when it is a candidate, else the first candidate.
pub fn pick_address(network: &dyn Network) -> Option<Ipv4Addr> {
    let candidates = network.candidate_ipv4();
    match network.default_route_ipv4() {
        Some(ip) if candidates.contains(&ip) => Some(ip),
        _ => candidates.first().copied(),
    }
}

/// `getifaddrs` (or the platform equivalent) filtered to private IPv4; the
/// default route's source from a UDP `connect`, which sends nothing (phase 0).
pub struct SystemNetwork;

impl Network for SystemNetwork {
    fn candidate_ipv4(&self) -> Vec<Ipv4Addr> {
        match if_addrs::get_if_addrs() {
            Ok(interfaces) => interfaces
                .into_iter()
                .filter_map(|interface| match interface.addr {
                    if_addrs::IfAddr::V4(v4) if is_private_lan(v4.ip) => Some(v4.ip),
                    _ => None,
                })
                .collect(),
            Err(_) => {
                tracing::warn!("network interfaces could not be listed");
                Vec::new()
            }
        }
    }

    fn default_route_ipv4(&self) -> Option<Ipv4Addr> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
        match socket.local_addr().ok()? {
            SocketAddr::V4(local) => Some(*local.ip()),
            SocketAddr::V6(_) => None,
        }
    }
}

/// One IPv4 listener on `ip:port`, created with `socket2` and without
/// `SO_REUSEADDR`/`SO_REUSEPORT`, so another program listening on the wildcard
/// address makes the port count as taken (D105). Bind is the listen itself:
/// there is no probe before it.
pub fn bind_listener(ip: Ipv4Addr, port: u16) -> io::Result<std::net::TcpListener> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((ip, port)).into())?;
    socket.listen(128)?;
    Ok(socket.into())
}

/// `P..=P+19`, capped at 65535 (D107).
pub fn port_candidates(preferred: u16) -> impl Iterator<Item = u16> {
    let last = (preferred as u32 + FALLBACK_PORTS as u32).min(u16::MAX as u32) as u16;
    preferred..=last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_validation_follows_d107() {
        assert_eq!(valid_port(8635), Some(8635));
        assert_eq!(valid_port(1024), Some(1024));
        assert_eq!(valid_port(65535), Some(65535));
        for bad in [0, 80, 1023, 65536, -1, 6000, 6665, 6669, 10080, 5060, 2049] {
            assert_eq!(valid_port(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_rfc1918_addresses_are_private() {
        for ip in ["10.0.0.1", "172.16.0.1", "172.31.255.254", "192.168.8.193"] {
            assert!(is_private_lan(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "169.254.1.1",
            "172.32.0.1",
            "100.64.0.1",
            "8.8.8.8",
            "0.0.0.0",
            "192.0.2.1",
        ] {
            assert!(!is_private_lan(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn candidates_stop_at_65535() {
        assert_eq!(port_candidates(8635).count(), 20);
        assert_eq!(
            port_candidates(65530).collect::<Vec<_>>().last(),
            Some(&65535)
        );
        assert_eq!(port_candidates(65530).count(), 6);
    }

    struct Fixed(Vec<Ipv4Addr>, Option<Ipv4Addr>);
    impl Network for Fixed {
        fn candidate_ipv4(&self) -> Vec<Ipv4Addr> {
            self.0.clone()
        }
        fn default_route_ipv4(&self) -> Option<Ipv4Addr> {
            self.1
        }
    }

    #[test]
    fn the_default_route_wins_only_when_it_is_a_candidate() {
        let a: Ipv4Addr = "192.168.1.5".parse().unwrap();
        let b: Ipv4Addr = "10.0.0.7".parse().unwrap();
        assert_eq!(pick_address(&Fixed(vec![a, b], Some(b))), Some(b));
        assert_eq!(
            pick_address(&Fixed(vec![a, b], Some("8.8.8.8".parse().unwrap()))),
            Some(a)
        );
        assert_eq!(pick_address(&Fixed(vec![], Some(a))), None);
    }

    #[test]
    fn the_system_network_offers_only_private_addresses() {
        for ip in SystemNetwork.candidate_ipv4() {
            assert!(is_private_lan(ip));
        }
    }

    #[test]
    fn a_wildcard_listener_of_another_program_makes_the_port_taken() {
        // The occupant uses std (which sets SO_REUSEADDR on Unix) on the
        // wildcard address, like the Docker Desktop case of phase 0.
        let occupant = std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = occupant.local_addr().unwrap().port();
        let error = bind_listener(Ipv4Addr::LOCALHOST, port).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        drop(occupant);
        // Free again: binding works and is the listener itself.
        let listener = bind_listener(Ipv4Addr::LOCALHOST, port).unwrap();
        assert_eq!(listener.local_addr().unwrap().port(), port);
    }

    #[test]
    fn the_listener_does_not_set_so_reuseaddr() {
        let listener = bind_listener(Ipv4Addr::LOCALHOST, 0).unwrap();
        let socket = Socket::from(listener);
        assert!(!socket.reuse_address().unwrap());
    }
}
