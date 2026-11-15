use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use super::addr::normalize_ip;

pub const V2_PREFIX: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];

const V1_MAX_LENGTH: usize = 108;

pub const MISSING_SOURCE: &str = "HAProxyMessage missing source address";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyHeader {
    Incomplete,
    Source { source: SocketAddr, consumed: usize },
}

pub fn parse(buf: &[u8]) -> Result<ProxyHeader, String> {
    if buf.len() < 13 {
        return Ok(ProxyHeader::Incomplete);
    }
    // The 13th byte decides between the binary and the text header.
    if buf[..12] == V2_PREFIX && buf[12] != 1 {
        parse_v2(buf)
    } else {
        parse_v1(buf)
    }
}

fn parse_v1(buf: &[u8]) -> Result<ProxyHeader, String> {
    let eol = buf.windows(2).position(|w| w == b"\r\n");
    let Some(eol) = eol else {
        if buf.len() > V1_MAX_LENGTH {
            return Err(format!(
                "header length (over {}) exceeds the allowed maximum ({V1_MAX_LENGTH})",
                buf.len()
            ));
        }
        return Ok(ProxyHeader::Incomplete);
    };
    if eol > V1_MAX_LENGTH {
        return Err(format!(
            "header length ({eol}) exceeds the allowed maximum ({V1_MAX_LENGTH})"
        ));
    }
    let header: String = buf[..eol]
        .iter()
        .map(|&b| if b.is_ascii() { b as char } else { '\u{FFFD}' })
        .collect();
    let source = decode_v1(&header)?;
    Ok(ProxyHeader::Source {
        source,
        consumed: eol + 2,
    })
}

fn decode_v1(header: &str) -> Result<SocketAddr, String> {
    let parts: Vec<&str> = header.trim_end_matches(' ').split(' ').collect();
    if parts.len() < 2 {
        return Err(format!(
            "invalid header: {header} (expected: 'PROXY' and proxied protocol values)"
        ));
    }
    if parts[0] != "PROXY" {
        return Err(format!("unknown identifier: {}", parts[0]));
    }
    const NAMES: [&str; 7] = [
        "UNKNOWN",
        "TCP4",
        "TCP6",
        "UDP4",
        "UDP6",
        "UNIX_STREAM",
        "UNIX_DGRAM",
    ];
    if !NAMES.contains(&parts[1]) {
        return Err(format!("unknown proxied protocol: {}", parts[1]));
    }
    let v6 = match parts[1] {
        "TCP4" => false,
        "TCP6" => true,
        "UNKNOWN" => return Err(MISSING_SOURCE.to_owned()),
        other => return Err(format!("unsupported v1 proxied protocol: {other}")),
    };
    if parts.len() != 6 {
        return Err(format!(
            "invalid TCP4/6 header: {header} (expected: 6 parts)"
        ));
    }
    let invalid = || "invalid HAProxy message".to_owned();
    let source_port = v1_port(parts[4]).ok_or_else(invalid)?;
    v1_port(parts[5]).ok_or_else(invalid)?;
    let source = v1_address(parts[2], v6).ok_or_else(invalid)?;
    v1_address(parts[3], v6).ok_or_else(invalid)?;
    Ok(SocketAddr::new(normalize_ip(source), source_port))
}

fn v1_port(text: &str) -> Option<u16> {
    text.parse::<u16>().ok().filter(|&port| port != 0)
}

fn v1_address(text: &str, v6: bool) -> Option<IpAddr> {
    if v6 {
        let inner = text
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .unwrap_or(text);
        inner.parse::<Ipv6Addr>().ok().map(IpAddr::V6)
    } else {
        text.parse::<Ipv4Addr>().ok().map(IpAddr::V4)
    }
}

fn parse_v2(buf: &[u8]) -> Result<ProxyHeader, String> {
    if buf.len() < 16 {
        return Ok(ProxyHeader::Incomplete);
    }
    let address_len = u16::from_be_bytes([buf[14], buf[15]]) as usize;
    let total = 16 + address_len;
    if buf.len() < total {
        return Ok(ProxyHeader::Incomplete);
    }
    let source = decode_v2(&buf[..total])?;
    Ok(ProxyHeader::Source {
        source,
        consumed: total,
    })
}

fn decode_v2(header: &[u8]) -> Result<SocketAddr, String> {
    let ver_cmd = header[12];
    match ver_cmd >> 4 {
        2 => {}
        1 => return Err(format!("version 1 unsupported: 0x{ver_cmd:x}")),
        version => return Err(format!("unknown version: {version}")),
    }
    match ver_cmd & 0x0F {
        0x01 => {}
        0x00 => return Err(MISSING_SOURCE.to_owned()),
        command => return Err(format!("unknown command: {command}")),
    }
    let family_byte = header[13];
    let family = match family_byte {
        0x11 | 0x12 => Family::V4,
        0x21 | 0x22 => Family::V6,
        0x31 | 0x32 => Family::Unix,
        0x00 => return Err(MISSING_SOURCE.to_owned()),
        other => return Err(format!("unknown address family: 0x{other:02x}")),
    };
    let info_len = u16::from_be_bytes([header[14], header[15]]) as usize;
    let body = &header[16..];
    let available = info_len.min(body.len());
    match family {
        Family::Unix => {
            if info_len < 216 || body.len() < 216 {
                return Err(format!(
                    "incomplete UNIX socket address information: {available} bytes (expected: 216+ bytes)"
                ));
            }
            Err(MISSING_SOURCE.to_owned())
        }
        Family::V4 => {
            if info_len < 12 || body.len() < 12 {
                return Err(format!(
                    "incomplete IPv4 address information: {available} bytes (expected: 12+ bytes)"
                ));
            }
            let ip = Ipv4Addr::new(body[0], body[1], body[2], body[3]);
            let port = u16::from_be_bytes([body[8], body[9]]);
            Ok(SocketAddr::new(IpAddr::V4(ip), port))
        }
        Family::V6 => {
            if info_len < 36 || body.len() < 36 {
                return Err(format!(
                    "incomplete IPv6 address information: {available} bytes (expected: 36+ bytes)"
                ));
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&body[..16]);
            let ip = normalize_ip(IpAddr::V6(Ipv6Addr::from(octets)));
            let port = u16::from_be_bytes([body[32], body[33]]);
            Ok(SocketAddr::new(ip, port))
        }
    }
}

#[derive(Clone, Copy)]
enum Family {
    V4,
    V6,
    Unix,
}

pub fn encode_v2(source: SocketAddr, destination: SocketAddr) -> Vec<u8> {
    let mut out = V2_PREFIX.to_vec();
    out.push(0x21);
    match (source.ip(), destination.ip()) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            out.push(0x11);
            out.extend_from_slice(&12u16.to_be_bytes());
            out.extend_from_slice(&s.octets());
            out.extend_from_slice(&d.octets());
        }
        (s, d) => {
            let to_v6 = |ip: IpAddr| match ip {
                IpAddr::V4(v4) => v4.to_ipv6_mapped(),
                IpAddr::V6(v6) => v6,
            };
            out.push(0x21);
            out.extend_from_slice(&36u16.to_be_bytes());
            out.extend_from_slice(&to_v6(s).octets());
            out.extend_from_slice(&to_v6(d).octets());
        }
    }
    out.extend_from_slice(&source.port().to_be_bytes());
    out.extend_from_slice(&destination.port().to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(buf: &[u8]) -> Result<ProxyHeader, String> {
        parse(buf)
    }

    #[test]
    fn v1_tcp4_and_leftover() {
        let mut data = b"PROXY TCP4 203.0.113.9 10.0.0.1 51000 12346\r\n".to_vec();
        let header_len = data.len();
        data.push(0x01);
        assert_eq!(
            source(&data).unwrap(),
            ProxyHeader::Source {
                source: "203.0.113.9:51000".parse().unwrap(),
                consumed: header_len
            }
        );
    }

    #[test]
    fn v1_tcp6() {
        let data = b"PROXY TCP6 2001:db8::1 ::1 4000 12346\r\n";
        match source(data).unwrap() {
            ProxyHeader::Source { source, .. } => {
                assert_eq!(source, "[2001:db8::1]:4000".parse().unwrap())
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn v1_errors() {
        assert_eq!(source(b"PROXY").unwrap(), ProxyHeader::Incomplete);
        assert_eq!(source(b"PROXY UNKNOWN\r\n").unwrap_err(), MISSING_SOURCE);
        assert_eq!(
            source(b"HELLO TCP4 1.1.1.1 2.2.2.2 1 2\r\n").unwrap_err(),
            "unknown identifier: HELLO"
        );
        assert_eq!(
            source(b"PROXY UDP4 1.1.1.1 2.2.2.2 1 2\r\n").unwrap_err(),
            "unsupported v1 proxied protocol: UDP4"
        );
        assert_eq!(
            source(b"PROXY TCP4 1.1.1.1 2.2.2.2 1\r\n").unwrap_err(),
            "invalid TCP4/6 header: PROXY TCP4 1.1.1.1 2.2.2.2 1 (expected: 6 parts)"
        );
        assert_eq!(
            source(b"PROXY TCP4 1.1.1.1 2.2.2.2 0 2\r\n").unwrap_err(),
            "invalid HAProxy message"
        );
        assert_eq!(
            source(b"PROXY TCP4 1.1.1.999 2.2.2.2 1 2\r\n").unwrap_err(),
            "invalid HAProxy message"
        );
        assert_eq!(
            source(b"PROXY XYZ 1.1.1.1 2.2.2.2 1 2\r\n").unwrap_err(),
            "unknown proxied protocol: XYZ"
        );
        let long = vec![b'A'; 109];
        assert_eq!(
            source(&long).unwrap_err(),
            "header length (over 109) exceeds the allowed maximum (108)"
        );
        assert!(matches!(
            source(b"PROXY TCP4 1.1.1.1 2.2.2.2 1 2  \r\n").unwrap(),
            ProxyHeader::Source { .. }
        ));
    }

    #[test]
    fn v2_ipv4_ipv6_and_errors() {
        let src: SocketAddr = "198.51.100.4:40000".parse().unwrap();
        let dst: SocketAddr = "10.0.0.1:12346".parse().unwrap();
        let mut data = encode_v2(src, dst);
        let len = data.len();
        data.extend_from_slice(&[0x01, 0x05]);
        assert_eq!(
            source(&data).unwrap(),
            ProxyHeader::Source {
                source: src,
                consumed: len
            }
        );
        let src6: SocketAddr = "[2001:db8::7]:1".parse().unwrap();
        let dst6: SocketAddr = "[::1]:2".parse().unwrap();
        match source(&encode_v2(src6, dst6)).unwrap() {
            ProxyHeader::Source { source, .. } => assert_eq!(source, src6),
            other => panic!("{other:?}"),
        }
        assert_eq!(source(&data[..20]).unwrap(), ProxyHeader::Incomplete);
        let mut local = encode_v2(src, dst);
        local[12] = 0x20;
        assert_eq!(source(&local).unwrap_err(), MISSING_SOURCE);
        let mut bad = encode_v2(src, dst);
        bad[12] = 0x31;
        assert_eq!(source(&bad).unwrap_err(), "unknown version: 3");
        bad[12] = 0x11;
        assert_eq!(source(&bad).unwrap_err(), "version 1 unsupported: 0x11");
        bad[12] = 0x2F;
        assert_eq!(source(&bad).unwrap_err(), "unknown command: 15");
        let mut short = V2_PREFIX.to_vec();
        short.extend_from_slice(&[0x21, 0x11, 0x00, 0x04, 1, 2, 3, 4]);
        assert_eq!(
            source(&short).unwrap_err(),
            "incomplete IPv4 address information: 4 bytes (expected: 12+ bytes)"
        );
        let mut family = encode_v2(src, dst);
        family[13] = 0x41;
        assert_eq!(source(&family).unwrap_err(), "unknown address family: 0x41");
    }
}
