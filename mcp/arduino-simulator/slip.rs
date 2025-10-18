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
        match self.state {
            SlipDecodeState::Idle => {
                if byte == SLIP_END {
                    debug!("SLIP: Frame start");
                    self.state = SlipDecodeState::Receiving;
                    self.buffer.clear();
                } else if byte == SLIP_ESC {
                    debug!("SLIP: Escape in idle");
                    self.state = SlipDecodeState::Escaped;
                }
                Ok(None)
            }
            SlipDecodeState::Receiving => {
                if byte == SLIP_END {
                    // End of frame
                    if !self.buffer.is_empty() {
                        let frame = self.buffer.clone();
                        debug!("SLIP: Frame complete ({} bytes)", frame.len());
                        self.reset();
                        Ok(Some(frame))
                    } else {
                        debug!("SLIP: Empty frame");
                        Ok(None)
                    }
                } else if byte == SLIP_ESC {
                    debug!("SLIP: Escape character");
                    self.state = SlipDecodeState::Escaped;
                    Ok(None)
                } else {
                    // Regular data byte
                    if self.buffer.len() < 1024 {
                        self.buffer.push(byte);
                    } else {
                        warn!("SLIP: Frame too large, resetting");
                        self.reset();
                        return Err(anyhow!("SLIP frame too large"));
                    }
                    Ok(None)
                }
            }
            SlipDecodeState::Escaped => {
                match byte {
                    SLIP_CLEAR => {
                        debug!("SLIP: Clear sequence");
                        self.reset();
                    }
                    SLIP_ESC_END => {
                        debug!("SLIP: Escaped END");
                        if self.buffer.len() < 1024 {
                            self.buffer.push(SLIP_END);
                        } else {
                            warn!("SLIP: Frame too large during escape");
                            self.reset();
                            return Err(anyhow!("SLIP frame too large"));
                        }
                    }
                    SLIP_ESC_ESC => {
                        debug!("SLIP: Escaped ESC");
                        if self.buffer.len() < 1024 {
                            self.buffer.push(SLIP_ESC);
                        } else {
                            warn!("SLIP: Frame too large during escape");
                            self.reset();
                            return Err(anyhow!("SLIP frame too large"));
                        }
                    }
                    _ => {
                        warn!("SLIP: Invalid escape sequence: 0x{:02X}", byte);
                        self.reset();
                        return Err(anyhow!("Invalid SLIP escape sequence: 0x{:02X}", byte));
                    }
                }
                self.state = SlipDecodeState::Receiving;
                Ok(None)
            }
        }
    }
}

/// Encode data into SLIP format
pub fn slip_encode(data: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(data.len() + 10);

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
