#ifndef MCP_HPP
#define MCP_HPP

#include <stdint.h>

#define MCP_TOOL(documentation)
#define MCP_DESCRIPTION(desc) struct __mcp_desc_sentinel { } __attribute__((annotate("MCP_DESCRIPTION:" desc)));

// SLIP protocol constants
#define SLIP_END     0xC0    // Frame marker
#define SLIP_ESC     0xDB    // Escape character
#define SLIP_ESC_END 0xDC    // Escaped END
#define SLIP_ESC_ESC 0xDD    // Escaped ESC
#define SLIP_CLEAR   0xDE    // Clear sequence

// MCP protocol state machine
enum MCPState {
    MCP_IDLE,
    MCP_RECEIVING,
    MCP_ESCAPED
};

class MCPHandler {
private:
    static const int MAX_FRAME_SIZE = 256;
    uint8_t frame_buffer[MAX_FRAME_SIZE];
    int frame_pos;
    MCPState state;
    uint8_t response_buffer[MAX_FRAME_SIZE];
    
    // Simple CRC-8 implementation
    uint8_t crc8(const uint8_t* data, int len) {
        uint8_t crc = 0x00;
        for (int i = 0; i < len; i++) {
            crc ^= data[i];
            for (int j = 0; j < 8; j++) {
                if (crc & 0x80) {
                    crc = (crc << 1) ^ 0x07; // CRC-8-CCITT polynomial
                } else {
                    crc <<= 1;
                }
            }
        }
        return crc;
    }
    
    void reset_frame() {
        frame_pos = 0;
        state = MCP_IDLE;
    }
    
    void send_slip_frame(const uint8_t* data, int len) {
        // Clear any garbage with ESC CLEAR sequence
        Serial.write(SLIP_ESC);
        Serial.write(SLIP_CLEAR);
        
        // Send frame start marker
        Serial.write(SLIP_END);
        
        // Send data with escaping
        for (int i = 0; i < len; i++) {
            if (data[i] == SLIP_END) {
                Serial.write(SLIP_ESC);
                Serial.write(SLIP_ESC_END);
            } else if (data[i] == SLIP_ESC) {
                Serial.write(SLIP_ESC);
                Serial.write(SLIP_ESC_ESC);
            } else {
                Serial.write(data[i]);
            }
        }
        
        // Send frame end marker
        Serial.write(SLIP_END);
    }
    
public:
    MCPHandler() : frame_pos(0), state(MCP_IDLE) {}
    
    void process_serial() {
        while (Serial.available() > 0) {
            uint8_t byte = Serial.read();
            Serial.write('R'); // Debug: byte received
            
            switch (state) {
                case MCP_IDLE:
                {
                    if (byte == SLIP_END) {
                        Serial.write('S'); // Debug: frame start
                        state = MCP_RECEIVING;
                        frame_pos = 0;
                    }
                    // Ignore other bytes when idle
                }
                break;
                
                case MCP_RECEIVING:
                {
                    if (byte == SLIP_END) {
                        Serial.write('E'); // Debug: frame end
                        // End of frame - process if we have data
                        if (frame_pos > 1) { // At least 1 byte data + 1 byte CRC
                            Serial.write('P'); // Debug: processing frame
                            process_frame();
                        }
                        reset_frame();
                    } else if (byte == SLIP_ESC) {
                        Serial.write('\\'); // Debug: escape character
                        state = MCP_ESCAPED;
                    } else {
                        // Regular data byte
                        if (frame_pos < MAX_FRAME_SIZE) {
                            Serial.write('D'); // Debug: data byte added
                            frame_buffer[frame_pos++] = byte;
                        } else {
                            // Frame too large - reset
                            Serial.write('X'); // Debug: frame too large
                            reset_frame();
                        }
                    }
                }
                break;
                
                case MCP_ESCAPED:
                {
                    if (byte == SLIP_ESC_END) {
                        Serial.write('e'); // Debug: escaped END
                        if (frame_pos < MAX_FRAME_SIZE) {
                            frame_buffer[frame_pos++] = SLIP_END;
                        } else {
                            Serial.write('X'); // Debug: frame too large
                            reset_frame();
                        }
                    } else if (byte == SLIP_ESC_ESC) {
                        Serial.write('s'); // Debug: escaped ESC
                        if (frame_pos < MAX_FRAME_SIZE) {
                            frame_buffer[frame_pos++] = SLIP_ESC;
                        } else {
                            Serial.write('X'); // Debug: frame too large
                            reset_frame();
                        }
                    } else {
                        // Invalid escape sequence - reset frame
                        Serial.write('!'); // Debug: invalid escape
                        reset_frame();
                    }
                    state = MCP_RECEIVING;
                }
                break;
            }
        }
    }
    
private:
    void process_frame(); // Implementation moved to after bindings include
};

// Global MCP handler instance
extern MCPHandler mcp_handler;

#endif // MCP_HPP