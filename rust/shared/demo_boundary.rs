use std::io::{self, Read, Seek};

pub(crate) fn end_tick(mut source: impl Read + Seek, length: u64) -> io::Result<i64> {
    let mut header = [0; 16];
    source.read_exact(&mut header)?;
    if &header[..8] != b"PBDEMS2\0" {
        return Err(invalid_frame());
    }
    let mut position = 16;
    let mut end = 0;
    while position < length {
        varint(&mut source, &mut position)?;
        let tick =
            u32::try_from(varint(&mut source, &mut position)?).map_err(|_| invalid_frame())?;
        let payload = varint(&mut source, &mut position)?;
        if payload > 256 * 1024 * 1024 || position > length || payload > length - position {
            return Err(invalid_frame());
        }
        source.seek_relative(payload as i64)?;
        position += payload;
        if tick != u32::MAX {
            end = end.max(i64::from(tick));
        }
    }
    Ok(end)
}

fn varint(source: &mut impl Read, position: &mut u64) -> io::Result<u64> {
    let mut value = 0;
    let mut byte = [0];
    for shift in (0..=63).step_by(7) {
        source.read_exact(&mut byte)?;
        *position += 1;
        let [byte] = byte;
        if shift == 63 && byte > 1 {
            return Err(invalid_frame());
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(value);
        }
    }
    Err(invalid_frame())
}

fn invalid_frame() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid PBDEMS2 frame boundary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn scans_past_payloads_and_ignores_the_stop_tick() {
        let mut bytes = b"PBDEMS2\0\0\0\0\0\0\0\0\0".to_vec();
        bytes.extend([1, 130, 1, 4, 255, 0, 99, 228]);
        bytes.extend([0, 255, 255, 255, 255, 15, 0]);
        let read = |bytes: &[u8]| end_tick(Cursor::new(bytes), bytes.len() as u64);
        assert_eq!(read(&bytes).unwrap(), 130);
        bytes.extend([1, 131, 1, 6, 33]);
        assert!(read(&bytes).is_err());
    }
}
