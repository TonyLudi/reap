use std::fmt;

use crate::PmWireError;

pub(crate) const MAX_EXACT_TEXT_BYTES: usize = 96;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ExactText {
    length: u8,
    bytes: [u8; MAX_EXACT_TEXT_BYTES],
}

impl ExactText {
    pub(crate) fn new(field: &'static str, value: &str) -> Result<Self, PmWireError> {
        if value.is_empty() {
            return Err(PmWireError::MissingField(field));
        }
        if value.len() > MAX_EXACT_TEXT_BYTES {
            return Err(PmWireError::FieldTooLong(field));
        }
        if !value.is_ascii() {
            return Err(PmWireError::NonAsciiField(field));
        }
        let mut bytes = [0_u8; MAX_EXACT_TEXT_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            length: value.len() as u8,
            bytes,
        })
    }

    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)])
            .expect("checked ASCII exact text")
    }
}

impl fmt::Debug for ExactText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ExactText")
            .field(&self.as_str())
            .finish()
    }
}

/// The exact 20-byte SHA-1 supplied by a Polymarket book snapshot.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotHash([u8; 20]);

impl SnapshotHash {
    pub fn parse_hex(input: &str) -> Result<Self, PmWireError> {
        if input.len() != 40
            || input
                .bytes()
                .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
        {
            return Err(PmWireError::NonCanonicalSnapshotHash);
        }
        let mut bytes = [0_u8; 20];
        for (index, output) in bytes.iter_mut().enumerate() {
            let high = hex_nibble(input.as_bytes()[index * 2]);
            let low = hex_nibble(input.as_bytes()[index * 2 + 1]);
            *output = (high << 4) | low;
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 20] {
        self.0
    }
}

impl fmt::Display for SnapshotHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

const fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}
