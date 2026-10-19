fn tamper_bytes(bytes: &mut [u8]) {
    // SenderKeyMessage: one version byte, protobuf, then a 64-byte signature.
    let payload = &bytes[1..bytes.len() - 64];
    let mut cursor = 0;
    while cursor < payload.len() {
        let tag = read_varint(payload, &mut cursor).expect("protobuf tag");
        match tag & 7 {
            0 => {
                read_varint(payload, &mut cursor).expect("protobuf integer");
            }
            2 => {
                let len = read_varint(payload, &mut cursor).expect("protobuf length") as usize;
                let end = cursor.checked_add(len).expect("field length");
                assert!(end <= payload.len(), "field within protobuf");
                if tag >> 3 == 4 {
                    assert!(len > 0, "ciphertext must not be empty");
                    bytes[1 + cursor + len / 2] ^= 1;
                    return;
                }
                cursor = end;
            }
            _ => panic!("unexpected SenderKeyMessage wire type"),
        }
    }
    panic!("SenderKeyMessage contains ciphertext");
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}
