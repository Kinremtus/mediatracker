//! Minimal vendored protobuf wire-format parser.
//!
//! Only the bits needed to read nested length-delimited messages out of an
//! upstream response: varint, length-delimited, fixed32 and fixed64. No
//! `prost`, no `protoc`, no external crate - the same idea as the vendored
//! MD5 helper in `chapter_count/mangaplus.rs`.

/// A single decoded wire value.
#[derive(Debug, PartialEq, Eq)]
pub enum WireValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
    Fixed64(u64),
}

/// Errors that can occur while decoding a wire buffer.
#[derive(Debug, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    BadWireType(u8),
}

/// Walk top-level fields. Unknown wire types -> `BadWireType`.
pub fn walk_fields<'a>(
    bytes: &'a [u8],
    f: &mut dyn FnMut(u32, WireValue<'a>),
) -> Result<(), WireError> {
    let mut pos = 0usize;
    while pos < bytes.len() {
        let tag = read_varint(bytes, &mut pos)?;
        let field_number = (tag >> 3) as u32;
        let wire_type = (tag & 0x7) as u8;
        match wire_type {
            // 0 = varint
            0 => {
                let value = read_varint(bytes, &mut pos)?;
                f(field_number, WireValue::Varint(value));
            }
            // 1 = 64-bit fixed
            1 => {
                let end = pos.checked_add(8).ok_or(WireError::Truncated)?;
                let slice = bytes.get(pos..end).ok_or(WireError::Truncated)?;
                let mut raw = [0u8; 8];
                raw.copy_from_slice(slice);
                pos = end;
                f(field_number, WireValue::Fixed64(u64::from_le_bytes(raw)));
            }
            // 2 = length-delimited
            2 => {
                let len = read_varint(bytes, &mut pos)? as usize;
                let end = pos.checked_add(len).ok_or(WireError::Truncated)?;
                let payload = bytes.get(pos..end).ok_or(WireError::Truncated)?;
                pos = end;
                f(field_number, WireValue::Bytes(payload));
            }
            // 5 = 32-bit fixed
            5 => {
                let end = pos.checked_add(4).ok_or(WireError::Truncated)?;
                let slice = bytes.get(pos..end).ok_or(WireError::Truncated)?;
                let mut raw = [0u8; 4];
                raw.copy_from_slice(slice);
                pos = end;
                f(field_number, WireValue::Fixed32(u32::from_le_bytes(raw)));
            }
            other => return Err(WireError::BadWireType(other)),
        }
    }
    Ok(())
}

/// All top-level values for a specific field number (usually 0 or 1).
///
/// Walk errors are ignored; whatever was collected before the error is returned.
pub fn field_values<'a>(bytes: &'a [u8], target: u32) -> Vec<WireValue<'a>> {
    let mut out = Vec::new();
    let _ = walk_fields(bytes, &mut |number, value| {
        if number == target {
            out.push(value);
        }
    });
    out
}

/// Decode the first length-delimited sub-message for field `target`.
pub fn message_field(bytes: &[u8], target: u32) -> Option<&[u8]> {
    let mut result = None;
    let _ = walk_fields(bytes, &mut |number, value| {
        if result.is_none() && number == target
            && let WireValue::Bytes(payload) = value
        {
            result = Some(payload);
        }
    });
    result
}

/// Read a base-128 varint, advancing `pos` past it.
///
/// A varint is at most 10 bytes; anything longer is treated as truncated.
pub fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, WireError> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    for _ in 0..10 {
        let byte = *bytes.get(*pos).ok_or(WireError::Truncated)?;
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
    Err(WireError::Truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode an unsigned integer as a base-128 varint.
    fn enc_varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                out.push(byte | 0x80);
            } else {
                out.push(byte);
                break;
            }
        }
        out
    }

    fn decode(bytes: &[u8]) -> u64 {
        let mut pos = 0usize;
        read_varint(bytes, &mut pos).unwrap()
    }

    #[test]
    fn varint_roundtrip() {
        for value in [1u64, 127, 128, 300, 1u64 << 31] {
            let encoded = enc_varint(value);
            assert_eq!(decode(&encoded), value, "roundtrip failed for {value}");
        }
    }

    #[test]
    fn varint_known_bytes() {
        assert_eq!(decode(&[0x01]), 1);
        assert_eq!(decode(&[0x7f]), 127);
        assert_eq!(decode(&[0x80, 0x01]), 128);
        assert_eq!(decode(&[0xac, 0x02]), 300);
        assert_eq!(decode(&[0x80, 0x80, 0x80, 0x80, 0x08]), 1 << 31);
    }

    #[test]
    fn register_shaped_nesting() {
        let secret: [u8; 32] = *b"0123456789abcdef0123456789abcdef";

        // Outermost message: field 1 (bytes) wraps the whole thing.
        // 0x0A = field 1, wire type 2. 0x24 = length 36.
        // Inner: 0x12 = field 2, wire type 2, 0x22 = length 34.
        // Innermost: 0x0A = field 1, wire type 2, 0x20 = length 32.
        let mut top = vec![0x0A, 0x24, 0x12, 0x22, 0x0A, 0x20];
        top.extend_from_slice(&secret);

        let outer = message_field(&top, 1).unwrap();
        let inner = message_field(outer, 2).unwrap();
        let decoded = message_field(inner, 1).unwrap();
        assert_eq!(decoded, &secret);
    }

    #[test]
    fn walk_fields_truncated() {
        // 0x0A = field 1, wire type 2, but the length byte is missing.
        assert_eq!(
            walk_fields(&[0x0A], &mut |_, _| {}),
            Err(WireError::Truncated)
        );

        // Length claims 5 bytes but none follow.
        assert_eq!(
            walk_fields(&[0x0A, 0x05], &mut |_, _| {}),
            Err(WireError::Truncated)
        );
    }

    #[test]
    fn unknown_wire_type() {
        // 0x0B = field 1, wire type 3.
        assert_eq!(
            walk_fields(&[0x0B], &mut |_, _| {}),
            Err(WireError::BadWireType(3))
        );
    }

    #[test]
    fn repeated_varint_field_values() {
        // 0x10 = field 2, wire type 0. Two values: 1 and 2.
        let bytes = [0x10, 0x01, 0x10, 0x02];
        assert_eq!(
            field_values(&bytes, 2),
            vec![WireValue::Varint(1), WireValue::Varint(2)]
        );
    }
}
