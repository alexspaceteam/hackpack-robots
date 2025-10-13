#ifndef MCP_PROCESS_FRAME_HPP
#define MCP_PROCESS_FRAME_HPP

#include "mcp.hpp"

// Implementation of MCPHandler::process_frame
// This must be included AFTER the project-specific mcp_bindings.hpp
inline void MCPHandler::process_frame() {
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

#endif // MCP_PROCESS_FRAME_HPP
