use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, RwLock},
};

use anyhow::{Result, ensure};
use ipnet::Ipv6Net;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, TcpStream};

use crate::server::ConnectionFilter;

const MAX_PREFIXES: usize = 32;
const MAX_PREFIX_JSON_BYTES: usize = 4096;
const MIN_LOCAL_PREFIX_BITS: u8 = 64;
const LISTEN_BACKLOG: i32 = 128;

#[derive(Clone, Default)]
pub struct LanPolicy(Arc<RwLock<Vec<Ipv6Net>>>);

impl LanPolicy {
    pub fn new(raw: &str) -> Result<Self> {
        Ok(Self(Arc::new(RwLock::new(parse_prefixes(raw)?))))
    }

    pub fn update(&self, raw: &str) {
        // An invalid or lost-network update must revoke, not retain, old IPv6 access.
        let prefixes = parse_prefixes(raw).unwrap_or_default();
        *self.0.write().unwrap_or_else(|error| error.into_inner()) = prefixes;
    }

    pub fn allows(&self, peer: SocketAddr) -> bool {
        let IpAddr::V6(address) = peer.ip() else {
            return true;
        };
        if address.to_ipv4_mapped().is_some()
            || address.is_loopback()
            || address.is_unicast_link_local()
        {
            return true;
        }
        if address.is_unspecified() || address.is_multicast() {
            return false;
        }
        self.0
            .read()
            .is_ok_and(|prefixes| prefixes.iter().any(|net| net.contains(&address)))
    }

    pub fn filter(&self) -> ConnectionFilter {
        let policy = self.clone();
        Arc::new(move |peer| policy.allows(peer))
    }
}

fn parse_prefixes(raw: &str) -> Result<Vec<Ipv6Net>> {
    ensure!(
        raw.len() <= MAX_PREFIX_JSON_BYTES,
        "network prefix input too large"
    );
    let values: Vec<String> = serde_json::from_str(raw)?;
    ensure!(values.len() <= MAX_PREFIXES, "too many network prefixes");
    let mut prefixes = Vec::new();
    for value in values {
        let net: Ipv6Net = value.parse()?;
        let address = net.addr();
        ensure!(
            net.prefix_len() >= MIN_LOCAL_PREFIX_BITS
                && !address.is_unspecified()
                && !address.is_loopback()
                && !address.is_multicast()
                && !address.is_unicast_link_local()
                && address.to_ipv4_mapped().is_none(),
            "invalid local IPv6 prefix"
        );
        prefixes.push(net.trunc());
    }
    prefixes.sort_unstable();
    prefixes.dedup();
    Ok(prefixes)
}

fn ipv6_unavailable(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EAFNOSUPPORT | libc::EPROTONOSUPPORT | libc::ENOPROTOOPT | libc::EADDRNOTAVAIL)
    )
}

fn bind_socket(address: SocketAddr) -> io::Result<TcpListener> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    if address.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    socket.listen(LISTEN_BACKLOG)?;
    TcpListener::from_std(socket.into())
}

pub fn bind(port: u16) -> io::Result<TcpListener> {
    match bind_socket(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))) {
        Err(error) if ipv6_unavailable(&error) => {
            bind_socket(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
        }
        result => result,
    }
}

pub struct MetadataListener {
    inner: TcpListener,
    filter: ConnectionFilter,
}

impl MetadataListener {
    pub fn new(inner: TcpListener, filter: ConnectionFilter) -> Self {
        Self { inner, filter }
    }
}

impl axum::serve::Listener for MetadataListener {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, peer) =
                <TcpListener as axum::serve::Listener>::accept(&mut self.inner).await;
            if (self.filter)(peer) {
                return (stream, peer);
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn peer(address: &str) -> SocketAddr {
        SocketAddr::new(address.parse().unwrap(), 12345)
    }

    #[test]
    fn preserves_ipv4_and_local_only_ipv6_without_prefixes() {
        let policy = LanPolicy::default();
        for address in [
            "127.0.0.1",
            "192.168.1.20",
            "::ffff:192.168.1.20",
            "::1",
            "fe80::123",
        ] {
            assert!(policy.allows(peer(address)));
        }
        for address in ["2001:db8:1::2", "fd00:1::2", "::", "ff02::1"] {
            assert!(!policy.allows(peer(address)));
        }
    }

    #[test]
    fn admits_only_current_interface_prefix_and_revokes_previous_network() {
        let policy = LanPolicy::new(r#"["2001:db8:1::123/64", "2001:db8:1::456/64"]"#).unwrap();
        assert_eq!(policy.0.read().unwrap().len(), 1);
        assert!(policy.allows(peer("2001:db8:1::20")));
        assert!(!policy.allows(peer("2001:db8:2::20")));
        policy.update(r#"["fd00:2::123/64"]"#);
        assert!(!policy.allows(peer("2001:db8:1::20")));
        assert!(policy.allows(peer("fd00:2::20")));
        policy.update("[]");
        assert!(!policy.allows(peer("fd00:2::20")));
    }

    #[test]
    fn invalid_update_fails_closed_instead_of_keeping_a_stale_prefix() {
        let policy = LanPolicy::new(r#"["2001:db8:1::1/64"]"#).unwrap();
        policy.update(r#"["::/0"]"#);
        assert!(!policy.allows(peer("2001:db8:1::20")));
        for input in [
            r#"["2001:db8::/32"]"#,
            r#"["192.0.2.1/24"]"#,
            r#"["::1/128"]"#,
            r#"["ff02::/64"]"#,
            r#"["fe80::/64"]"#,
            r#"["::ffff:192.0.2.1/128"]"#,
            "{}",
            "[1]",
        ] {
            assert!(LanPolicy::new(input).is_err());
        }
        assert!(LanPolicy::new(&serde_json::to_string(&vec!["fd00::/64"; 33]).unwrap()).is_err());
        assert!(LanPolicy::new(&" ".repeat(4097)).is_err());
    }

    #[test]
    fn poison_does_not_grant_global_ipv6_access() {
        let policy = LanPolicy::new(r#"["2001:db8:1::1/64"]"#).unwrap();
        let worker = policy.clone();
        let _ = std::thread::spawn(move || {
            let _guard = worker.0.write().unwrap();
            panic!("synthetic poisoned policy");
        })
        .join();
        assert!(!policy.allows(peer("2001:db8:1::2")));
        assert!(policy.allows(peer("127.0.0.1")));
    }

    #[test]
    fn only_unavailable_ipv6_errors_allow_ipv4_fallback() {
        for code in [
            libc::EAFNOSUPPORT,
            libc::EPROTONOSUPPORT,
            libc::ENOPROTOOPT,
            libc::EADDRNOTAVAIL,
        ] {
            assert!(ipv6_unavailable(&io::Error::from_raw_os_error(code)));
        }
        for code in [libc::EADDRINUSE, libc::EACCES, libc::EINVAL] {
            assert!(!ipv6_unavailable(&io::Error::from_raw_os_error(code)));
        }
    }

    #[tokio::test]
    async fn one_socket_accepts_both_loopback_families_and_conflicts_are_errors() {
        let listener = bind(0).unwrap();
        let address = listener.local_addr().unwrap();
        assert!(
            address.is_ipv6(),
            "this qualification requires host IPv6 support"
        );
        for host in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let client = TcpStream::connect(SocketAddr::new(host, address.port()))
                .await
                .unwrap();
            let (server, peer) = listener.accept().await.unwrap();
            assert!(LanPolicy::default().allows(peer));
            drop(server);
            drop(client);
        }
        assert_eq!(
            bind(address.port()).unwrap_err().kind(),
            io::ErrorKind::AddrInUse
        );
    }

    #[tokio::test]
    async fn metadata_filter_rejects_before_http_and_can_admit_a_later_connection() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let allowed = Arc::new(AtomicBool::new(false));
        let gate = allowed.clone();
        let listener =
            MetadataListener::new(listener, Arc::new(move |_| gate.load(Ordering::SeqCst)));
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/", axum::routing::get(|| async { "ok" })),
            )
            .await
            .unwrap();
        });
        let mut denied = TcpStream::connect(address).await.unwrap();
        denied
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = [0; 16];
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            denied.read(&mut response),
        )
        .await
        .unwrap();
        assert!(
            matches!(read, Ok(0) | Err(_)),
            "HTTP data leaked before policy"
        );
        allowed.store(true, Ordering::SeqCst);
        let mut accepted = TcpStream::connect(address).await.unwrap();
        accepted
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut data = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            accepted.read_to_end(&mut data),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(data.starts_with(b"HTTP/1.1 200"));
        task.abort();
    }
}
