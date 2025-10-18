use anyhow::{anyhow, Result};
use tracing::{debug, warn};

// SLIP protocol constants
const SLIP_END: u8 = 0xC0;
const SLIP_ESC: u8 = 0xDB;
const SLIP_ESC_END: u8 = 0xDC;
const SLIP_ESC_ESC: u8 = 0xDD;
const SLIP_CLEAR: u8 = 0xDE;

#[derive(Debug, Clone, PartialEq)]
pub enum SlipDecodeState {
    Idle,
    Receiving,
    Escaped,
}

pub struct SlipDecoder {
    state: SlipDecodeState,
    buffer: Vec<u8>,
}

impl SlipDecoder {
    pub fn new() -> Self {
        Self {
            state: SlipDecodeState::Idle,
            buffer: Vec::with_capacity(256),
        }
    }

    pub fn reset(&mut self) {
        self.state = SlipDecodeState::Idle;
        self.buffer.clear();
    }
    

    /// Process a single byte, returning Some(frame) when a complete frame is decoded
    pub fn process_byte(&mut self, byte: u8) -> Result<Option<Vec<u8>>> {
        let char_display = if byte >= 32 && byte <= 126 {
            format!("'{}'", byte as char)
        } else {
            format!("0x{:02X}", byte)
        };
        debug!("SLIP State: {:?}, Byte: {} ({})", self.state, byte, char_display);
        match self.state {
            SlipDecodeState::Idle => {
                if byte == SLIP_END {
                    debug!("SLIP Frame start detected, switching to Receiving");
                    self.state = SlipDecodeState::Receiving;
                    self.buffer.clear();
                } else if byte == SLIP_ESC {
                    // ESC in idle state - could be clear sequence
                    debug!("SLIP Escape in idle, waiting for next byte");
                    self.state = SlipDecodeState::Escaped;
                }
                // Ignore other bytes when idle
                Ok(None)
            }
            SlipDecodeState::Receiving => {
                if byte == SLIP_END {
                    // End of frame
                    if !self.buffer.is_empty() {
                        let frame = self.buffer.clone();
                        debug!("SLIP Frame complete, {} bytes received", frame.len());
                        self.reset();
                        Ok(Some(frame))
                    } else {
                        debug!("SLIP Empty frame, ignoring");
                        // Empty frame, continue receiving
                        Ok(None)
                    }
                } else if byte == SLIP_ESC {
                    debug!("SLIP Escape character detected, switching to Escaped state");
                    self.state = SlipDecodeState::Escaped;
                    Ok(None)
                } else {
                    // Regular data byte
                    if self.buffer.len() < 1024 {
                        // Prevent excessive memory usage
                        self.buffer.push(byte);
                        debug!("SLIP Added data byte to buffer (buffer len: {})", self.buffer.len());
                    } else {
                        warn!("SLIP Frame too large, resetting");
                        self.reset();
                        return Err(anyhow!("SLIP frame too large"));
                    }
                    Ok(None)
                }
            }
            SlipDecodeState::Escaped => {
                match byte {
                    SLIP_CLEAR => {
                        debug!("SLIP Clear sequence detected (ESC+CLEAR), resetting decoder");
                        self.reset();
                    }
                    SLIP_ESC_END => {
                        debug!("SLIP Escaped END byte, adding 0xC0 to buffer");
                        if self.buffer.len() < 1024 {
                            self.buffer.push(SLIP_END);
                        } else {
                            warn!("SLIP Frame too large during escape, resetting");
                            self.reset();
                            return Err(anyhow!("SLIP frame too large"));
                        }
                    }
                    SLIP_ESC_ESC => {
                        debug!("SLIP Escaped ESC byte, adding 0xDB to buffer");
                        if self.buffer.len() < 1024 {
                            self.buffer.push(SLIP_ESC);
                        } else {
                            warn!("SLIP Frame too large during escape, resetting");
                            self.reset();
                            return Err(anyhow!("SLIP frame too large"));
                        }
                    }
                    _ => {
                        warn!("SLIP Invalid escape sequence: 0x{:02X}, resetting", byte);
                        // Invalid escape sequence
                        self.reset();
                        return Err(anyhow!("Invalid SLIP escape sequence: 0x{:02X}", byte));
                    }
                }
                if self.state != SlipDecodeState::Receiving {
                    self.state = SlipDecodeState::Receiving;
                }
                Ok(None)
            }
        }
    }

}

/// Encode data into SLIP format
pub fn slip_encode(data: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(data.len() + 10); // Extra space for escaping

    // Start frame marker
    encoded.push(SLIP_END);

    // Encode data with escaping
    for &byte in data {
        match byte {
            SLIP_END => {
                encoded.push(SLIP_ESC);
                encoded.push(SLIP_ESC_END);
            }
            SLIP_ESC => {
                encoded.push(SLIP_ESC);
                encoded.push(SLIP_ESC_ESC);
            }
            _ => {
                encoded.push(byte);
            }
        }
    }

    // End frame marker
    encoded.push(SLIP_END);

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slip_encode_simple() {
        let data = vec![0x01, 0x02, 0x03];
        let encoded = slip_encode(&data);
        assert_eq!(encoded, vec![SLIP_END, 0x01, 0x02, 0x03, SLIP_END]);
    }

    #[test]
    fn test_slip_encode_with_escaping() {
        let data = vec![0x01, SLIP_END, 0x03, SLIP_ESC, 0x05];
        let encoded = slip_encode(&data);
        let expected = vec![
            SLIP_END,
            0x01,
            SLIP_ESC,
            SLIP_ESC_END,
            0x03,
            SLIP_ESC,
            SLIP_ESC_ESC,
            0x05,
            SLIP_END,
        ];
        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_slip_decode_simple() {
        let mut decoder = SlipDecoder::new();
        let input = vec![SLIP_END, 0x01, 0x02, 0x03, SLIP_END];

        let mut frames = Vec::new();
        for &byte in &input {
            if let Some(frame) = decoder.process_byte(byte).unwrap() {
                frames.push(frame);
            }
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_slip_decode_with_escaping() {
        let mut decoder = SlipDecoder::new();
        let input = vec![
            SLIP_END,
            0x01,
            SLIP_ESC,
            SLIP_ESC_END,
            0x03,
            SLIP_ESC,
            SLIP_ESC_ESC,
            0x05,
            SLIP_END,
        ];

        let mut frames = Vec::new();
        for &byte in &input {
            if let Some(frame) = decoder.process_byte(byte).unwrap() {
                frames.push(frame);
            }
        }
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], vec![0x01, SLIP_END, 0x03, SLIP_ESC, 0x05]);
    }

    #[test]
    fn test_slip_roundtrip() {
        let original = vec![0x01, SLIP_END, 0x03, SLIP_ESC, 0x05, 0x42];
        let encoded = slip_encode(&original);

        let mut decoder = SlipDecoder::new();
        let mut frames = Vec::new();
        for &byte in &encoded {
            if let Some(frame) = decoder.process_byte(byte).unwrap() {
                frames.push(frame);
            }
        }

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], original);
    }
}