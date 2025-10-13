#include "../mcp/mcp.hpp"

// Define global MCP handler instance
MCPHandler mcp_handler;

MCP_DESCRIPTION(
    "LED blinker with remote control functions"
)

const int greenPin = 14;
const int yellowPin = 15;
const int redPin = 16;

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

MCP_TOOL("  ")
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
