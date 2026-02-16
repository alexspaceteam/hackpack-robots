#include <Arduino.h>
#include <Servo.h>
#include <IRremote.hpp>

#define DECODE_NEC  // Defines the type of IR transmission to decode based on the remote

const int IR_RECEIVE_PIN = 9;  // Same pin as ir-turret

//////////////////////////////////////////////////
               //  SERVOS  //
//////////////////////////////////////////////////

Servo yawServo;   // YAW rotation servo
Servo pitchServo; // PITCH rotation servo
Servo rollServo;  // ROLL rotation servo

//////////////////////////////////////////////////  
                //  S E T U P  //
//////////////////////////////////////////////////

void setup() {
    Serial.begin(115200);

    // Wait for serial port to connect
    delay(1000);

    Serial.println(F("==============================="));
    Serial.println(F("IR DEBUG PROGRAM"));
    Serial.println(F("==============================="));
    Serial.println(F("START " __FILE__ " from " __DATE__));
    Serial.println(F("Using library version " VERSION_IRREMOTE));
    Serial.println();

    // Attach servos to test PWM interference with IR
    //yawServo.attach(10);   // attach YAW servo to pin 10
    pitchServo.attach(2); // attach PITCH servo to pin 11
    delay(250);
    pitchServo.write(70);
    delay(250);
    pitchServo.write(110);
    delay(250);
    pitchServo.write(90);
   

    //rollServo.attach(12);  // attach ROLL servo to pin 12
    Serial.println(F("Servos attached to pins 10, 11, 12"));
    Serial.println();

    // Start the IR receiver
    IrReceiver.begin(IR_RECEIVE_PIN, ENABLE_LED_FEEDBACK);

    Serial.print(F("Ready to receive IR signals of protocols: "));
    printActiveIRProtocols(&Serial);
    Serial.print(F("at pin "));
    Serial.println(IR_RECEIVE_PIN);
    Serial.println();
    Serial.println(F("Waiting for IR signals..."));
    Serial.println(F("==============================="));
    Serial.println();
}

//////////////////////////////////////////////////
                //  L O O P  //
//////////////////////////////////////////////////

void loop() {
    if (IrReceiver.decode()) {
        // Print a separator for readability
        Serial.println(F("--- IR Signal Received ---"));

        // Print the raw received data
        Serial.print(F("Protocol: "));
        Serial.println(getProtocolString(IrReceiver.decodedIRData.protocol));

        Serial.print(F("Address: 0x"));
        Serial.println(IrReceiver.decodedIRData.address, HEX);

        Serial.print(F("Command: 0x"));
        Serial.println(IrReceiver.decodedIRData.command, HEX);

        Serial.print(F("Raw Data: 0x"));
        Serial.println(IrReceiver.decodedIRData.decodedRawData, HEX);

        // Check if this is a repeat signal
        if (IrReceiver.decodedIRData.flags & IRDATA_FLAGS_IS_REPEAT) {
            Serial.println(F("Flags: REPEAT"));
        } else {
            Serial.println(F("Flags: NEW"));
        }

        // Print the complete data in one line for easy copy-paste
        Serial.print(F("Summary: "));
        Serial.print(getProtocolString(IrReceiver.decodedIRData.protocol));
        Serial.print(F(" | Addr:0x"));
        Serial.print(IrReceiver.decodedIRData.address, HEX);
        Serial.print(F(" | Cmd:0x"));
        Serial.print(IrReceiver.decodedIRData.command, HEX);
        Serial.print(F(" | Raw:0x"));
        Serial.println(IrReceiver.decodedIRData.decodedRawData, HEX);

        Serial.println();

        switch (IrReceiver.decodedIRData.command) {
            case 0x18: //up
                pitchServo.write(80);
                break;
            case 0x1c: //ok
                pitchServo.write(90);
                break;
            case 0x52: //down
                pitchServo.write(100);
                break;
        }

        // Enable receiving of the next IR signal
        IrReceiver.resume();
    }

    delay(5);  // Small delay for smoothness
}
