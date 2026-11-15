use std::fmt::Write as _;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

pub fn java_host_address(ip: IpAddr) -> String {
    match normalize_ip(ip) {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => ipv6_full(&v6),
    }
}

fn ipv6_full(v6: &Ipv6Addr) -> String {
    let mut out = String::with_capacity(39);
    for (i, segment) in v6.segments().iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        let _ = write!(out, "{segment:x}");
    }
    out
}

pub fn ip_port(addr: SocketAddr) -> String {
    format!("{}:{}", java_host_address(addr.ip()), addr.port())
}

pub fn java_socket_address(addr: SocketAddr) -> String {
    match normalize_ip(addr.ip()) {
        IpAddr::V4(v4) => format!("/{v4}:{}", addr.port()),
        IpAddr::V6(v6) => format!("/[{}]:{}", ipv6_full(&v6), addr.port()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_shapes() {
        let v4: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(ip_port(v4), "127.0.0.1:5000");
        assert_eq!(java_socket_address(v4), "/127.0.0.1:5000");
        let v6: SocketAddr = "[::1]:5000".parse().unwrap();
        assert_eq!(ip_port(v6), "0:0:0:0:0:0:0:1:5000");
        assert_eq!(java_socket_address(v6), "/[0:0:0:0:0:0:0:1]:5000");
        let mapped: SocketAddr = "[::ffff:10.1.2.3]:7".parse().unwrap();
        assert_eq!(ip_port(mapped), "10.1.2.3:7");
        let v6b: SocketAddr = "[2001:db8::ff00:42:8329]:1".parse().unwrap();
        assert_eq!(ip_port(v6b), "2001:db8:0:0:0:ff00:42:8329:1");
    }
}
