use std::net::{IpAddr, SocketAddr};

pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

pub fn ip_port(addr: SocketAddr) -> String {
    SocketAddr::new(normalize_ip(addr.ip()), addr.port()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_text() {
        let v4: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(ip_port(v4), "127.0.0.1:5000");
        let v6: SocketAddr = "[::1]:5000".parse().unwrap();
        assert_eq!(ip_port(v6), "[::1]:5000");
        let mapped: SocketAddr = "[::ffff:10.1.2.3]:7".parse().unwrap();
        assert_eq!(ip_port(mapped), "10.1.2.3:7");
    }
}
