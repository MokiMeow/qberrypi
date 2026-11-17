//! The few bytes of DNS that the generator needs: a query in the wire format, the header of a
//! reply, and the A records of an answer for the checks before a run. Nothing else of a message
//! is read, so a reply costs the generator a few loads, not a parse.

use std::net::Ipv4Addr;

/// The record type A.
pub const TYPE_A: u16 = 1;
/// The class IN.
const CLASS_IN: u16 = 1;
/// The length of a DNS header.
pub const HEADER_LEN: usize = 12;
/// The longest label.
const MAX_LABEL: usize = 63;
/// The longest name on the wire, its length bytes and the root's 0 included.
const MAX_NAME: usize = 255;
/// The response code SERVFAIL.
pub const RCODE_SERVFAIL: u8 = 2;
/// The response code NXDOMAIN.
pub const RCODE_NXDOMAIN: u8 = 3;
/// The response code REFUSED.
pub const RCODE_REFUSED: u8 = 5;

/// Why a name cannot be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// No label at all, or an empty label (`a..b`).
    EmptyLabel,
    /// A label of more than 63 bytes.
    LabelTooLong,
    /// More than 255 bytes on the wire.
    NameTooLong,
    /// A byte that no host name has: a space, a control character or one beyond ASCII.
    BadByte,
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NameError::EmptyLabel => "the name has an empty label",
            NameError::LabelTooLong => "a label of the name is longer than 63 bytes",
            NameError::NameTooLong => "the name is longer than 255 bytes",
            NameError::BadByte => "the name has a byte that no host name has",
        })
    }
}

impl std::error::Error for NameError {}

/// Writes the query for `name` and `qtype` under `id` into `out`, which is cleared first:
/// a header with RD set and one question of class IN, no EDNS. A trailing dot is allowed.
pub fn encode_query(out: &mut Vec<u8>, id: u16, name: &str, qtype: u16) -> Result<(), NameError> {
    out.clear();
    out.extend_from_slice(&id.to_be_bytes());
    // RD; one question; no answers, authorities or additionals.
    out.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    let name = name.strip_suffix('.').unwrap_or(name);
    for label in name.split('.') {
        if label.is_empty() {
            return Err(NameError::EmptyLabel);
        }
        if label.len() > MAX_LABEL {
            return Err(NameError::LabelTooLong);
        }
        if !label.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(NameError::BadByte);
        }
        // At most 63, so it fits a byte.
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    if out.len() - HEADER_LEN > MAX_NAME {
        return Err(NameError::NameTooLong);
    }
    out.extend_from_slice(&qtype.to_be_bytes());
    out.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(())
}

/// Puts `id` into the header of `packet`; a packet shorter than two bytes is left alone.
pub fn set_id(packet: &mut [u8], id: u16) {
    if let Some(head) = packet.get_mut(..2) {
        head.copy_from_slice(&id.to_be_bytes());
    }
}

/// What the generator reads of a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub id: u16,
    /// The QR bit: a response, not a query.
    pub response: bool,
    /// The response code of the header (the low four bits; no EDNS extension).
    pub rcode: u8,
}

/// The header of `reply`, or `None` for fewer than [`HEADER_LEN`] bytes.
pub fn header(reply: &[u8]) -> Option<Header> {
    let head = reply.get(..HEADER_LEN)?;
    Some(Header {
        id: u16::from_be_bytes([head[0], head[1]]),
        response: head[2] & 0x80 != 0,
        rcode: head[3] & 0x0f,
    })
}

/// Whether an answer with this response code gives a client nothing: SERVFAIL or REFUSED. A
/// load that is answered so is not served, however fast the answer came. NXDOMAIN is an answer.
pub fn is_failure(rcode: u8) -> bool {
    matches!(rcode, RCODE_SERVFAIL | RCODE_REFUSED)
}

/// A reply read for the checks before a run: its response code and the addresses of its A
/// records; the other records are counted and not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub rcode: u8,
    pub v4: Vec<Ipv4Addr>,
    /// Answer records other than A: a CNAME, an AAAA and so on.
    pub other_records: usize,
}

impl Answer {
    /// Whether this is what a blocker answers for a blocked name: NOERROR with addresses, all of
    /// them 0.0.0.0, as qberrypi, Pi-hole and Blocky do by default; or NXDOMAIN with no record,
    /// as a server set to answer blocked names that way does.
    pub fn is_blocked(&self) -> bool {
        let null = self.rcode == 0
            && !self.v4.is_empty()
            && self.other_records == 0
            && self.v4.iter().all(Ipv4Addr::is_unspecified);
        let nxdomain = self.rcode == RCODE_NXDOMAIN && self.v4.is_empty();
        null || nxdomain
    }

    /// What it was, in words, without its addresses.
    pub fn describe(&self) -> String {
        let code = match self.rcode {
            0 => "NOERROR".to_owned(),
            RCODE_SERVFAIL => "SERVFAIL".to_owned(),
            RCODE_NXDOMAIN => "NXDOMAIN".to_owned(),
            RCODE_REFUSED => "REFUSED".to_owned(),
            other => format!("RCODE {other}"),
        };
        let unspecified = self.v4.iter().filter(|a| a.is_unspecified()).count();
        match (self.v4.len(), unspecified) {
            (0, _) => code,
            (n, z) if n == z => format!("{code} with 0.0.0.0"),
            (n, _) => format!("{code} with {n} addresses, not only 0.0.0.0"),
        }
    }
}

/// The position after the name at `at` in `message`, following no pointer: a pointer ends the
/// name where it stands.
fn skip_name(message: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let len = *message.get(at)?;
        match len {
            0 => return Some(at + 1),
            l if l & 0xc0 == 0xc0 => return Some(at + 2),
            l if l & 0xc0 == 0 => at += 1 + usize::from(l),
            _ => return None,
        }
    }
}

/// The two bytes at `at` as a big-endian number.
fn u16_at(message: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *message.get(at)?,
        *message.get(at + 1)?,
    ]))
}

/// Reads `reply`: its response code and the A records of its answer section. `None` for bytes
/// that are not a whole response.
pub fn answer(reply: &[u8]) -> Option<Answer> {
    let head = header(reply)?;
    if !head.response {
        return None;
    }
    let questions = u16_at(reply, 4)?;
    let answers = u16_at(reply, 6)?;
    let mut at = HEADER_LEN;
    for _ in 0..questions {
        at = skip_name(reply, at)? + 4;
    }
    let mut v4 = Vec::new();
    let mut other_records = 0;
    for _ in 0..answers {
        at = skip_name(reply, at)?;
        let rtype = u16_at(reply, at)?;
        let rdlen = usize::from(u16_at(reply, at + 8)?);
        let data = reply.get(at + 10..at + 10 + rdlen)?;
        match (rtype, data) {
            (TYPE_A, &[a, b, c, d]) => v4.push(Ipv4Addr::new(a, b, c, d)),
            _ => other_records += 1,
        }
        at += 10 + rdlen;
    }
    Some(Answer {
        rcode: head.rcode,
        v4,
        other_records,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A reply to `query` with `code` and one A record per address, its name a pointer to the
    /// question, as servers write it.
    pub(crate) fn reply(query: &[u8], code: u8, addrs: &[Ipv4Addr]) -> Vec<u8> {
        let mut out = query.to_vec();
        out[2] |= 0x80;
        out[3] = (out[3] & 0xf0) | code;
        out[6..8].copy_from_slice(&(addrs.len() as u16).to_be_bytes());
        for addr in addrs {
            out.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
            out.extend_from_slice(&addr.octets());
        }
        out
    }

    #[test]
    fn a_query_is_the_header_and_one_question() {
        let mut out = Vec::new();
        encode_query(&mut out, 0xabcd, "example.com.", TYPE_A).unwrap();
        let mut want = vec![0xab, 0xcd, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 7];
        want.extend_from_slice(b"example");
        want.push(3);
        want.extend_from_slice(b"com");
        want.extend_from_slice(&[0, 0, 1, 0, 1]);
        assert_eq!(out, want);
        // Without the dot it is the same.
        let mut again = Vec::new();
        encode_query(&mut again, 0xabcd, "example.com", TYPE_A).unwrap();
        assert_eq!(again, want);
        set_id(&mut again, 7);
        assert_eq!(&again[..2], &[0, 7]);
    }

    #[test]
    fn a_name_that_cannot_be_asked_is_an_error() {
        let mut out = Vec::new();
        let long_label = format!("{}.example", "a".repeat(64));
        let long_name = vec!["a".repeat(60); 5].join(".");
        for (name, error) in [
            ("", NameError::EmptyLabel),
            ("a..example", NameError::EmptyLabel),
            (".", NameError::EmptyLabel),
            (long_label.as_str(), NameError::LabelTooLong),
            (long_name.as_str(), NameError::NameTooLong),
            ("a b.example", NameError::BadByte),
            ("é.example", NameError::BadByte),
        ] {
            assert_eq!(
                encode_query(&mut out, 1, name, TYPE_A),
                Err(error),
                "{name}"
            );
        }
    }

    #[test]
    fn the_header_gives_the_id_the_qr_bit_and_the_code() {
        let mut query = Vec::new();
        encode_query(&mut query, 513, "example.org", TYPE_A).unwrap();
        assert_eq!(
            header(&query),
            Some(Header {
                id: 513,
                response: false,
                rcode: 0
            })
        );
        let answered = reply(&query, RCODE_SERVFAIL, &[]);
        assert_eq!(
            header(&answered),
            Some(Header {
                id: 513,
                response: true,
                rcode: 2
            })
        );
        assert_eq!(header(&answered[..11]), None);
        assert!(is_failure(2) && is_failure(5) && !is_failure(3) && !is_failure(0));
    }

    #[test]
    fn an_answer_gives_its_addresses_and_whether_it_is_a_block() {
        let mut query = Vec::new();
        encode_query(&mut query, 9, "ads.example", TYPE_A).unwrap();
        let null = answer(&reply(&query, 0, &[Ipv4Addr::UNSPECIFIED])).unwrap();
        assert!(null.is_blocked());
        assert_eq!(null.describe(), "NOERROR with 0.0.0.0");
        let real = answer(&reply(&query, 0, &[Ipv4Addr::new(192, 0, 2, 1)])).unwrap();
        assert!(!real.is_blocked());
        assert_eq!(real.v4, [Ipv4Addr::new(192, 0, 2, 1)]);
        assert_eq!(
            real.describe(),
            "NOERROR with 1 addresses, not only 0.0.0.0"
        );
        let nx = answer(&reply(&query, RCODE_NXDOMAIN, &[])).unwrap();
        assert!(nx.is_blocked());
        let empty = answer(&reply(&query, 0, &[])).unwrap();
        assert!(!empty.is_blocked());
        assert_eq!(empty.describe(), "NOERROR");
        // A query is no answer, and nor are cut bytes.
        assert_eq!(answer(&query), None);
        let cut = reply(&query, 0, &[Ipv4Addr::UNSPECIFIED]);
        assert_eq!(answer(&cut[..cut.len() - 1]), None);
    }
}
