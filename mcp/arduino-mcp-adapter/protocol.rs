use anyhow::{anyhow, Result};
use tracing::debug;

pub struct ResponseDecoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ResponseDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn read_i16(&mut self) -> Result<i16> {
        if self.pos + 2 > self.data.len() {
            return Err(anyhow!("Not enough data for i16"));
        }
        let value = i16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(value)
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        if self.pos + 4 > self.data.len() {
            return Err(anyhow!("Not enough data for i32"));
        }
        let value = i32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(value)
    }

    pub fn read_cstring(&mut self) -> Result<String> {
        let remaining = &self.data[self.pos..];

        // Find null terminator or use all remaining data
        let end_pos = remaining
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(remaining.len());

        if end_pos == 0 && remaining.len() > 0 && remaining[0] == 0 {
            // Empty string with null terminator
            self.pos += 1;
            return Ok(String::new());
        }

        let str_bytes = &remaining[..end_pos];
        let result = String::from_utf8_lossy(str_bytes).to_string();

        // Skip past the string and null terminator if present
        self.pos += end_pos;
        if self.pos < self.data.len() && self.data[self.pos] == 0 {
            self.pos += 1; // Skip null terminator
        }

        debug!("Decoded C string: '{}'", result);
        Ok(result)
    }
}

pub struct CommandEncoder {
    data: Vec<u8>,
}

impl CommandEncoder {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn write_i16(&mut self, value: i16) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i32(&mut self, value: i32) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_cstring(&mut self, value: &str) {
        self.data.extend_from_slice(value.as_bytes());
        self.data.push(0); // Null terminator
    }

    pub fn finish(self) -> Vec<u8> {
        self.data
    }
}

pub fn decode_response_by_type(data: &[u8], return_type: &str) -> Result<String> {
    // Handle void functions (no data)
    if data.is_empty() {
        return Ok("Command executed successfully".to_string());
    }

    let mut decoder = ResponseDecoder::new(data);

    match return_type {
        "CStr" => decoder.read_cstring(),
        "i16" => {
            let value = decoder.read_i16()?;
            Ok(value.to_string())
        }
        "i32" => {
            let value = decoder.read_i32()?;
            Ok(value.to_string())
        }
        _ => decoder.read_cstring(), // Default to string
    }
}
