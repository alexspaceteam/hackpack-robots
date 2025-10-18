use anyhow::{anyhow, Result};
use tracing::debug;

/// CRC-8-CCITT algorithm
/// Polynomial: 0x07 (x^8 + x^2 + x + 1)
/// Initial value: 0x00
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0x00;

    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc = crc << 1;
            }
        }
    }

    crc
}

/// Response data types
pub enum ResponseData {
    Void,
    I16(i16),
    I32(i32),
    CStr(String),
}

/// Decode a command frame: [tag] [args...] [crc]
/// Returns (tag, args_without_crc)
pub fn decode_command(frame: &[u8]) -> Result<(u8, &[u8])> {
    if frame.is_empty() {
        return Err(anyhow!("Empty command frame"));
    }

    if frame.len() < 2 {
        return Err(anyhow!("Command frame too short (need at least tag + CRC)"));
    }

    // Split into data and CRC
    let (data, crc_bytes) = frame.split_at(frame.len() - 1);
    let received_crc = crc_bytes[0];

    // Validate CRC
    let calculated_crc = crc8(data);
    if calculated_crc != received_crc {
        debug!(
            "CRC mismatch: calculated=0x{:02X}, received=0x{:02X}",
            calculated_crc, received_crc
        );
        return Err(anyhow!("CRC mismatch"));
    }

    debug!("CRC valid: 0x{:02X}", received_crc);

    // Extract tag and arguments
    let tag = data[0];
    let args = if data.len() > 1 { &data[1..] } else { &[] };

    Ok((tag, args))
}

/// Encode a response frame: [data...] [crc]
pub fn encode_response(response_data: &ResponseData) -> Result<Vec<u8>> {
    let mut frame = Vec::new();

    match response_data {
        ResponseData::Void => {
            // Empty response, just CRC
        }
        ResponseData::I16(value) => {
            frame.extend_from_slice(&value.to_le_bytes());
        }
        ResponseData::I32(value) => {
            frame.extend_from_slice(&value.to_le_bytes());
        }
        ResponseData::CStr(s) => {
            frame.extend_from_slice(s.as_bytes());
            frame.push(0); // Null terminator
        }
    }

    // Calculate and append CRC
    let crc = crc8(&frame);
    frame.push(crc);

    debug!(
        "Response encoded: {} bytes (CRC: 0x{:02X})",
        frame.len(),
        crc
    );

    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc8() {
        // Test with known values
        let data = vec![0x01, 0x02, 0x03];
        let crc = crc8(&data);
        // CRC-8-CCITT with polynomial 0x07 and init 0x00
        // Verified output for input [0x01, 0x02, 0x03]
        assert_eq!(crc, 0x48);
    }

    #[test]
    fn test_encode_void() {
        let response = encode_response(&ResponseData::Void).unwrap();
        assert_eq!(response.len(), 1); // Just CRC
    }

    #[test]
    fn test_encode_i16() {
        let response = encode_response(&ResponseData::I16(42)).unwrap();
        assert_eq!(response.len(), 3); // 2 bytes + CRC
        assert_eq!(response[0], 42); // Little-endian low byte
        assert_eq!(response[1], 0); // Little-endian high byte
    }

    #[test]
    fn test_encode_i32() {
        let response = encode_response(&ResponseData::I32(1000)).unwrap();
        assert_eq!(response.len(), 5); // 4 bytes + CRC
        assert_eq!(response[0], 0xE8); // Little-endian: 1000 = 0x03E8
        assert_eq!(response[1], 0x03);
        assert_eq!(response[2], 0x00);
        assert_eq!(response[3], 0x00);
    }

    #[test]
    fn test_encode_cstr() {
        let response = encode_response(&ResponseData::CStr("hello".to_string())).unwrap();
        assert_eq!(response.len(), 7); // "hello" + null + CRC
        assert_eq!(&response[0..5], b"hello");
        assert_eq!(response[5], 0); // Null terminator
    }

    #[test]
    fn test_decode_command() {
        // Command with tag 5, no args
        let crc = crc8(&[5]);
        let frame = vec![5, crc];
        let (tag, args) = decode_command(&frame).unwrap();
        assert_eq!(tag, 5);
        assert_eq!(args.len(), 0);
    }

    #[test]
    fn test_decode_command_with_args() {
        // Command with tag 1, i16 arg = 100
        let data = vec![1, 100, 0]; // tag + little-endian i16
        let crc = crc8(&data);
        let mut frame = data;
        frame.push(crc);

        let (tag, args) = decode_command(&frame).unwrap();
        assert_eq!(tag, 1);
        assert_eq!(args, &[100, 0]);
    }

    #[test]
    fn test_decode_command_bad_crc() {
        let frame = vec![5, 0xFF]; // Wrong CRC
        let result = decode_command(&frame);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("CRC"));
    }
}
