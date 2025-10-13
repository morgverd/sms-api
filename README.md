# SMS Server

A powerful, self-hosted SMS gateway that enables you to send and receive text messages using your own cellular hardware. Built with security and flexibility in mind.

### **Built-in Security**
- Encryption by default for all message storage within database
- Optional token-based authentication for HTTP API and WebSocket connections
- TLS/HTTPS support for secure communications

### **Multiple Integration Options**
- **HTTP REST API** for sending and reading SMS messages, modem requests and device info
- **Real-time WebSocket** to hold a persistent low-overhead connection for receiving events
- **HTTP webhooks** to receive events with a HTTP server, sending POST requests to provided URLs

### **Advanced SMS Features**
- Automatic handling of multipart SMS messages
- SMS delivery report tracking with status updates
- Support for both incoming and outgoing message management
- International phone number format handling

### **Location Services**
- Built-in GNSS/GPS location tracking (configurable)
- Real-time position reporting via events
- Location data integration with SMS workflows

## Documentation

| Document                                     | Description                                        |
|----------------------------------------------|----------------------------------------------------|
| [Configuration Guide](docs/configuration.md) | Complete configuration reference with examples     |
| [Event Types](docs/events.md)                | Available events received via WebSocket or Webhook |
| [HTTP API Reference](docs/http.md)           | REST API endpoints for SMS operations              |
| [WebSocket Guide](docs/websocket.md)         | Real-time event streaming setup                    |

## Features

| Name          | Default | Description                                                                            |
|---------------|---------|----------------------------------------------------------------------------------------|
| `gpio`        | ✔️      | GPIO power pin support for automatic HAT power management                              |
| `http-server` | ✔️      | HTTP server to control the modem and access database                                   |
| `db-sqlite`   | ✔️      | SQLite database connection driver (currently only database supported)                  |
| `tls-rustls`  | ✔️      | Uses rustls and aws-lc-rs for TLS all connections                                      | 
| `tls-native`  |         | Uses openssl for http-server (if enabled) and native-tls for all other TLS connections |
| `sentry`      |         | Adds Sentry error reporting / logging integration                                      |

## Installation

```shell
git clone https://github.com/morgverd/sms-server

# Build, with all default.
cargo build -r

# Build with Sentry error forwarding.
cargo build -r --features sentry

# Build without HTTP server, and with GPIO, SQLite and Rust TLS.
cargo build -r --no-default-features -F gpio,db-sqlite,tls-rustls

# Build with native SSL and default features.
cargo build -r --no-default-features -F gpio,http-server,db-sqlite,tls-native 
```
```shell
# Show command line help.
./sms-server -h

# Start the SMS server with a config path, can be relative or absolute.
./sms-server -c config.toml

# Start the SMS server with debug logging.
RUST_LOG=debug ./sms-server -c config.toml
```

## Examples

### [💬 ChatGPT SMS Bot](examples/chatgpt-sms)

An example chatbot that integrates with OpenAI's ChatGPT API. Receives incoming messages via webhooks, generates replies using message history, and responds automatically.

### [🗺️ Real-time GNSS Viewer](examples/gnss-viewer)

A web-based GPS tracking dashboard that connects via WebSocket to display live position updates. Monitor location accuracy, track movement patterns, and analyze GPS performance in real-time. Accessible from any networked device with a modern web browser.

## Hardware Requirements

You'll need some form of GSM modem that allows for serial connection.
I use (and this project has only been tested with) a [Waveshare GSM Pi Hat](https://www.waveshare.com/gsm-gprs-gnss-hat.htm) on a Raspberry Pi.

> [!TIP]
> Many SIM cards require carrier-specific APN configuration and network registration before SMS functionality becomes available.

## Known Limitations

- **Delivery Confirmation Scope**: Only the final segment of multipart SMS messages receives delivery confirmation tracking, which may mask delivery failures in earlier message parts.
- **Sequential Processing**: Messages are processed sequentially, which ensures reliability but may impact throughput for high-volume scenarios.