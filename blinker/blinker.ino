#include "../mcp/mcp.hpp"

// Define global MCP handler instance
MCPHandler mcp_handler;

MCP_DESCRIPTION(
    "LED blinker with remote control functions"
)

const int greenPin = 5;
const int yellowPin = 4;
const int redPin = 3;
int currentLed = -1; // Track which LED is currently active

void setLedState(int pin, int level) {
    digitalWrite(greenPin, pin == greenPin ? level : LOW);
    digitalWrite(yellowPin, pin == yellowPin ? level : LOW);
    digitalWrite(redPin, pin == redPin ? level : LOW);
    currentLed = (level == HIGH) ? pin : -1;
}

MCP_TOOL("Switch LED to red color")
void switchToRed() 
{
    setLedState(redPin, HIGH);
}

MCP_TOOL("Switch LED to yellow color")
void switchToYellow() {
    setLedState(yellowPin, HIGH);
}

MCP_TOOL("Switch LED to green color")
void switchToGreen() {
    setLedState(greenPin, HIGH);
}

MCP_TOOL("Blink last active LED n times")
void blinkNTimes(int n) {
    int originalLed = currentLed;
    for (int i = 0; i < n; i++) {
        setLedState(-1, LOW); // Turn off all LEDs
        delay(200);
        if (originalLed != -1) {
            setLedState(originalLed, HIGH); // Turn on original LED
        } else {
            setLedState(greenPin, HIGH); // Default to green if no LED was on
        }
        delay(200);
    }
}

MCP_TOOL("Get currently active LED pin number")
int getActiveLedPin() {
    return currentLed;
}

MCP_TOOL("Get currently active LED color")
const char* getActiveLedColor() {
    if (currentLed == redPin) {
        return "red";
    } else if (currentLed == yellowPin) {
        return "yellow";
    } else if (currentLed == greenPin) {
        return "green";
    } else {
        return "off";
    }
}

MCP_TOOL("Print message N times to serial port")
void printMessage(const char* msg, int times) {
    for (int i = 0; i < times; i++) {
        Serial.print("Message ");
        Serial.print(i + 1);
        Serial.print(": ");
        Serial.println(msg);
        delay(100);
    }
}


// Test function with unsupported type - should cause error
// MCP_TOOL("Test unsupported type")
// void testUnsupported(uint64_t bignum) {
//     Serial.println(bignum);
// }

#include "build/mcp_bindings.hpp"

// Implementation of MCPHandler::process_frame (after bindings are available)
void MCPHandler::process_frame() {
    // Frame format: [data...][crc8]
    if (frame_pos < 2) return; // Need at least data + CRC
    
    int data_len = frame_pos - 1;
    uint8_t received_crc = frame_buffer[frame_pos - 1];
    uint8_t calculated_crc = crc8(frame_buffer, data_len);
    
    if (received_crc != calculated_crc) {
        // CRC mismatch - send error response
        uint8_t error_response[2] = {0xFF, 0x01}; // Error code 1: CRC error
        uint8_t error_crc = crc8(error_response, 1);
        error_response[1] = error_crc;
        send_slip_frame(error_response, 2);
        return;
    }
    
    // CRC valid - dispatch the command
    int response_len;
    int result = MCPBindings::dispatch(frame_buffer, data_len, response_buffer, MAX_FRAME_SIZE - 1, &response_len);
    
    if (result == 0) {
        // Success - send response with CRC
        uint8_t response_crc = crc8(response_buffer, response_len);
        response_buffer[response_len] = response_crc;
        send_slip_frame(response_buffer, response_len + 1);
    } else {
        // Error - send error response
        uint8_t error_response[2] = {0xFF, 0x02}; // Error code 2: Dispatch error
        uint8_t error_crc = crc8(error_response, 1);
        error_response[1] = error_crc;
        send_slip_frame(error_response, 2);
    }
}

void setup() {
    Serial.begin(115200);
    pinMode(greenPin, OUTPUT);
    pinMode(yellowPin, OUTPUT);
    pinMode(redPin, OUTPUT);
    setLedState(-1, LOW);
    Serial.write('>');
}

void loop() {
    // Process incoming MCP commands via SLIP protocol
    mcp_handler.process_serial();
    
    // Optional: Add a small delay to prevent overwhelming the serial buffer
    delay(1);
}
